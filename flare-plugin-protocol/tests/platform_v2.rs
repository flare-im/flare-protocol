//! Plugin platform v2: system tickets for platform callbacks, operation
//! modes, the viewer layer (`viewerState`, `basedOnVersion` and the
//! versioned merge), grouped entries, menu actions, account entries, the
//! entity lifecycle, the admin section and staff permission codes, envelope
//! `displayName` and unavailable reasons.
use flare_plugin_protocol::{
    PluginManifest,
    invocation::*,
    manifest::{
        ClientPlatform, OperationMode, UNBOUND_REASON_POST_FAILED, UNBOUND_REASON_RECALLED,
        UiContribution, UiContributionKind,
    },
    runtime::configuration_digest,
    state::SharedState,
    ui::*,
};
use serde_json::{Value, json};

// ---- system tickets ----

fn system_claims() -> PluginSystemCallClaims {
    PluginSystemCallClaims {
        tenant_id: "201".into(),
        plugin_id: "flare.poll".into(),
        plugin_version: "0.2.0".into(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        deployment_id: "poll-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 3,
        configuration_digest: configuration_digest(&json!({})).unwrap(),
        operation: "flare.poll.lifecycle.unbound".into(),
        entity: "poll-7".into(),
        reason: UNBOUND_REASON_RECALLED.into(),
        request_id: "req-1".into(),
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    }
}

#[test]
fn system_tickets_verify_only_as_system_tickets() {
    let signer = PluginCallSigner::new("call-1".into(), [7; 32]).unwrap();
    let mut verifier = PluginCallVerifier::default();
    verifier
        .insert("call-1".into(), signer.public_key())
        .unwrap();
    let claims = system_claims();
    let token = signer.sign_system(&claims, 1000).unwrap();
    assert_eq!(verifier.verify_system(&token, 1000).unwrap(), claims);
    assert!(verifier.verify(&token, 1000).is_err(), "not a user ticket");
    assert!(
        verifier.verify_staff(&token, 1000).is_err(),
        "not a staff ticket"
    );
    assert!(verifier.verify_system(&token, 2000).is_err(), "expired");

    let user = PluginCallClaims {
        tenant_id: "201".into(),
        plugin_id: "flare.poll".into(),
        plugin_version: "0.2.0".into(),
        manifest_digest: claims.manifest_digest.clone(),
        deployment_id: "poll-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 3,
        configuration_digest: claims.configuration_digest.clone(),
        actor_id: "u1".into(),
        operation: "flare.poll.lifecycle.unbound".into(),
        request_id: "req-2".into(),
        conversation_id: None,
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    };
    let user_token = signer.sign(&user, 1000).unwrap();
    assert!(
        verifier.verify_system(&user_token, 1000).is_err(),
        "a user ticket is not a system ticket"
    );
    let staff = PluginStaffCallClaims {
        scope: Default::default(),
        tenant_id: "201".into(),
        plugin_id: "flare.poll".into(),
        plugin_version: "0.2.0".into(),
        manifest_digest: claims.manifest_digest.clone(),
        deployment_id: "poll-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 3,
        configuration_digest: claims.configuration_digest.clone(),
        staff_id: "staff-1".into(),
        org_id: None,
        permission: "plugin:flare.poll:manage".into(),
        operation: "flare.poll.admin.close".into(),
        request_id: "req-3".into(),
        reason: None,
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    };
    let staff_token = signer.sign_staff(&staff, 1000).unwrap();
    assert!(
        verifier.verify_system(&staff_token, 1000).is_err(),
        "a staff ticket is not a system ticket"
    );

    assert!(
        signer
            .sign_system(
                &PluginSystemCallClaims {
                    reason: UNBOUND_REASON_POST_FAILED.into(),
                    ..system_claims()
                },
                1000
            )
            .is_ok()
    );
    for bad in [
        PluginSystemCallClaims {
            reason: "deleted".into(),
            ..system_claims()
        },
        PluginSystemCallClaims {
            entity: "poll 7".into(),
            ..system_claims()
        },
        PluginSystemCallClaims {
            entity: String::new(),
            ..system_claims()
        },
        PluginSystemCallClaims {
            tenant_id: "*".into(),
            ..system_claims()
        },
        PluginSystemCallClaims {
            grant_revision: 0,
            ..system_claims()
        },
        PluginSystemCallClaims {
            operation: String::new(),
            ..system_claims()
        },
        PluginSystemCallClaims {
            expires_at_ms: 1001 + MAX_CALL_TTL_MS,
            ..system_claims()
        },
    ] {
        assert!(signer.sign_system(&bad, 1000).is_err(), "{bad:?}");
    }
    // Unknown fields are refused, like every other ticket.
    let mut raw = serde_json::to_value(system_claims()).unwrap();
    raw["actorId"] = json!("u1");
    assert!(serde_json::from_value::<PluginSystemCallClaims>(raw).is_err());
}

#[test]
fn a_ticket_names_its_key_so_a_host_can_tell_a_rotation_from_a_bad_ticket() {
    let signer = PluginCallSigner::new("call-2".into(), [8; 32]).unwrap();
    let token = signer.sign_system(&system_claims(), 1000).unwrap();
    assert_eq!(ticket_key_id(&token).as_deref(), Some("call-2"));
    let mut verifier = PluginCallVerifier::default();
    verifier
        .insert(
            "call-1".into(),
            PluginCallSigner::new("call-1".into(), [7; 32])
                .unwrap()
                .public_key(),
        )
        .unwrap();
    // Signed under a key this verifier has not been given: it does not verify, and says why.
    assert!(verifier.verify_system(&token, 1000).is_err());
    assert!(!verifier.has_key("call-2"));
    assert!(verifier.has_key("call-1"));
    // Not an envelope, or an envelope naming an invalid key id: no key id.
    for bad in ["", "not a ticket", "e30"] {
        assert_eq!(ticket_key_id(bad), None, "{bad}");
    }
    let forged = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        json!({"keyId": "bad id!", "payload": "", "signature": ""}).to_string(),
    );
    assert_eq!(ticket_key_id(&forged), None);
}

// ---- manifest fixture ----

