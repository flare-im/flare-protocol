//! The plugin package, the unit a publisher ships: a zip named `.fpkg` around
//! the signed manifest.
//!
//! ```text
//! flare.redpacket-0.8.0.fpkg
//!   manifest.json    the exact bytes the publisher signed
//!   manifest.sig     {"keyId": "..", "signature": "<base64>"}
//!   assets/          icons and screenshots listed in `manifest.assets`
//!   README.md        optional, outside the signature
//!   CHANGELOG.md     optional, outside the signature
//! ```
//!
//! Signature and digest stay those of `manifest.json`, so hosts, tickets and
//! projections compare the same bytes as before. An asset is covered through
//! its digest in the manifest: changing one fails [`PluginPackage::open`].
//!
//! Entries are stored, never compressed. The same contents give the same file
//! whatever built it, and opening a package has no decompression to exhaust.
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, System, ZipArchive, ZipWriter};

use crate::manifest::{MAX_PLUGIN_ASSETS, is_asset_path};
use crate::runtime::digest_bytes;
use crate::signature::{PublisherTrustStore, VerifiedManifest};
use crate::{PluginManifest, ProtocolError, Result};

/// File extension of a package.
pub const PACKAGE_EXTENSION: &str = "fpkg";
pub const MANIFEST_ENTRY: &str = "manifest.json";
pub const SIGNATURE_ENTRY: &str = "manifest.sig";
pub const README_ENTRY: &str = "README.md";
pub const CHANGELOG_ENTRY: &str = "CHANGELOG.md";
/// A package file, and the sum of its entries.
pub const MAX_PACKAGE_BYTES: u64 = 32 * 1024 * 1024;
/// The assets and the four fixed entries.
const MAX_PACKAGE_ENTRIES: usize = MAX_PLUGIN_ASSETS + 4;
const MAX_SIGNATURE_BYTES: usize = 1024;

/// Contents of `manifest.sig`, the two values
/// [`PublisherTrustStore::verify`] takes besides the manifest bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSignature {
    pub key_id: String,
    /// Base64 Ed25519 signature of the exact `manifest.json` bytes.
    pub signature: String,
}

/// A package whose signature and declared assets have been verified.
#[derive(Debug, Clone)]
pub struct PluginPackage {
    manifest: VerifiedManifest,
    manifest_json: Vec<u8>,
    signature: PackageSignature,
    assets: BTreeMap<String, Vec<u8>>,
    readme: Option<Vec<u8>>,
    changelog: Option<Vec<u8>>,
}

impl PluginPackage {
    /// Reads and verifies a package: bounded size and entry count, safe unique
    /// paths, a trusted signature over the exact `manifest.json` bytes, every
    /// declared asset with its size and digest, and no other file than
    /// `README.md` and `CHANGELOG.md`.
    pub fn open(bytes: &[u8], trust: &PublisherTrustStore) -> Result<Self> {
        let mut entries = read_entries(bytes)?;
        let manifest_json = entries
            .remove(MANIFEST_ENTRY)
            .ok_or(ProtocolError::InvalidPackage("missing manifest.json"))?;
        let raw_signature = entries
            .remove(SIGNATURE_ENTRY)
            .ok_or(ProtocolError::InvalidPackage("missing manifest.sig"))?;
        if raw_signature.len() > MAX_SIGNATURE_BYTES {
            return Err(ProtocolError::InvalidPackage("manifest.sig is too large"));
        }
        let signature: PackageSignature = serde_json::from_slice(&raw_signature)
            .map_err(|_| ProtocolError::InvalidPackage("unreadable manifest.sig"))?;
        let manifest = trust.verify(&manifest_json, &signature.key_id, &signature.signature)?;
        let readme = entries.remove(README_ENTRY);
        let changelog = entries.remove(CHANGELOG_ENTRY);
        check_assets(manifest.manifest(), &entries)?;
        Ok(Self {
            manifest,
            manifest_json,
            signature,
            assets: entries,
            readme,
            changelog,
        })
    }

    pub fn manifest(&self) -> &VerifiedManifest {
        &self.manifest
    }
    /// The exact signed bytes.
    pub fn manifest_json(&self) -> &[u8] {
        &self.manifest_json
    }
    pub fn signature(&self) -> &PackageSignature {
        &self.signature
    }
    /// The verified assets by package path.
    pub fn assets(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.assets
    }
    /// Not covered by the signature.
    pub fn readme(&self) -> Option<&[u8]> {
        self.readme.as_deref()
    }
    /// Not covered by the signature.
    pub fn changelog(&self) -> Option<&[u8]> {
        self.changelog.as_deref()
    }
}

