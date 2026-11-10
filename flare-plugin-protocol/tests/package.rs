//! Opening and writing `.fpkg` packages.
use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use flare_plugin_protocol::ProtocolError;
use flare_plugin_protocol::package::{
    CHANGELOG_ENTRY, MANIFEST_ENTRY, MAX_PACKAGE_BYTES, PackageContents, PackageSignature,
    PluginPackage, README_ENTRY, SIGNATURE_ENTRY, write_package,
};
use flare_plugin_protocol::runtime::digest_bytes;
use flare_plugin_protocol::signature::PublisherTrustStore;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const ICON: &[u8] = b"icon bytes";
const SHOT: &[u8] = b"screenshot bytes";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn trust() -> PublisherTrustStore {
    let mut trust = PublisherTrustStore::default();
    trust
        .insert(
            "key-1".into(),
            "example".into(),
            key().verifying_key().to_bytes(),
        )
        .unwrap();
    trust
}

fn asset(path: &str, bytes: &[u8]) -> Value {
    json!({"path": path, "digest": digest_bytes(bytes), "size": bytes.len()})
}

fn manifest(assets: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "manifestVersion": 1, "pluginId": "example.pkg", "version": "1.0.0", "apiVersion": "1.0",
        "displayName": "Package", "publisher": {"publisherId": "example", "displayName": "Example"},
        "runtime": {"kind": "client"},
        "capabilities": [{"id": "example.pkg", "operations": [{
            "id": "run", "inputSchema": {"type": "object"}, "outputSchema": {"type": "object"},
            "permissionIds": ["example.pkg.use"], "idempotent": true, "timeoutMs": 1000
        }]}],
        "permissions": [{"id": "example.pkg.use", "description": "use", "dataScopes": [], "riskLevel": "low"}],
        "features": [{"id": "run", "label": {"en-US": "Run"}, "gates": {"operations": ["example.pkg.run"]}}],
        "assets": assets
    }))
    .unwrap()
}

fn signed_manifest() -> Vec<u8> {
    manifest(json!([
        asset("assets/icon.png", ICON),
        asset("assets/shots/one.png", SHOT)
    ]))
}

fn signature(manifest: &[u8]) -> PackageSignature {
    PackageSignature {
        key_id: "key-1".into(),
        signature: STANDARD.encode(key().sign(manifest).to_bytes()),
    }
}

fn signature_entry(manifest: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&signature(manifest)).unwrap()
}

fn assets() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("assets/icon.png".to_owned(), ICON.to_vec()),
        ("assets/shots/one.png".to_owned(), SHOT.to_vec()),
    ])
}

fn package() -> Vec<u8> {
    let manifest = signed_manifest();
    write_package(&PackageContents {
        manifest_json: &manifest,
        signature: &signature(&manifest),
        assets: &assets(),
        readme: Some(b"# Package"),
        changelog: None,
    })
    .unwrap()
}

/// A zip the writer would refuse to build, entry by entry.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// The entries of a good package, with `extra` appended.
fn package_with(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let manifest = signed_manifest();
    let signature = signature_entry(&manifest);
    let mut entries: Vec<(&str, &[u8])> = vec![
        (MANIFEST_ENTRY, manifest.as_slice()),
        (SIGNATURE_ENTRY, signature.as_slice()),
        ("assets/icon.png", ICON),
        ("assets/shots/one.png", SHOT),
    ];
    entries.extend_from_slice(extra);
    zip_of(&entries)
}

fn refused(bytes: &[u8]) -> ProtocolError {
    PluginPackage::open(bytes, &trust()).unwrap_err()
}

fn replace(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let mut found = 0;
    for start in 0..=bytes.len() - from.len() {
        if &bytes[start..start + from.len()] == from {
            bytes[start..start + to.len()].copy_from_slice(to);
            found += 1;
        }
    }
    assert!(found > 0);
}

#[test]
fn a_written_package_opens_with_its_exact_manifest_and_assets() {
    let manifest = signed_manifest();
    let opened = PluginPackage::open(&package(), &trust()).unwrap();
    assert_eq!(opened.manifest_json(), manifest);
    assert_eq!(opened.manifest().digest(), digest_bytes(&manifest));
    assert_eq!(opened.manifest().manifest().plugin_id, "example.pkg");
    assert_eq!(opened.signature(), &signature(&manifest));
    assert_eq!(opened.assets(), &assets());
    assert_eq!(opened.readme(), Some(&b"# Package"[..]));
    assert_eq!(opened.changelog(), None);
}

#[test]
fn the_same_contents_write_the_same_sorted_bytes() {
    let bytes = package();
    assert_eq!(bytes, package());
    let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let names: Vec<_> = archive
        .file_names()
        .map(|n| n.unwrap().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            README_ENTRY,
            "assets/icon.png",
            "assets/shots/one.png",
            MANIFEST_ENTRY,
            SIGNATURE_ENTRY
        ]
    );
    let entry = archive.by_index(0).unwrap();
    assert_eq!(entry.compression(), CompressionMethod::Stored);
    assert_eq!(entry.last_modified(), Some(zip::DateTime::DEFAULT));
}

#[test]
fn the_signature_covers_the_exact_manifest_bytes() {
    let mut manifest = signed_manifest();
    let signature = signature_entry(&manifest);
    manifest.push(b' ');
    assert_eq!(
        refused(&zip_of(&[
            (MANIFEST_ENTRY, &manifest),
            (SIGNATURE_ENTRY, &signature),
            ("assets/icon.png", ICON),
            ("assets/shots/one.png", SHOT),
        ])),
        ProtocolError::InvalidSignature
    );
    assert_eq!(
        PluginPackage::open(&package(), &PublisherTrustStore::default()).unwrap_err(),
        ProtocolError::InvalidSignature
    );
}