fn manifest_value() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap()
}
fn parse(manifest: &Value) -> Result<PluginManifest, flare_plugin_protocol::ProtocolError> {
    PluginManifest::from_json(&serde_json::to_vec(manifest).unwrap())
}
fn card_index(manifest: &Value) -> usize {
    manifest["clientExtensions"]["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["id"] == "example.poll.card")
        .unwrap()
}
/// The fixture with a `view` query operation and a wallet page and query.
fn platform_manifest() -> Value {
    let mut manifest = manifest_value();
    let operations = manifest["capabilities"][0]["operations"]
        .as_array_mut()
        .unwrap();
    for (id, mode) in [
        ("view", Some("query")),
        ("wallet", Some("query")),
        ("unbound", None),
    ] {
        let mut operation = json!({
            "id": id, "idempotent": true, "inputSchema": {"type": "object"},
            "outputSchema": {"type": "object"}, "permissionIds": ["example.poll.read"],
            "timeoutMs": 5000
        });
        if let Some(mode) = mode {
            operation["mode"] = json!(mode);
        }
        operations.push(operation);
    }
    manifest["clientExtensions"]["contributions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind": "page", "id": "example.poll.wallet", "schemaVersion": 1,
            "requiredPermissions": [], "schema": {"title": "钱包"}
        }));
    manifest
}

#[test]
fn operation_mode_defaults_to_command_and_round_trips() {
    let manifest = parse(&platform_manifest()).unwrap();
    assert_eq!(
        manifest.operation("example.poll.vote").unwrap().mode,
        OperationMode::Command
    );
    assert_eq!(
        manifest.operation("example.poll.view").unwrap().mode,
        OperationMode::Query
    );
    let raw = serde_json::to_value(&manifest).unwrap();
    let vote = raw["capabilities"][0]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "vote")
        .unwrap();
    assert!(vote.get("mode").is_none(), "command is not serialized");
    let mut bad = platform_manifest();
    bad["capabilities"][0]["operations"][0]["mode"] = json!("read");
    assert!(parse(&bad).is_err());
}

// ---- viewerState ----

fn viewer_state() -> Value {
    json!({"operation": "example.poll.view", "input": {"pollId": "/pollId"},
           "refreshOn": ["present", "sharedVersion"], "minIntervalMs": 2000})
}

#[test]
fn viewer_state_needs_a_declared_query_on_a_card() {
    let mut manifest = platform_manifest();
    let card = card_index(&manifest);
    manifest["clientExtensions"]["contributions"][card]["viewerState"] = viewer_state();
    let parsed = parse(&manifest).unwrap();
    let viewer = parsed
        .contribution("example.poll.card")
        .unwrap()
        .viewer_state
        .clone()
        .unwrap();
    assert!(viewer.refreshes_on(UiViewerRefresh::Present));
    assert!(viewer.refreshes_on(UiViewerRefresh::SharedVersion));
    assert_eq!(
        viewer
            .bind_input(&json!({"pollId": "p-1", "x": 1}))
            .unwrap(),
        json!({"pollId": "p-1"})
    );

    let reject = |edit: &dyn Fn(&mut Value)| {
        let mut manifest = platform_manifest();
        let card = card_index(&manifest);
        manifest["clientExtensions"]["contributions"][card]["viewerState"] = viewer_state();
        edit(&mut manifest["clientExtensions"]["contributions"][card]["viewerState"]);
        parse(&manifest).is_err()
    };
    assert!(
        reject(&|v| v["operation"] = json!("example.poll.vote")),
        "a command"
    );
    assert!(reject(&|v| v["operation"] = json!("example.poll.nope")));
    assert!(reject(&|v| v["minIntervalMs"] = json!(999)));
    assert!(reject(&|v| v["minIntervalMs"] = json!(3_600_001)));
    assert!(reject(&|v| v["refreshOn"] = json!([])));
    assert!(reject(&|v| v["refreshOn"] = json!(["present", "present"])));
    assert!(reject(&|v| v["refreshOn"] = json!(["always"])));
    assert!(reject(&|v| v["input"] = json!({"pollId": "pollId"})));
    assert!(reject(&|v| v["input"] = json!({"bad-name": "/pollId"})));
    assert!(reject(&|v| v["extra"] = json!(1)));
    // message-content only.
    let mut manifest = platform_manifest();
    manifest["clientExtensions"]["contributions"][1]["viewerState"] = viewer_state();
    assert!(parse(&manifest).is_err());
}

fn local(version: u64, based_on: Option<u64>, values: Value) -> PluginMessageState {
    PluginMessageState {
        version,
        based_on_version: based_on,
        values: values.as_object().unwrap().clone(),
    }
}
fn shared(version: u64, values: Value) -> SharedState {
    SharedState {
        entity: "7".into(),
        version,
        values: values.as_object().unwrap().clone(),
    }
}
fn card_contribution() -> UiContribution {
    parse(&platform_manifest())
        .unwrap()
        .contribution("example.poll.card")
        .unwrap()
        .clone()
}

#[test]
fn a_fresh_local_layer_replaces_shared_keys() {
    let card = card_contribution();
    let shared = shared(5, json!({"counts": [1, 2], "status": "open"}));
    let local = local(3, Some(5), json!({"counts": [1, 3], "canClose": true}));
    let merged = UiStateLayers {
        shared: Some(&shared),
        local: Some(&local),
    }
    .merge(&card, &json!({"pollId": 7, "question": "q"}));
    assert_eq!(
        merged,
        json!({"pollId": 7, "question": "q", "counts": [1, 3], "status": "open", "canClose": true})
    );
    // Newer than the shared layer is fresh too.
    let ahead = local_based(6);
    assert!(ahead.is_fresh_against(5));
}
fn local_based(based_on: u64) -> PluginMessageState {
    local(1, Some(based_on), json!({}))
}

#[test]
fn a_stale_local_layer_keeps_the_shared_value_of_shared_keys() {
    let card = card_contribution();
    let shared = shared(
        6,
        json!({"counts": [4, 5], "status": "closed", "gone": null}),
    );
    let local = local(
        3,
        Some(5),
        json!({"counts": [1, 3], "status": "open", "gone": "local", "canClose": true}),
    );
    let merged = UiStateLayers {
        shared: Some(&shared),
        local: Some(&local),
    }
    .merge(&card, &json!({"pollId": 7, "gone": "payload"}));
    assert_eq!(
        merged,
        json!({"pollId": 7, "counts": [4, 5], "status": "closed", "canClose": true}),
        "shared keys (even a removal) win; the local-only key still applies"
    );
    // Without basedOnVersion the layer counts as based on version 0.
    let unversioned = PluginMessageState {
        based_on_version: None,
        ..local.clone()
    };
    assert!(!unversioned.is_fresh_against(1));
    assert_eq!(
        UiStateLayers {
            shared: Some(&shared),
            local: Some(&unversioned),
        }
        .merge(&card, &json!({"pollId": 7}))["counts"],
        json!([4, 5])
    );
}