/// What [`write_package`] puts in a package.
#[derive(Debug, Clone, Copy)]
pub struct PackageContents<'a> {
    pub manifest_json: &'a [u8],
    pub signature: &'a PackageSignature,
    /// By package path; must be exactly the manifest's `assets` table.
    pub assets: &'a BTreeMap<String, Vec<u8>>,
    pub readme: Option<&'a [u8]>,
    pub changelog: Option<&'a [u8]>,
}

/// Builds a package that [`PluginPackage::open`] accepts once the publisher
/// key is trusted. The same contents always give the same bytes: sorted
/// entries, stored, with a fixed timestamp, mode and system.
pub fn write_package(contents: &PackageContents<'_>) -> Result<Vec<u8>> {
    let manifest = PluginManifest::from_json(contents.manifest_json)?;
    check_assets(&manifest, contents.assets)?;
    let signature =
        serde_json::to_vec(contents.signature).map_err(|_| ProtocolError::InvalidJson)?;
    let mut entries: BTreeMap<&str, &[u8]> = contents
        .assets
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect();
    entries.insert(MANIFEST_ENTRY, contents.manifest_json);
    entries.insert(SIGNATURE_ENTRY, signature.as_slice());
    entries.extend(contents.readme.map(|bytes| (README_ENTRY, bytes)));
    entries.extend(contents.changelog.map(|bytes| (CHANGELOG_ENTRY, bytes)));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::DEFAULT)
        .unix_permissions(0o644)
        .system(System::Unix);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer.start_file(name, options).map_err(write_failed)?;
        writer.write_all(bytes).map_err(write_failed)?;
    }
    let bytes = writer.finish().map_err(write_failed)?.into_inner();
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(ProtocolError::InvalidPackage("package exceeds 32 MiB"));
    }
    Ok(bytes)
}

fn write_failed<E>(_: E) -> ProtocolError {
    ProtocolError::InvalidPackage("package could not be written")
}

/// The entries by name, after every check that needs no manifest.
fn read_entries(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(ProtocolError::InvalidPackage("package exceeds 32 MiB"));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| ProtocolError::InvalidPackage("not a zip archive"))?;
    if archive.len() > MAX_PACKAGE_ENTRIES {
        return Err(ProtocolError::InvalidPackage("too many entries"));
    }
    // A repeated name, or a local header that disagrees with the central
    // directory, lets two readers see different packages.
    if archive.has_duplicate_names() {
        return Err(ProtocolError::InvalidPackage("duplicate entry"));
    }
    archive
        .validate_header_consistency()
        .map_err(|_| ProtocolError::InvalidPackage("inconsistent zip headers"))?;
    let mut entries = BTreeMap::new();
    let mut remaining = MAX_PACKAGE_BYTES;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| ProtocolError::InvalidPackage("unreadable entry"))?;
        let name = entry
            .name()
            .map_err(|_| ProtocolError::InvalidPackage("unreadable entry name"))?
            .into_owned();
        if !is_package_path(&name) {
            return Err(ProtocolError::InvalidPackage("undeclared entry path"));
        }
        if entry.is_dir()
            || entry.is_symlink()
            || entry.encrypted()
            || entry.compression() != CompressionMethod::Stored
        {
            return Err(ProtocolError::InvalidPackage(
                "entries are plain stored files",
            ));
        }
        if entry.size() > remaining {
            return Err(ProtocolError::InvalidPackage("entries exceed 32 MiB"));
        }
        let mut content = Vec::new();
        entry
            .take(remaining.saturating_add(1))
            .read_to_end(&mut content)
            .map_err(|_| ProtocolError::InvalidPackage("unreadable entry"))?;
        remaining = remaining
            .checked_sub(content.len() as u64)
            .ok_or(ProtocolError::InvalidPackage("entries exceed 32 MiB"))?;
        if entries.insert(name, content).is_some() {
            return Err(ProtocolError::InvalidPackage("duplicate entry"));
        }
    }
    Ok(entries)
}

fn is_package_path(name: &str) -> bool {
    [
        MANIFEST_ENTRY,
        SIGNATURE_ENTRY,
        README_ENTRY,
        CHANGELOG_ENTRY,
    ]
    .contains(&name)
        || is_asset_path(name)
}

/// `assets` holds exactly what the manifest declares. Declared paths are
/// unique, so equal counts and a match for each mean equal sets.
fn check_assets(manifest: &PluginManifest, assets: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    if assets.len() != manifest.assets.len() {
        return Err(ProtocolError::InvalidPackage(
            "assets differ from the manifest",
        ));
    }
    for declared in &manifest.assets {
        let bytes = assets
            .get(&declared.path)
            .ok_or(ProtocolError::InvalidPackage(
                "assets differ from the manifest",
            ))?;
        if bytes.len() as u64 != declared.size || digest_bytes(bytes) != declared.digest {
            return Err(ProtocolError::InvalidPackage(
                "asset does not match its declared digest",
            ));
        }
    }
    Ok(())
}