#[test]
fn a_package_needs_its_manifest_and_a_readable_signature() {
    let manifest = signed_manifest();
    let signature = signature_entry(&manifest);
    assert!(matches!(
        refused(&zip_of(&[(SIGNATURE_ENTRY, &signature)])),
        ProtocolError::InvalidPackage("missing manifest.json")
    ));
    assert!(matches!(
        refused(&zip_of(&[(MANIFEST_ENTRY, &manifest)])),
        ProtocolError::InvalidPackage("missing manifest.sig")
    ));
    assert!(matches!(
        refused(&zip_of(&[
            (MANIFEST_ENTRY, &manifest),
            (SIGNATURE_ENTRY, b"not json")
        ])),
        ProtocolError::InvalidPackage("unreadable manifest.sig")
    ));
    assert!(matches!(
        refused(b"not a zip"),
        ProtocolError::InvalidPackage("not a zip archive")
    ));
}

#[test]
fn assets_match_the_signed_table_exactly() {
    let manifest = signed_manifest();
    let signature = signature_entry(&manifest);
    let good = |shot: &'static [u8]| {
        zip_of(&[
            (MANIFEST_ENTRY, &manifest),
            (SIGNATURE_ENTRY, &signature),
            ("assets/icon.png", ICON),
            ("assets/shots/one.png", shot),
        ])
    };
    // Same size, other bytes; then another size.
    assert!(matches!(
        refused(&good(b"Screenshot bytes")),
        ProtocolError::InvalidPackage("asset does not match its declared digest")
    ));
    assert!(matches!(
        refused(&good(b"short")),
        ProtocolError::InvalidPackage("asset does not match its declared digest")
    ));
    assert!(matches!(
        refused(&zip_of(&[
            (MANIFEST_ENTRY, &manifest),
            (SIGNATURE_ENTRY, &signature),
            ("assets/icon.png", ICON),
        ])),
        ProtocolError::InvalidPackage("assets differ from the manifest")
    ));
    assert!(matches!(
        refused(&package_with(&[("assets/extra.png", b"x")])),
        ProtocolError::InvalidPackage("assets differ from the manifest")
    ));
    // README.md and CHANGELOG.md are the only other files.
    let opened = PluginPackage::open(
        &package_with(&[(README_ENTRY, b"r"), (CHANGELOG_ENTRY, b"c")]),
        &trust(),
    )
    .unwrap();
    assert_eq!(opened.changelog(), Some(&b"c"[..]));
}

#[test]
fn entries_stay_on_known_paths_and_are_plain_files() {
    for path in [
        "unknown.txt",
        "../evil.png",
        "assets/../evil.png",
        "/assets/evil.png",
        "assets\\evil.png",
        "assets/",
        "Manifest.json",
    ] {
        assert!(
            matches!(
                refused(&package_with(&[(path, b"x")])),
                ProtocolError::InvalidPackage("undeclared entry path")
            ),
            "{path}"
        );
    }
    let options = SimpleFileOptions::default();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.add_directory("assets/", options).unwrap();
    assert!(matches!(
        refused(&writer.finish().unwrap().into_inner()),
        ProtocolError::InvalidPackage("undeclared entry path")
    ));
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("assets/link.png", "/etc/passwd", options)
        .unwrap();
    assert!(matches!(
        refused(&writer.finish().unwrap().into_inner()),
        ProtocolError::InvalidPackage("entries are plain stored files")
    ));
}

#[test]
fn duplicate_names_are_refused() {
    let mut bytes = package_with(&[("assets/a.png", b"1"), ("assets/b.png", b"2")]);
    // One name twice, in the local and the central headers alike.
    replace(&mut bytes, b"assets/b.png", b"assets/a.png");
    assert!(matches!(
        refused(&bytes),
        ProtocolError::InvalidPackage("duplicate entry")
    ));
}

#[test]
fn a_compressed_entry_is_refused() {
    let mut bytes = package();
    // Compression method of the first entry: local header offset 8, central 10.
    bytes[8] = 8;
    let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    bytes[central + 10] = 8;
    assert!(matches!(refused(&bytes), ProtocolError::InvalidPackage(_)));
}

#[test]
fn a_package_is_bounded() {
    let oversized = vec![0; MAX_PACKAGE_BYTES as usize + 1];
    assert!(matches!(
        refused(&oversized),
        ProtocolError::InvalidPackage("package exceeds 32 MiB")
    ));
    let names: Vec<String> = (0..80).map(|i| format!("assets/{i}.png")).collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    assert!(matches!(
        refused(&zip_of(&entries)),
        ProtocolError::InvalidPackage("too many entries")
    ));
}

#[test]
fn writing_checks_the_asset_table() {
    let manifest = signed_manifest();
    let signature = signature(&manifest);
    let write = |assets: &BTreeMap<String, Vec<u8>>| {
        write_package(&PackageContents {
            manifest_json: &manifest,
            signature: &signature,
            assets,
            readme: None,
            changelog: None,
        })
    };
    assert!(write(&assets()).is_ok());
    let mut missing = assets();
    missing.remove("assets/icon.png");
    assert!(write(&missing).is_err());
    let mut changed = assets();
    changed.insert("assets/icon.png".into(), b"other bytes".to_vec());
    assert!(write(&changed).is_err());
}