#[test]
fn without_a_shared_layer_the_local_layer_applies_as_is() {
    let card = card_contribution();
    let local = local(3, None, json!({"counts": [1, 3], "mySelection": [1]}));
    let payload = json!({"pollId": 7, "counts": [0, 0]});
    let expected = json!({"pollId": 7, "counts": [1, 3], "mySelection": [1]});
    assert_eq!(
        UiStateLayers {
            shared: None,
            local: Some(&local),
        }
        .merge(&card, &payload),
        expected
    );
    // A shared layer of another entity does not apply, so it is not "newer".
    let other = SharedState {
        entity: "8".into(),
        ..shared(9, json!({"counts": [9, 9]}))
    };
    assert_eq!(
        UiStateLayers {
            shared: Some(&other),
            local: Some(&local),
        }
        .merge(&card, &payload),
        expected
    );
    // merge_shared never reads the local layer.
    assert_eq!(
        UiStateLayers {
            shared: None,
            local: Some(&local),
        }
        .merge_shared(&card, &payload),
        payload
    );
}

#[test]
fn viewer_reads_are_needed_when_missing_or_stale_and_overlay_the_previous_layer() {
    assert!(UiViewerState::needs_read(None, 0));
    assert!(UiViewerState::needs_read(None, 4));
    let fresh = local(2, Some(4), json!({}));
    assert!(!UiViewerState::needs_read(Some(&fresh), 4));
    assert!(UiViewerState::needs_read(Some(&fresh), 5));
    let unversioned = local(2, None, json!({}));
    assert!(!UiViewerState::needs_read(Some(&unversioned), 0));
    assert!(UiViewerState::needs_read(Some(&unversioned), 1));

    // The result's basedOnVersion wins over the version seen at call time.
    let previous = local(7, Some(3), json!({"mySelection": [1], "canVote": true}));
    let layer = UiViewerState::local_layer(
        &json!({"hasVoted": true, "canVote": false, "counts": [2, 1], "basedOnVersion": 6}),
        Some(&previous),
        8,
    )
    .unwrap();
    assert_eq!(layer.version, 7, "keeps the previous layer's version");
    assert_eq!(layer.based_on_version, Some(6));
    assert_eq!(
        Value::Object(layer.values),
        json!({"mySelection": [1], "canVote": false, "hasVoted": true, "counts": [2, 1]}),
        "a device-only key the read does not know is kept"
    );
    // Without basedOnVersion the seen shared version is used; null removes a key.
    let layer = UiViewerState::local_layer(
        &json!({"mySelection": null, "canClose": true}),
        Some(&previous),
        8,
    )
    .unwrap();
    assert_eq!(layer.based_on_version, Some(8));
    assert_eq!(
        Value::Object(layer.values),
        json!({"canVote": true, "canClose": true})
    );
    let first = UiViewerState::local_layer(&json!({"canVote": true}), None, 0).unwrap();
    assert_eq!((first.version, first.based_on_version), (0, Some(0)));
    // A result's refreshOn narrows the declared triggers and is not stored.
    let large_group = json!({"canVote": true, "refreshOn": ["present", "later"]});
    let narrowed = UiViewerState::result_refresh_on(&large_group).unwrap();
    assert_eq!(narrowed, [UiViewerRefresh::Present]);
    assert!(
        UiViewerState::local_layer(&large_group, None, 0)
            .unwrap()
            .values
            .get("refreshOn")
            .is_none()
    );
    assert_eq!(
        UiViewerState::result_refresh_on(&json!({"canVote": true})),
        None
    );
    let declared: UiViewerState = serde_json::from_value(viewer_state()).unwrap();
    assert!(declared.refreshes_on_with(UiViewerRefresh::SharedVersion, None));
    assert!(!declared.refreshes_on_with(UiViewerRefresh::SharedVersion, Some(&narrowed)));
    assert!(declared.refreshes_on_with(UiViewerRefresh::Present, Some(&narrowed)));
    let present_only: UiViewerState = serde_json::from_value(json!({
        "operation": "x.op.view", "refreshOn": ["present"], "minIntervalMs": 1000
    }))
    .unwrap();
    assert!(
        !present_only.refreshes_on_with(
            UiViewerRefresh::SharedVersion,
            Some(&[UiViewerRefresh::SharedVersion])
        ),
        "a result never adds a trigger"
    );
    // Not an object, a malformed basedOnVersion, or out of bounds: nothing.
    assert!(UiViewerState::local_layer(&json!([1]), None, 0).is_none());
    assert!(UiViewerState::local_layer(&json!({"basedOnVersion": -1}), None, 0).is_none());
    assert!(UiViewerState::local_layer(&json!({"basedOnVersion": "3"}), None, 0).is_none());
    assert!(UiViewerState::local_layer(&json!({"nested": {"a": 1}}), None, 0).is_none());
    // basedOnVersion round-trips on the wire and stays bounded.
    let state: PluginMessageState =
        serde_json::from_value(json!({"version": 1, "basedOnVersion": 4, "values": {}})).unwrap();
    assert_eq!(state.based_on_version, Some(4));
    assert!(
        PluginMessageState {
            based_on_version: Some(u64::MAX),
            ..state
        }
        .validate()
        .is_err()
    );
}

#[test]
fn a_plugin_cannot_pin_the_local_layer_fresh_beyond_the_shared_version_seen() {
    // A result claiming a shared version the caller never saw is capped at the seen one, so the
    // next shared layer makes it stale again and its shared keys yield.
    for claimed in [9, 1_000_000, i64::MAX as u64] {
        let layer = UiViewerState::local_layer(
            &json!({"counts": [9, 9], "basedOnVersion": claimed}),
            None,
            4,
        )
        .unwrap();
        assert_eq!(layer.based_on_version, Some(4), "{claimed}");
        assert!(layer.is_fresh_against(4));
        assert!(!layer.is_fresh_against(5), "{claimed}");
        assert!(UiViewerState::needs_read(Some(&layer), 5));
    }
    // Without a shared layer seen, no claim makes the layer fresh against a later one.
    let layer =
        UiViewerState::local_layer(&json!({"counts": [1], "basedOnVersion": 7}), None, 0).unwrap();
    assert_eq!(layer.based_on_version, Some(0));
    // The same cap for a layer that comes whole (an action result's messageState).
    assert_eq!(
        local(2, Some(1_000), json!({}))
            .capped_at(3)
            .based_on_version,
        Some(3)
    );
    assert_eq!(
        local(2, Some(2), json!({})).capped_at(3).based_on_version,
        Some(2)
    );
    assert_eq!(
        local(2, None, json!({})).capped_at(3).based_on_version,
        None
    );
}

// ---- grouped entries ----

fn grouped_detail(entries: Value) -> ResolvedPluginView {
    let contribution: UiContribution = serde_json::from_value(json!({
        "kind": "message-content", "id": "x.card", "schemaVersion": 1, "requiredPermissions": [],
        "schema": {"title": "t", "blocks": [{
            "kind": "entries", "itemsPath": "/voters", "namePath": "/name", "userPath": "/userId",
            "numbered": false, "limit": 50,
            "groupPath": "/option", "groupLabelPath": "/optionLabel",
            "groupCountPath": "/optionCount", "groupTotalPath": "/totalVotes"
        }]}
    }))
    .unwrap();
    contribution.schema.validate().unwrap();
    contribution.resolve(&json!({"voters": entries}), "", "u1")
}
fn voter(option: u32, label: &str, count: u64, user: &str) -> Value {
    json!({"option": option, "optionLabel": label, "optionCount": count, "totalVotes": 8,
           "userId": user, "name": user.to_uppercase()})
}
fn entries(view: &ResolvedPluginView) -> &ResolvedUiEntries {
    match &view.blocks[0] {
        ResolvedUiBlock::Entries(entries) => entries,
        other => panic!("{other:?}"),
    }
}

#[test]
fn entries_group_by_consecutive_keys_with_headers_on_the_first_entry() {
    let view = grouped_detail(json!([
        voter(0, "火锅", 3, "u1"),
        voter(0, "火锅", 3, "u2"),
        voter(1, "烧烤", 5, "u3"),
        {"userId": "u4", "name": "no group"},
        voter(0, "火锅", 3, "u5"),
    ]));
    let block = entries(&view);
    assert!(block.grouped);
    assert!(!block.numbered);
    let rows: Vec<_> = block
        .items
        .iter()
        .map(|item| {
            (
                item.index,
                item.user_id.clone().unwrap(),
                item.group
                    .as_ref()
                    .map(|g| (g.key.clone(), g.label.clone(), g.count, g.total, g.percent)),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                1,
                "u1".to_owned(),
                Some(("0".into(), "火锅".into(), Some(3), Some(8), Some(38)))
            ),
            (2, "u2".to_owned(), None),
            (
                1,
                "u3".to_owned(),
                Some(("1".into(), "烧烤".into(), Some(5), Some(8), Some(63)))
            ),
            (
                1,
                "u5".to_owned(),
                Some(("0".into(), "火锅".into(), Some(3), Some(8), Some(38)))
            ),
        ],
        "a key change starts a group (again); an entry without a key is not shown"
    );
    let json = serde_json::to_value(block).unwrap();
    assert_eq!(json["grouped"], json!(true));
    assert_eq!(
        json["items"][0]["group"],
        json!({"key": "0", "label": "火锅", "count": 3, "total": 8, "percent": 38})
    );
    assert!(json["items"][1].get("group").is_none());
}

#[test]
fn group_headers_fall_back_to_the_key_and_omit_unknown_counts() {
    let contribution: UiContribution = serde_json::from_value(json!({
        "kind": "message-content", "id": "x.card", "schemaVersion": 1, "requiredPermissions": [],
        "schema": {"title": "t", "blocks": [
            {"kind": "entries", "itemsPath": "/rows", "textPath": "/text", "groupPath": "/day"}
        ]}
    }))
    .unwrap();
    contribution.schema.validate().unwrap();
    let view = contribution.resolve(
        &json!({"rows": [{"day": "周一", "text": "a"}, {"day": "周一", "text": "b"}]}),
        "",
        "u1",
    );
    let header = entries(&view).items[0].group.clone().unwrap();
    assert_eq!(header.label, "周一");
    assert_eq!(
        (header.count, header.total, header.percent),
        (None, None, None)
    );
    assert!(entries(&view).numbered, "numbered stays the default");
    // Group paths without groupPath are refused.
    let invalid: UiDocument = serde_json::from_value(json!({"title": "t", "blocks": [
        {"kind": "entries", "itemsPath": "/rows", "groupLabelPath": "/label"}
    ]}))
    .unwrap();
    assert!(invalid.validate().is_err());
    let invalid: UiDocument = serde_json::from_value(json!({"title": "t", "blocks": [
        {"kind": "entries", "itemsPath": "/rows", "groupPath": "day"}
    ]}))
    .unwrap();
    assert!(invalid.validate().is_err());
}

#[test]
fn a_page_continuing_the_last_group_drops_its_header_and_numbers_on() {
    let mut view = grouped_detail(json!([
        voter(0, "火锅", 3, "u1"),
        voter(0, "火锅", 3, "u2"),
    ]));
    let page = grouped_detail(json!([
        voter(0, "火锅", 4, "u3"),
        voter(1, "烧烤", 4, "u4"),
        voter(1, "烧烤", 4, "u5"),
    ]));
    view.append_page(page);
    let block = entries(&view);
    let rows: Vec<_> = block
        .items
        .iter()
        .map(|item| (item.index, item.group.as_ref().map(|g| g.key.clone())))
        .collect();
    assert_eq!(
        rows,
        [
            (1, Some("0".into())),
            (2, None),
            (3, None),
            (1, Some("1".into())),
            (2, None)
        ]
    );
    assert_eq!(
        block.items[0].group.as_ref().unwrap().count,
        Some(4),
        "the continued header takes the newer count"
    );
    // A page that starts a new group keeps its header and numbering.
    let mut view = grouped_detail(json!([voter(0, "火锅", 1, "u1")]));
    view.append_page(grouped_detail(json!([voter(1, "烧烤", 1, "u2")])));
    let rows: Vec<_> = entries(&view)
        .items
        .iter()
        .map(|item| (item.index, item.group.is_some()))
        .collect();
    assert_eq!(rows, [(1, true), (1, true)]);
}

// ---- menu actions ----

#[test]
fn menu_actions_resolve_apart_from_the_button_row() {
    let contribution: UiContribution = serde_json::from_value(json!({
        "kind": "message-content", "id": "x.card", "schemaVersion": 1, "requiredPermissions": [],
        "schema": {"title": "t", "actions": [
            {"id": "details", "label": "详情", "target": {"kind": "operation", "operation": "x.op.details"}},
            {"id": "close", "label": "结束投票", "placement": "menu", "style": "danger",
             "target": {"kind": "operation", "operation": "x.op.close"},
             "hiddenWhen": [{"path": "/canClose", "anyOf": ["false"]}]},
            {"id": "cancel", "label": "取消投票", "placement": "menu",
             "target": {"kind": "operation", "operation": "x.op.cancel"}}
        ], "translations": {"en": {"title": "T", "actions": {"close": "Close poll"}}}}
    }))
    .unwrap();
    contribution.schema.validate().unwrap();
    let view = contribution.resolve(&json!({"canClose": true}), "en", "u1");
    assert_eq!(
        view.actions
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["details"]
    );
    assert_eq!(
        view.menu_actions
            .iter()
            .map(|a| (a.id.as_str(), a.label.as_str(), a.placement))
            .collect::<Vec<_>>(),
        [
            ("close", "Close poll", UiActionPlacement::Menu),
            ("cancel", "取消投票", UiActionPlacement::Menu)
        ]
    );
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["menuActions"][0]["placement"], json!("menu"));
    assert!(json["actions"][0].get("placement").is_none());
    // hiddenWhen applies in the menu too; an empty menu is not serialized.
    let view = contribution.resolve(&json!({"canClose": false}), "", "u1");
    assert_eq!(view.menu_actions.len(), 1);
    // An action a block owns cannot go to the menu.
    let owned: UiDocument = serde_json::from_value(json!({"title": "t",
        "blocks": [{"kind": "choices", "options": [{"label": "a"}], "action": "vote"}],
        "actions": [{"id": "vote", "label": "投票", "placement": "menu",
                     "target": {"kind": "operation", "operation": "x.op.vote"}}]}))
    .unwrap();
    assert!(owned.validate().is_err());
}

// ---- account entries ----

fn account_entry() -> Value {
    json!({
        "kind": "account-entry", "id": "example.poll.wallet-entry", "schemaVersion": 1,
        "requiredPermissions": [], "icon": "wallet", "order": 100,
        "schema": {"title": "钱包", "translations": {"en": {"title": "Wallet"}},
                   "actions": [{"id": "open", "label": "钱包",
                                "target": {"kind": "page", "page": "example.poll.wallet"}}]},
        "summary": {"operation": "example.poll.wallet", "path": "/balanceText"}
    })
}
fn with_entry(entry: Value) -> Value {
    let mut manifest = platform_manifest();
    manifest["clientExtensions"]["contributions"]
        .as_array_mut()
        .unwrap()
        .push(entry);
    manifest
}

#[test]
fn account_entries_hold_one_page_action_and_an_optional_query_summary() {
    let manifest = parse(&with_entry(account_entry())).unwrap();
    let entry = manifest.contribution("example.poll.wallet-entry").unwrap();
    assert_eq!(entry.kind, UiContributionKind::AccountEntry);
    assert_eq!(entry.schema.localized_title("en-US"), "Wallet");
    assert_eq!(
        serde_json::to_value(entry.kind).unwrap(),
        json!("account-entry")
    );
    let summary = entry.summary.as_ref().unwrap();
    assert_eq!(
        summary.text(&json!({"balanceText": "¥12.30"})).as_deref(),
        Some("¥12.30")
    );
    assert_eq!(summary.text(&json!({"balanceText": "  "})), None);
    assert_eq!(summary.text(&json!({"balanceText": {"a": 1}})), None);
    assert_eq!(
        summary.text(&json!({"balanceText": 12})).as_deref(),
        Some("12")
    );
    // Without a summary it is valid too.
    let mut bare = account_entry();
    bare.as_object_mut().unwrap().remove("summary");
    assert!(parse(&with_entry(bare)).is_ok());

    let reject = |edit: &dyn Fn(&mut Value)| {
        let mut entry = account_entry();
        edit(&mut entry);
        parse(&with_entry(entry)).is_err()
    };
    assert!(reject(
        &|e| e["summary"]["operation"] = json!("example.poll.vote")
    ));
    assert!(reject(&|e| e["summary"]["path"] = json!("balanceText")));
    assert!(reject(&|e| e["schema"]["actions"][0]["target"] =
        json!({"kind": "page", "page": "example.poll.missing"})));
    assert!(reject(&|e| e["schema"]["actions"][0]["target"] =
        json!({"kind": "operation", "operation": "example.poll.view"})));
    assert!(reject(&|e| e["schema"]["actions"] = json!([])));
    assert!(reject(
        &|e| e["schema"]["blocks"] = json!([{"kind": "text", "text": "x"}])
    ));
    assert!(reject(
        &|e| e["schema"]["fields"] = json!([{"label": "x", "path": "/x"}])
    ));
    assert!(reject(
        &|e| e["audience"] = json!({"conversationTypes": ["group"]})
    ));
    assert!(reject(&|e| e["icon"] = json!("Wallet")));
    // summary on any other kind is refused.
    let mut manifest = platform_manifest();
    let card = card_index(&manifest);
    manifest["clientExtensions"]["contributions"][card]["summary"] =
        json!({"operation": "example.poll.wallet", "path": "/x"});
    assert!(parse(&manifest).is_err());
}

// ---- composer entry availability ----

#[test]
fn composer_entries_take_an_optional_query_availability_that_only_true_admits() {
    let mut manifest = platform_manifest();
    manifest["clientExtensions"]["contributions"][1]["availability"] =
        json!({"operation": "example.poll.view", "path": "/available"});
    let parsed = parse(&manifest).unwrap();
    let entry = parsed.contribution("example.poll.create").unwrap();
    assert_eq!(entry.kind, UiContributionKind::ComposerAction);
    let availability = entry.availability.as_ref().unwrap();
    assert!(availability.admits(&json!({"available": true})));
    for refused in [
        json!({"available": false}),
        json!({"available": "true"}),
        json!({"available": 1}),
        json!({}),
        Value::Null,
    ] {
        assert!(!availability.admits(&refused), "{refused}");
    }
    assert!(parsed.client_operation_refs().contains("example.poll.view"));
    // Absent, it serializes as before.
    let bare = parse(&platform_manifest()).unwrap();
    assert!(
        serde_json::to_value(bare.contribution("example.poll.create").unwrap())
            .unwrap()
            .get("availability")
            .is_none()
    );

    let reject = |index: usize, availability: Value| {
        let mut manifest = platform_manifest();
        manifest["clientExtensions"]["contributions"][index]["availability"] = availability;
        parse(&manifest).is_err()
    };
    // A command, an undeclared operation or a malformed path is refused.
    assert!(reject(
        1,
        json!({"operation": "example.poll.vote", "path": "/available"})
    ));
    assert!(reject(
        1,
        json!({"operation": "example.poll.missing", "path": "/available"})
    ));
    assert!(reject(
        1,
        json!({"operation": "example.poll.view", "path": "available"})
    ));
    assert!(reject(1, json!({"operation": "", "path": "/available"})));
    // Only composer-action takes it.
    let card = card_index(&platform_manifest());
    assert!(reject(
        card,
        json!({"operation": "example.poll.view", "path": "/available"})
    ));
    let mut entry = account_entry();
    entry["availability"] = json!({"operation": "example.poll.view", "path": "/available"});
    assert!(parse(&with_entry(entry)).is_err());
}

// ---- entity lifecycle ----

#[test]
fn entity_lifecycle_needs_an_entity_card_and_a_declared_operation() {
    let mut manifest = platform_manifest();
    let card = card_index(&manifest);
    let warnings = parse(&manifest).unwrap().warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].field.contains("example.poll.card"));

    manifest["clientExtensions"]["contributions"][card]["entityLifecycle"] =
        json!({"unbound": "example.poll.unbound"});
    let parsed = parse(&manifest).unwrap();
    assert!(parsed.warnings().is_empty());
    assert_eq!(
        parsed
            .contribution("example.poll.card")
            .unwrap()
            .entity_lifecycle
            .as_ref()
            .unwrap()
            .unbound,
        "example.poll.unbound"
    );

    let mut undeclared = manifest.clone();
    undeclared["clientExtensions"]["contributions"][card]["entityLifecycle"] =
        json!({"unbound": "example.poll.missing"});
    assert!(parse(&undeclared).is_err());
    let mut unbound_card = manifest.clone();
    unbound_card["clientExtensions"]["contributions"][card]
        .as_object_mut()
        .unwrap()
        .remove("entityPath");
    assert!(parse(&unbound_card).is_err(), "needs entityPath");
    let mut other_kind = platform_manifest();
    other_kind["clientExtensions"]["contributions"][1]["entityLifecycle"] =
        json!({"unbound": "example.poll.unbound"});
    assert!(parse(&other_kind).is_err());
}

// ---- admin section ----

fn admin_manifest() -> Value {
    let mut manifest = platform_manifest();
    for op in manifest["capabilities"][0]["operations"]
        .as_array_mut()
        .unwrap()
    {
        if op["id"] == "list" {
            op["mode"] = json!("query");
        }
    }
    manifest["configurationSchema"] = json!({
        "type": "object", "properties": {"maxOptions": {"type": "integer", "default": 20}}
    });
    let version = manifest["version"].as_str().unwrap().to_owned();
    manifest["admin"] = json!({
        "contractVersion": 1,
        "permissions": [
            {"id": "read", "label": {"zh-CN": "查看投票", "en-US": "View polls"}, "risk": "low",
             "defaultRoles": ["org_owner", "org_admin", "im_operator"]},
            {"id": "manage", "label": {"zh-CN": "管理投票"}, "risk": "high",
             "defaultRoles": ["org_owner"]}
        ],
        "operations": [
            {"id": "example.poll.list", "mode": "query", "permission": "read"},
            {"id": "example.poll.remove", "mode": "command", "permission": "manage",
             "requiresReason": true, "auditTarget": "/pollId"}
        ],
        "navigation": [
            {"id": "polls", "section": "plugins", "label": {"zh-CN": "投票"}, "icon": "poll",
             "order": 110, "page": "polls"}
        ],
        "pages": [
            {"id": "polls", "kind": "list", "permission": "read", "document": {
                "title": "投票",
                "blocks": [{"kind": "rows", "source": "example.poll.list", "path": "/items",
                            "keyPath": "/pollId", "columns": [{"label": "题目", "path": "/question"}],
                            "actions": ["remove"]}],
                "actions": [{"id": "remove", "label": "删除", "input": {"pollId": "/pollId"},
                             "target": {"kind": "operation", "operation": "example.poll.remove"}}]
            }},
            {"id": "overview", "kind": "dashboard", "permission": "read",
             "document": {"title": "概览"},
             "widgets": [{"widget": "kit:stat", "label": {"zh-CN": "进行中"},
                          "source": "example.poll.list", "path": "/openCount"}]}
        ],
        "settings": {"uiHints": {"maxOptions": {"widget": "number", "group": "limits"}}},
        "events": {"emit": [], "subscribe": [{"id": "flare.im.message.recalled", "schemaVersion": 1}]},
        "lifecycle": {
            "migrations": [{"from": ">=0.1.0, <0.2.0", "to": version, "mode": "offline",
                            "reversible": false}],
            "uninstall": {"behavior": "export-then-retain", "export": true}
        }
    });
    manifest
}

#[test]
fn the_admin_section_declares_permissions_operations_pages_and_lifecycle() {
    let manifest = parse(&admin_manifest()).unwrap();
    let admin = manifest.admin.as_ref().unwrap();
    assert!(manifest.admin_operations().contains("example.poll.list"));
    assert!(manifest.admin_operations().contains("example.poll.remove"));
    assert_eq!(
        manifest
            .admin_permission_for("example.poll.list")
            .as_deref(),
        Some("plugin:example.poll:read")
    );
    assert_eq!(
        manifest
            .admin_permission_for("example.poll.remove")
            .as_deref(),
        Some("plugin:example.poll:manage")
    );
    assert_eq!(manifest.admin_permission_for("example.poll.vote"), None);
    assert!(admin.lifecycle.as_ref().unwrap().configuration_carry_over);
    assert!(
        admin
            .migration_from(&semver::Version::parse("0.1.3").unwrap())
            .is_some_and(|m| !m.reversible)
    );
    assert!(
        admin
            .migration_from(&semver::Version::parse("0.3.0").unwrap())
            .is_none()
    );
    // Round trip.
    let again = parse(&serde_json::to_value(&manifest).unwrap()).unwrap();
    assert_eq!(again.admin, manifest.admin);

    let reject = |edit: &dyn Fn(&mut Value)| {
        let mut manifest = admin_manifest();
        edit(&mut manifest["admin"]);
        parse(&manifest).is_err()
    };
    assert!(reject(&|a| a["contractVersion"] = json!(2)));
    assert!(
        reject(&|a| a["permissions"][1]["id"] = json!("read")),
        "duplicate"
    );
    assert!(reject(&|a| a["permissions"][0]["id"] = json!("Read")));
    assert!(reject(&|a| a["permissions"][0]["label"] = json!({})));
    assert!(
        reject(&|a| a["operations"][0]["mode"] = json!("command")),
        "mode must match"
    );
    assert!(reject(
        &|a| a["operations"][0]["id"] = json!("example.poll.nope")
    ));
    assert!(reject(
        &|a| a["operations"][0]["permission"] = json!("audit")
    ));
    assert!(reject(
        &|a| a["operations"][1]["auditTarget"] = json!("pollId")
    ));
    assert!(
        reject(&|a| a["operations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "example.poll.details", "mode": "command", "permission": "read"}))),
        "the card's detail is a user operation"
    );
    assert!(reject(&|a| a["navigation"][0]["page"] = json!("missing")));
    assert!(reject(&|a| a["navigation"][0]["section"] = json!("billing")));
    assert!(reject(&|a| a["navigation"][0]["icon"] = json!("Poll")));
    assert!(
        reject(&|a| a["pages"][0]["kind"] = json!("form")),
        "a form has no rows"
    );
    assert!(reject(&|a| a["pages"][0]["permission"] = json!("audit")));
    assert!(reject(
        &|a| a["pages"][0]["document"]["actions"][0]["target"] =
            json!({"kind": "operation", "operation": "example.poll.vote"})
    ));
    assert!(
        reject(&|a| a["pages"][0]["document"]["actions"][0]["target"] =
            json!({"kind": "page", "page": "example.poll.wallet"})),
        "an admin row action calls an admin operation, never opens a page"
    );
    assert!(
        reject(&|a| a["pages"][1]["widgets"][0]["source"] = json!("example.poll.remove")),
        "a widget reads a query"
    );
    assert!(reject(
        &|a| a["pages"][1]["widgets"][0]["widget"] = json!("iframe")
    ));
    assert!(reject(&|a| a["pages"][1]["widgets"] = json!([])));
    assert!(reject(
        &|a| a["settings"]["uiHints"]["unknownKey"] = json!({"widget": "text"})
    ));
    assert!(reject(
        &|a| a["events"]["emit"] = json!([{"id": "flare.poll.undeclared", "schemaVersion": 1}])
    ));
    assert!(reject(
        &|a| a["lifecycle"]["migrations"][0]["to"] = json!("99.0.0")
    ));
    assert!(reject(
        &|a| a["lifecycle"]["migrations"][0]["from"] = json!("not a range")
    ));
    assert!(reject(&|a| a["extra"] = json!(true)));
}

#[test]
fn admin_workspaces_reference_bounded_unique_pages_without_parent_cycles() {
    let mut raw = admin_manifest();
    let mut form = raw["admin"]["pages"][0].clone();
    form["id"] = json!("helper");
    raw["admin"]["pages"].as_array_mut().unwrap().push(form);
    raw["admin"]["navigation"][0]["pages"] = json!([
        {"page":"polls","label":{"zh-CN":"记录"}},
        {"page":"helper","label":{"zh-CN":"辅助操作"},"placement":"tool","parentPage":"polls"}
    ]);
    let manifest = parse(&raw).unwrap();
    assert_eq!(manifest.admin.unwrap().navigation[0].pages.len(), 2);
    for pages in [
        json!([{"page":"missing","label":{"zh-CN":"未知页面"}}]),
        json!([{"page":"polls","label":{"zh-CN":"记录"}},{"page":"polls","label":{"zh-CN":"重复"}}]),
        json!([{"page":"polls","label":{"zh-CN":"记录"},"parentPage":"helper"},{"page":"helper","label":{"zh-CN":"辅助"},"parentPage":"polls"}]),
    ] {
        let mut bad = raw.clone();
        bad["admin"]["navigation"][0]["pages"] = pages;
        assert!(parse(&bad).is_err(), "invalid workspace accepted: {bad}");
    }
}

#[test]
fn staff_tickets_carry_the_admin_permission_of_their_plugin() {
    assert_eq!(
        plugin_permission("flare.poll", "manage").as_deref(),
        Some("plugin:flare.poll:manage")
    );
    assert_eq!(plugin_permission("flare.poll", ""), None);
    assert_eq!(plugin_permission("flare.poll", "a:b"), None);
    assert_eq!(
        plugin_permission_target("plugin:flare.poll:manage"),
        Some(("flare.poll", "manage"))
    );
    assert_eq!(plugin_permission_target("core:plugins:write"), None);
    let signer = PluginCallSigner::new("call-1".into(), [7; 32]).unwrap();
    let staff = |permission: &str| PluginStaffCallClaims {
        scope: Default::default(),
        tenant_id: "201".into(),
        plugin_id: "flare.poll".into(),
        plugin_version: "0.2.0".into(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        deployment_id: "poll-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 3,
        configuration_digest: configuration_digest(&json!({})).unwrap(),
        staff_id: "staff-1".into(),
        org_id: None,
        permission: permission.into(),
        operation: "flare.poll.admin.close".into(),
        request_id: "req-1".into(),
        reason: None,
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    };
    for ok in ["plugin:flare.poll:manage", "plugin:flare.poll:read"] {
        assert!(signer.sign_staff(&staff(ok), 1000).is_ok(), "{ok}");
    }
    for bad in [
        "plugin:other.plugin:manage",
        "plugin:flare.poll:",
        "flare.poll:manage",
    ] {
        assert!(signer.sign_staff(&staff(bad), 1000).is_err(), "{bad}");
    }
}

// ---- envelope displayName and unavailable reasons ----

fn message(display_name: Option<&str>) -> PluginMessage {
    PluginMessage {
        plugin_id: "example.poll".into(),
        contribution_id: "example.poll.card".into(),
        schema_version: 1,
        summary: "[投票] 午饭".into(),
        payload: json!({"pollId": "p-1"}),
        display_name: display_name.map(str::to_owned),
    }
}

#[test]
fn display_name_is_bounded_display_text() {
    assert!(message(Some("投票")).validate().is_ok());
    assert!(message(None).validate().is_ok());
    assert!(
        message(Some(&"名".repeat(22))).validate().is_err(),
        "66 bytes"
    );
    assert!(message(Some(" ")).validate().is_err());
    assert!(message(Some("a\u{0007}")).validate().is_err());
    let json = serde_json::to_value(message(Some("投票"))).unwrap();
    assert_eq!(json["displayName"], json!("投票"));
    assert!(
        serde_json::to_value(message(None))
            .unwrap()
            .get("displayName")
            .is_none()
    );
}

fn catalog(platforms: Value, unavailable: Value) -> UiCatalog {
    let mut manifest = manifest_value();
    manifest["clientExtensions"]["platforms"] = platforms;
    let manifest = parse(&manifest).unwrap();
    UiCatalog {
        tenant_id: "201".into(),
        version: 1,
        valid_until_ms: 10_000,
        plugins: vec![ClientPlugin {
            operations: manifest.operation_ids(),
            plugin_id: manifest.plugin_id,
            version: manifest.version,
            manifest_digest: format!("sha256:{}", "a".repeat(64)),
            grant_revision: 1,
            extensions: manifest.client_extensions,
            configuration: Default::default(),
        }],
        unavailable: serde_json::from_value(unavailable).unwrap(),
    }
}
fn context(tenant: &str, now_ms: i64) -> UiRenderContext<'_> {
    UiRenderContext {
        tenant,
        platform: ClientPlatform::Web,
        kind: UiContributionKind::MessageContent,
        locale: "",
        viewer: "u1",
        now_ms,
    }
}

#[test]
fn inert_views_say_why() {
    let catalog = catalog(json!(["web"]), json!([]));
    let reason = |catalog: &UiCatalog, message: &PluginMessage, tenant: &str, now: i64| {
        let view =
            catalog.resolve_with_state(message, UiStateLayers::default(), context(tenant, now));
        assert_eq!(
            view.available,
            view.unavailable_reason.is_none(),
            "{view:?}"
        );
        view.unavailable_reason
    };
    assert_eq!(reason(&catalog, &message(None), "201", 1000), None);
    assert_eq!(
        reason(&catalog, &message(None), "202", 1000),
        Some(UiUnavailableReason::NotInstalled)
    );
    assert_eq!(
        reason(&catalog, &message(None), "201", 10_000),
        Some(UiUnavailableReason::CatalogUnreachable)
    );
    let mut other = message(None);
    other.plugin_id = "example.other".into();
    other.contribution_id = "example.other.card".into();
    assert_eq!(
        reason(&catalog, &other, "201", 1000),
        Some(UiUnavailableReason::NotInstalled)
    );
    let listed = self::catalog(
        json!(["web"]),
        json!([{"pluginId": "example.other", "reason": "disabled"}]),
    );
    assert_eq!(
        reason(&listed, &other, "201", 1000),
        Some(UiUnavailableReason::Disabled)
    );
    let no_web = self::catalog(json!(["ios"]), json!([]));
    assert_eq!(
        reason(&no_web, &message(None), "201", 1000),
        Some(UiUnavailableReason::UnsupportedPlatform)
    );
    let mut unknown = message(None);
    unknown.contribution_id = "example.poll.gone".into();
    assert_eq!(
        reason(&catalog, &unknown, "201", 1000),
        Some(UiUnavailableReason::IncompatibleVersion)
    );
    let mut newer = message(None);
    newer.schema_version = 2;
    assert_eq!(
        reason(&catalog, &newer, "201", 1000),
        Some(UiUnavailableReason::IncompatibleVersion)
    );
    let view = catalog.resolve_with_state(&other, UiStateLayers::default(), context("201", 1000));
    assert_eq!(view.title, "[投票] 午饭", "the inert summary");
    assert_eq!(
        serde_json::to_value(&view).unwrap()["unavailableReason"],
        json!("not_installed")
    );
    assert_eq!(
        catalog.unavailable_reason(&message(None), &context("201", 1000)),
        None
    );
    // Every reason has its wire name.
    for (reason, wire) in [
        (UiUnavailableReason::Disabled, "disabled"),
        (UiUnavailableReason::NotInstalled, "not_installed"),
        (UiUnavailableReason::NoPermission, "no_permission"),
        (
            UiUnavailableReason::UnsupportedPlatform,
            "unsupported_platform",
        ),
        (
            UiUnavailableReason::IncompatibleVersion,
            "incompatible_version",
        ),
        (
            UiUnavailableReason::CatalogUnreachable,
            "catalog_unreachable",
        ),
        (UiUnavailableReason::UnboundCopy, "unbound_copy"),
    ] {
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(wire));
    }
}

#[test]
fn a_read_only_copy_keeps_its_content_and_drops_everything_interactive() {
    let catalog = catalog(json!(["web"]), json!([]));
    let view = catalog
        .resolve_with_state(
            &message(None),
            UiStateLayers::default(),
            context("201", 1000),
        )
        .into_read_only();
    assert!(view.available);
    assert_eq!(
        view.unavailable_reason,
        Some(UiUnavailableReason::UnboundCopy)
    );
    assert!(view.actions.is_empty() && view.menu_actions.is_empty());
    assert!(view.inputs.is_empty());
    assert!(!view.detail);
    for block in &view.blocks {
        match block {
            ResolvedUiBlock::Choices(choices) => assert!(choices.action.is_none()),
            ResolvedUiBlock::Entries(entries) => assert!(!entries.more),
            _ => {}
        }
    }
    // An inert view stays as it is.
    let inert = ResolvedPluginView::unavailable(&message(None), UiUnavailableReason::Disabled);
    assert_eq!(inert.clone().into_read_only(), inert);
}
