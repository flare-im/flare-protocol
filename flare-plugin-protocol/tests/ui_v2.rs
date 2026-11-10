//! UiDocument v2: blocks, inputs, forms, detail, audience, admin pages,
//! state layers and resolution.
use flare_plugin_protocol::{
    PluginManifest, ProtocolError,
    manifest::{ClientPlatform, UiContribution, UiContributionKind},
    state::SharedState,
    ui::*,
};
use serde_json::{Value, json};

fn manifest() -> PluginManifest {
    PluginManifest::from_json(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap()
}
fn contribution(id: &str) -> UiContribution {
    manifest().contribution(id).unwrap().clone()
}
/// The fixture's console page (`admin.pages`).
fn admin_page() -> flare_plugin_protocol::admin::AdminPage {
    manifest().admin.unwrap().page("polls").unwrap().clone()
}
fn catalog() -> UiCatalog {
    let manifest = manifest();
    UiCatalog {
        tenant_id: "201".into(),
        version: 1,
        valid_until_ms: 10000,
        plugins: vec![ClientPlugin {
            query_operations: Default::default(),
            operations: manifest.operation_ids(),
            plugin_id: manifest.plugin_id,
            version: manifest.version,
            manifest_digest: format!("sha256:{}", "a".repeat(64)),
            grant_revision: 1,
            extensions: manifest.client_extensions,
            configuration: Default::default(),
        }],
        unavailable: vec![],
    }
}
fn context<'a>(locale: &'a str, viewer: &'a str) -> UiRenderContext<'a> {
    UiRenderContext {
        tenant: "201",
        platform: ClientPlatform::Web,
        kind: UiContributionKind::MessageContent,
        locale,
        viewer,
        now_ms: 1000,
    }
}
fn card(payload: Value) -> PluginMessage {
    PluginMessage {
        plugin_id: "example.poll".into(),
        contribution_id: "example.poll.card".into(),
        schema_version: 1,
        summary: "投票".into(),
        payload,
        display_name: None,
    }
}
fn doc(raw: Value) -> UiDocument {
    serde_json::from_value(raw).unwrap()
}
/// Resolve a standalone document (no detail) against `data`.
fn resolve(document: UiDocument, data: Value) -> ResolvedPluginView {
    let contribution: UiContribution = serde_json::from_value(json!({
        "kind": "message-content", "id": "x.card", "schemaVersion": 1,
        "requiredPermissions": [], "schema": document
    }))
    .unwrap();
    contribution.schema.validate().unwrap();
    contribution.resolve(&data, "", "u1")
}
fn op(id: &str) -> Value {
    json!({"kind": "operation", "operation": format!("x.op.{id}")})
}
fn only_block(view: &ResolvedPluginView) -> &ResolvedUiBlock {
    assert_eq!(view.blocks.len(), 1, "{view:#?}");
    &view.blocks[0]
}

#[test]
fn v2_manifest_validates_and_round_trips() {
    let manifest = manifest();
    let raw = serde_json::to_value(&manifest).unwrap();
    let again: PluginManifest = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(again, manifest);
    again.validate().unwrap();
    // Defaults stay out of the serialized form.
    let card = &raw["clientExtensions"]["contributions"][0];
    assert_eq!(card["entityPath"], json!("/pollId"));
    assert!(card.get("forwardable").is_none());
    assert!(card.get("audience").is_none());
    let choices = &card["schema"]["blocks"][1];
    assert!(choices.get("showResults").is_none());
    assert!(choices.get("inputName").is_none());
    assert_eq!(
        manifest.admin_operations().into_iter().collect::<Vec<_>>(),
        [
            "example.poll.end",
            "example.poll.inspect",
            "example.poll.list",
            "example.poll.remove"
        ]
    );
    assert_eq!(
        contribution("example.poll.card").entity_of(&json!({"pollId": "p-1"})),
        Some("p-1".into())
    );
    // Console pages live in the `admin` section, never among the contributions.
    assert!(serde_json::from_value::<UiContributionKind>(json!("admin-page")).is_err());
}

#[test]
fn red_packet_0_4_1_manifest_still_parses_and_resolves_as_before() {
    let raw = include_bytes!("fixtures/red-packet-0.4.1.json");
    let manifest = PluginManifest::from_json(raw).unwrap();
    assert_eq!(
        (manifest.plugin_id.as_str(), manifest.version.as_str()),
        ("flare.redpacket", "0.4.1")
    );
    // Re-serialized, no v2 key appears on any contribution or document.
    let reserialized = serde_json::to_string(&manifest.client_extensions).unwrap();
    for key in [
        "entityPath",
        "forwardable",
        "audience",
        "detail",
        "blocks",
        "inputs",
        "form",
        "confirm",
        "style",
        "exists",
        "texts",
        "configuration",
    ] {
        assert!(
            !reserialized.contains(&format!("\"{key}\"")),
            "{key} leaked"
        );
    }
    let original: Value = serde_json::from_slice(raw).unwrap();
    assert_eq!(
        serde_json::to_value(&manifest.client_extensions).unwrap(),
        original["clientExtensions"]
    );
    let card = manifest.contribution("flare.redpacket.card").unwrap();
    assert_eq!(card.entity_path, None);
    assert!(!card.forwardable);
    let view = card.resolve(
        &json!({"packetId": "p", "greeting": "恭喜", "kind": "lucky", "status": "open", "recipients": ["u1"], "canClaim": true}),
        "en-US",
        "u1",
    );
    assert_eq!(view.title, "Red packet");
    assert_eq!(view.fields[1].value, "Lucky draw");
    assert_eq!(
        view.actions
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["details", "claim"]
    );
    assert!(view.blocks.is_empty() && view.inputs.is_empty() && !view.detail);
    let json = serde_json::to_value(&view).unwrap();
    for key in ["blocks", "inputs", "detail"] {
        assert!(json.get(key).is_none(), "{key} serialized on a v1 view");
    }
}

#[test]
fn poll_card_resolves_layers_choices_and_owned_actions() {
    let catalog = catalog();
    let message = card(json!({
        "pollId": "p-1", "question": "午饭吃什么？", "options": ["面", "饭", "饺子"],
        "creatorId": "u1", "deadlineMs": 5000, "closed": false
    }));
    let shared = SharedState {
        entity: "p-1".into(),
        version: 3,
        values: serde_json::from_value(json!({"counts": [2, 1, 0], "voterCount": 3})).unwrap(),
    };
    let local: PluginMessageState =
        serde_json::from_value(json!({"version": 1, "values": {"mySelection": [0]}})).unwrap();
    let layers = UiStateLayers {
        shared: Some(&shared),
        local: Some(&local),
    };
    let view = catalog.resolve_with_state(&message, layers, context("en-US", "u1"));
    assert!(view.available);
    assert_eq!(view.title, "Poll");
    assert!(view.detail);
    // vote belongs to the choices block; close is a button for the creator.
    assert_eq!(view.actions.len(), 1);
    assert_eq!(view.actions[0].id, "close");
    assert_eq!(view.actions[0].label, "Close poll");
    assert_eq!(
        view.actions[0].confirm.as_deref(),
        Some("No one can vote after the poll is closed")
    );
    assert_eq!(view.actions[0].style, Some(UiActionStyle::Danger));
    let ResolvedUiBlock::Text(question) = &view.blocks[0] else {
        panic!("{:?}", view.blocks[0])
    };
    assert_eq!(
        (question.text.as_str(), question.style, question.max_lines),
        ("午饭吃什么？", UiTextStyle::Title, Some(3))
    );
    let ResolvedUiBlock::Choices(choices) = &view.blocks[1] else {
        panic!("{:?}", view.blocks[1])
    };
    assert!(choices.voted && choices.show_results && !choices.closed && !choices.multiple);
    assert_eq!(choices.total, Some(3));
    assert_eq!(choices.input_name, "selection");
    let rows: Vec<_> = choices
        .options
        .iter()
        .map(|o| (o.index, o.label.as_str(), o.selected, o.count, o.percent))
        .collect();
    assert_eq!(
        rows,
        [
            (0, "面", true, Some(2), Some(67)),
            (1, "饭", false, Some(1), Some(33)),
            (2, "饺子", false, Some(0), Some(0)),
        ]
    );
    let submit = choices.action.as_ref().unwrap();
    assert_eq!(
        (submit.id.as_str(), submit.label.as_str()),
        ("vote", "Vote")
    );
    let ResolvedUiBlock::Time(deadline) = &view.blocks[2] else {
        panic!()
    };
    assert_eq!(
        (
            deadline.label.as_deref(),
            deadline.at_ms,
            deadline.countdown
        ),
        (Some("Ends"), 5000, true)
    );
    let ResolvedUiBlock::Status(status) = &view.blocks[3] else {
        panic!()
    };
    assert_eq!(
        (status.text.as_str(), status.tone),
        ("Open", UiTone::Success)
    );
    let ResolvedUiBlock::Note(note) = &view.blocks[4] else {
        panic!()
    };
    assert_eq!(note.text, "Anonymous poll");

    // Another viewer, before voting: no results, no close button.
    let fresh = catalog.resolve_with_state(
        &message,
        UiStateLayers {
            shared: Some(&shared),
            local: None,
        },
        context("zh-CN", "u2"),
    );
    assert!(fresh.actions.is_empty());
    let ResolvedUiBlock::Choices(choices) = &fresh.blocks[1] else {
        panic!()
    };
    assert!(!choices.voted && !choices.show_results);
    assert!(
        choices
            .options
            .iter()
            .all(|o| o.count.is_none() && o.percent.is_none())
    );
    assert_eq!(choices.total, None);
    assert_eq!(choices.action.as_ref().unwrap().label, "投票");

    // Closed by a newer shared state: results for everyone, no choosing.
    let closed = SharedState {
        version: 4,
        values: serde_json::from_value(
            json!({"counts": [2, 2, 0], "voterCount": 4, "closed": true}),
        )
        .unwrap(),
        ..shared.clone()
    };
    let view = catalog.resolve_with_state(
        &message,
        UiStateLayers {
            shared: Some(&closed),
            local: None,
        },
        context("zh-CN", "u2"),
    );
    let ResolvedUiBlock::Choices(choices) = &view.blocks[1] else {
        panic!()
    };
    assert!(choices.closed && choices.show_results && choices.action.is_none());
    assert_eq!(choices.options[0].percent, Some(50));
    let ResolvedUiBlock::Status(status) = &view.blocks[3] else {
        panic!()
    };
    assert_eq!(status.text, "已结束");

    // The shared layer of another entity, or an invalid one, is ignored.
    let other = SharedState {
        entity: "p-2".into(),
        ..closed.clone()
    };
    let invalid = SharedState {
        version: 0,
        ..closed.clone()
    };
    for layer in [&other, &invalid] {
        let view = catalog.resolve_with_state(
            &message,
            UiStateLayers {
                shared: Some(layer),
                local: None,
            },
            context("zh-CN", "u2"),
        );
        let ResolvedUiBlock::Choices(choices) = &view.blocks[1] else {
            panic!()
        };
        assert!(!choices.closed);
    }
    // No layers: exactly resolve_with_context.
    assert_eq!(
        catalog.resolve_with_state(&message, UiStateLayers::default(), context("en", "u1")),
        catalog.resolve_with_context(&message, context("en", "u1"))
    );
    // The resolved view round-trips for hosts that parse it back.
    let raw = serde_json::to_value(&view).unwrap();
    assert_eq!(raw["blocks"][1]["kind"], json!("choices"));
    assert_eq!(
        serde_json::from_value::<ResolvedPluginView>(raw).unwrap(),
        view
    );
}

#[test]
fn state_layers_merge_payload_then_shared_then_local() {
    let card = contribution("example.poll.card");
    let shared = SharedState {
        entity: "7".into(),
        version: 1,
        values: serde_json::from_value(json!({"a": "shared", "b": "shared", "gone": null}))
            .unwrap(),
    };
    let local: PluginMessageState = serde_json::from_value(
        json!({"version": 9, "basedOnVersion": 1, "values": {"b": "local"}}),
    )
    .unwrap();
    let merged = UiStateLayers {
        shared: Some(&shared),
        local: Some(&local),
    }
    .merge(
        &card,
        &json!({"pollId": 7, "a": "payload", "gone": "x", "keep": 1}),
    );
    assert_eq!(
        merged,
        json!({"pollId": 7, "a": "shared", "b": "local", "keep": 1})
    );
    // A contribution without entityPath never takes a shared layer.
    let mut unbound = card.clone();
    unbound.entity_path = None;
    let merged = UiStateLayers {
        shared: Some(&shared),
        local: None,
    }
    .merge(&unbound, &json!({"pollId": 7, "a": "payload"}));
    assert_eq!(merged, json!({"pollId": 7, "a": "payload"}));
}

#[test]
fn text_fields_status_progress_time_divider_and_note_blocks() {
    let view = resolve(
        doc(json!({"title": "t", "blocks": [
            {"kind": "text", "text": "第一行\n第二行"},
            {"kind": "text", "path": "/body", "maxLines": 4},
            {"kind": "text", "path": "/missing"},
            {"kind": "fields", "fields": [{"label": "地点", "path": "/place"}, {"label": "空", "path": "/none"}]},
            {"kind": "fields", "fields": [{"label": "空", "path": "/none"}]},
            {"kind": "status", "label": "状态", "path": "/state", "values": {"done": {"label": "完成", "tone": "success"}}},
            {"kind": "status", "path": "/odd"},
            {"kind": "progress", "label": "进度", "valuePath": "/done", "totalPath": "/all"},
            {"kind": "progress", "valuePath": "/done", "totalPath": "/zero"},
            {"kind": "time", "path": "/start", "endPath": "/end"},
            {"kind": "time", "path": "/badTime"},
            {"kind": "divider"},
            {"kind": "note", "path": "/hint", "tone": "warning"}
        ]})),
        json!({"body": "正文\u{0007}内容", "place": "3F", "state": "done", "odd": "paused",
               "done": 2, "all": 3, "zero": 0, "start": "1700000000000", "end": 1700003600000_i64,
               "badTime": "soon", "hint": "仅群主可见"}),
    );
    let blocks: Vec<Value> = view
        .blocks
        .iter()
        .map(|b| serde_json::to_value(b).unwrap())
        .collect();
    assert_eq!(
        blocks,
        [
            json!({"kind": "text", "text": "第一行\n第二行", "style": "body"}),
            json!({"kind": "text", "text": "正文内容", "maxLines": 4, "style": "body"}),
            json!({"kind": "fields", "fields": [{"label": "地点", "value": "3F"}]}),
            json!({"kind": "status", "label": "状态", "text": "完成", "tone": "success"}),
            json!({"kind": "status", "text": "paused", "tone": "neutral"}),
            json!({"kind": "progress", "label": "进度", "value": 2, "total": 3, "percent": 67}),
            json!({"kind": "progress", "value": 2, "total": 0, "percent": 0}),
            json!({"kind": "time", "atMs": 1700000000000_i64, "endMs": 1700003600000_i64, "countdown": false}),
            json!({"kind": "divider"}),
            json!({"kind": "note", "text": "仅群主可见", "tone": "warning"}),
        ]
    );
    // Exactly one of text and path; bounded lines.
    for bad in [
        json!({"kind": "text"}),
        json!({"kind": "text", "text": "a", "path": "/a"}),
        json!({"kind": "text", "path": "/a", "maxLines": 0}),
        json!({"kind": "text", "path": "/a", "maxLines": 21}),
        json!({"kind": "text", "text": "x".repeat(2001)}),
        json!({"kind": "note", "text": "tab\there"}),
        json!({"kind": "fields", "fields": []}),
        json!({"kind": "status", "path": "/s", "values": {"": {"label": "x"}}}),
        json!({"kind": "time", "path": "relative"}),
    ] {
        let parsed: std::result::Result<UiDocument, _> =
            serde_json::from_value(json!({"title": "t", "blocks": [bad.clone()]}));
        assert!(
            parsed.map_or(true, |d| d.validate().is_err()),
            "{bad} must be rejected"
        );
    }
    assert!(
        serde_json::from_value::<UiDocument>(json!({"title": "t", "blocks": [{"kind": "html"}]}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<UiDocument>(
            json!({"title": "t", "blocks": [{"kind": "divider", "x": 1}]})
        )
        .is_err()
    );
}

#[test]
fn choices_show_results_modes_multiple_and_selection_input() {
    let base = |show: &str| {
        doc(json!({"title": "t",
            "actions": [{"id": "vote", "label": "投", "target": op("vote"), "input": {"id": "/id"}}],
            "blocks": [{"kind": "choices", "optionsPath": "/options", "labelPath": "/text",
                "countsPath": "/counts", "selectedPath": "/mine", "multiplePath": "/multi",
                "showResults": show, "closedWhen": [{"path": "/status", "anyOf": ["closed"]}],
                "action": "vote", "inputName": "picked"}]}))
    };
    let data = |mine: Value, status: &str| {
        json!({"id": "p", "options": [{"text": "A"}, {"text": "B"}], "counts": [1, "2"],
               "mine": mine, "multi": "true", "status": status})
    };
    let choices = |show: &str, mine: Value, status: &str| match only_block(&resolve(
        base(show),
        data(mine, status),
    )) {
        ResolvedUiBlock::Choices(c) => c.clone(),
        other => panic!("{other:?}"),
    };
    assert!(choices("always", json!([]), "open").show_results);
    assert!(!choices("voted", json!([]), "open").show_results);
    assert!(choices("voted", json!(1), "open").show_results);
    assert!(choices("voted", json!([]), "closed").show_results);
    assert!(!choices("closed", json!([1]), "open").show_results);
    let closed = choices("closed", json!([1]), "closed");
    assert!(closed.show_results && closed.action.is_none());
    // Counts accept integers and digit strings; the total defaults to their sum.
    assert_eq!(closed.total, Some(3));
    assert_eq!(closed.options[1].count, Some(2));
    assert_eq!(closed.options[1].percent, Some(67));
    assert!(closed.multiple);
    assert_eq!(closed.input_name, "picked");

    let document = base("always");
    let block = document.choices_for("vote").unwrap();
    let card = data(json!([]), "open");
    assert_eq!(
        block.selection_input(&card, &[1, 0]).unwrap(),
        ("picked".into(), json!([1, 0]))
    );
    for bad in [&[][..], &[2], &[0, 0]] {
        assert!(block.selection_input(&card, bad).is_err(), "{bad:?}");
    }
    let mut single = card.clone();
    single["multi"] = json!(false);
    assert!(block.selection_input(&single, &[0, 1]).is_err());
    assert!(block.selection_input(&single, &[1]).is_ok());
    let action = &document.actions[0];
    let (name, value) = block.selection_input(&card, &[1]).unwrap();
    assert_eq!(
        bind_action_input_with(action, &card, [(name, value)].into_iter().collect()).unwrap(),
        json!({"id": "p", "picked": [1]})
    );

    // Malformed option data hides the block instead of misaligning counts.
    for options in [
        json!([]),
        json!([{"text": "A"}, {"other": "B"}]),
        json!("A"),
        json!(vec!["x"; 51]),
    ] {
        let mut broken = card.clone();
        broken["options"] = options;
        assert!(resolve(base("always"), broken).blocks.is_empty());
    }
    // The selection name may not collide with the action input, and the action
    // must target an operation.
    for bad in [
        json!({"kind": "choices", "optionsPath": "/o", "action": "vote", "inputName": "id"}),
        json!({"kind": "choices", "optionsPath": "/o", "action": "page"}),
        json!({"kind": "choices", "optionsPath": "/o", "action": "missing"}),
        json!({"kind": "choices", "optionsPath": "/o", "inputName": "bad-name"}),
        json!({"kind": "choices", "optionsPath": "/o", "closedWhen": [{"path": "/s"}]}),
    ] {
        let document = doc(json!({"title": "t", "actions": [
            {"id": "vote", "label": "投", "target": op("vote"), "input": {"id": "/id"}},
            {"id": "page", "label": "页", "target": {"kind": "page", "page": "x.page"}}
        ], "blocks": [bad.clone()]}));
        assert!(document.validate().is_err(), "{bad}");
    }
    // One block owns an action.
    let twice = doc(json!({"title": "t",
        "actions": [{"id": "vote", "label": "投", "target": op("vote")}],
        "blocks": [{"kind": "choices", "optionsPath": "/o", "action": "vote"},
                   {"kind": "choices", "optionsPath": "/p", "action": "vote"}]}));
    assert!(twice.validate().is_err());
}

#[test]
fn entries_and_people_blocks() {
    let document = doc(json!({"title": "接龙", "blocks": [
        {"kind": "entries", "itemsPath": "/lines", "textPath": "/text", "namePath": "/name",
         "userPath": "/user", "limit": 2, "countPath": "/count", "emptyText": "还没有人参与"},
        {"kind": "entries", "itemsPath": "/plain", "numbered": false},
        {"kind": "people", "max": 2, "groups": [
            {"label": "已确认", "usersPath": "/confirmed", "tone": "success"},
            {"label": "未确认", "countPath": "/pending"},
            {"label": "缺失", "usersPath": "/nobody"}
        ]}
    ]}));
    let view = resolve(
        document.clone(),
        json!({
            "lines": [{"text": "两箱", "name": "Alice", "user": "u1"}, {"text": "一箱", "user": "bad id"}, {"text": "三箱"}],
            "count": 5,
            "plain": ["a", 7, {"x": 1}],
            "confirmed": ["u1", "u2", "u3", 4],
            "pending": "12"
        }),
    );
    let blocks: Vec<Value> = view
        .blocks
        .iter()
        .map(|b| serde_json::to_value(b).unwrap())
        .collect();
    // The document declares no detail, so nothing offers "view all".
    assert_eq!(
        blocks,
        [
            json!({"kind": "entries", "items": [
                {"index": 1, "text": "两箱", "name": "Alice", "userId": "u1"},
                {"index": 2, "text": "一箱"}
            ], "numbered": true, "total": 5, "more": false, "emptyText": "还没有人参与"}),
            json!({"kind": "entries", "items": [
                {"index": 1, "text": "a"}, {"index": 2, "text": "7"}
            ], "numbered": false, "total": 3, "more": false}),
            json!({"kind": "people", "groups": [
                {"label": "已确认", "userIds": ["u1", "u2"], "count": 3, "tone": "success"},
                {"label": "未确认", "userIds": [], "count": 12}
            ]}),
        ]
    );
    // Present but empty shows the empty text; missing hides the block.
    let empty = resolve(document, json!({"lines": []}));
    assert_eq!(
        serde_json::to_value(only_block(&empty)).unwrap(),
        json!({"kind": "entries", "items": [], "numbered": true, "total": 0, "more": false, "emptyText": "还没有人参与"})
    );
    for bad in [
        json!({"kind": "entries", "itemsPath": "/l", "limit": 0}),
        json!({"kind": "entries", "itemsPath": "/l", "limit": 51}),
        json!({"kind": "people", "groups": []}),
        json!({"kind": "people", "groups": [{"label": "x"}]}),
        json!({"kind": "people", "max": 11, "groups": [{"label": "x", "countPath": "/c"}]}),
        json!({"kind": "people", "groups": vec![json!({"label": "x", "countPath": "/c"}); 5]}),
    ] {
        assert!(
            doc(json!({"title": "t", "blocks": [bad.clone()]}))
                .validate()
                .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn admin_page_rows_bind_row_actions_detail_and_cursor() {
    let admin = admin_page();
    let data = json!({
        "items": [
            {"pollId": "p1", "question": "午饭？", "status": "open", "createdAtMs": 1700000000000_i64},
            {"pollId": "p2", "question": "晚饭？", "status": "closed"},
            {"question": "no key"}
        ],
        "nextCursor": "c-2"
    });
    let view = admin.resolve(&data, "", "staff-1");
    assert!(view.detail);
    assert!(
        view.actions.is_empty(),
        "row actions are owned by the rows block"
    );
    assert_eq!(view.inputs.len(), 1);
    let ResolvedUiBlock::Rows(rows) = only_block(&view) else {
        panic!()
    };
    assert_eq!(rows.source, "example.poll.list");
    assert_eq!(rows.next_cursor.as_deref(), Some("c-2"));
    assert_eq!(
        rows.columns
            .iter()
            .map(|c| (c.label.as_str(), c.format))
            .collect::<Vec<_>>(),
        [
            ("问题", UiColumnFormat::Text),
            ("状态", UiColumnFormat::Text),
            ("创建时间", UiColumnFormat::Time)
        ]
    );
    assert_eq!(
        rows.actions
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["close", "remove"]
    );
    assert_eq!(rows.rows.len(), 2, "a row without a key is skipped");
    let first = serde_json::to_value(&rows.rows[0]).unwrap();
    assert_eq!(
        first,
        json!({"key": "p1", "cells": [
            {"text": "午饭？"}, {"text": "进行中"}, {"text": "1700000000000", "timeMs": 1700000000000_i64}
        ], "actions": [
            {"id": "close", "input": {"pollId": "p1"}},
            {"id": "remove", "input": {"pollId": "p1"}}
        ], "detailInput": {"pollId": "p1"}})
    );
    // `close` is hidden on a closed row; a missing value is an empty cell.
    assert_eq!(
        rows.rows[1]
            .actions
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["remove"]
    );
    assert_eq!(rows.rows[1].cells[2].text, "");
    // Pages: filters plus the cursor; filters are validated like a form.
    let block = admin.document.rows().unwrap();
    let filters = admin
        .document
        .validate_inputs(&json!({"status": "open"}))
        .unwrap();
    assert_eq!(
        block.source_input(Some("c-2"), filters).unwrap(),
        json!({"status": "open", "cursor": "c-2"})
    );
    assert_eq!(
        block.source_input(None, Default::default()).unwrap(),
        json!({"cursor": ""})
    );
    assert!(
        block
            .source_input(Some(&"c".repeat(257)), Default::default())
            .is_err()
    );
    assert!(
        admin
            .document
            .validate_inputs(&json!({"status": "paused"}))
            .is_err()
    );
    // At most 100 rows are resolved.
    let many = json!({"items": (0..150).map(|i| json!({"pollId": format!("p{i}"), "status": "open"})).collect::<Vec<_>>()});
    let ResolvedUiBlock::Rows(rows) = only_block(&admin.resolve(&many, "", "s")).clone() else {
        panic!()
    };
    assert_eq!(rows.rows.len(), 100);
    assert_eq!(rows.next_cursor, None);
    // The detail resolves against the detail operation's result.
    let detail = admin
        .resolve_detail(
            &json!({"voters": [{"option": "面", "userId": "u1"}]}),
            "",
            "s",
        )
        .unwrap();
    let ResolvedUiBlock::Entries(entries) = only_block(&detail) else {
        panic!()
    };
    assert_eq!(entries.items[0].user_id.as_deref(), Some("u1"));
    assert!(!detail.detail);
}

fn edited(edit: impl Fn(&mut Value)) -> flare_plugin_protocol::Result<PluginManifest> {
    let mut raw: Value =
        serde_json::from_slice(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap();
    edit(&mut raw);
    PluginManifest::from_json(&serde_json::to_vec(&raw).unwrap())
}
/// The manifest field a rejection names (or the error itself).
fn rejected_field(result: flare_plugin_protocol::Result<PluginManifest>) -> String {
    match result {
        Err(ProtocolError::InvalidManifest { field, .. }) => field,
        Err(other) => format!("{other:?}"),
        Ok(_) => "accepted".into(),
    }
}
fn set(index: usize, key: &'static str, value: Value) -> impl Fn(&mut Value) {
    move |raw| raw["clientExtensions"]["contributions"][index][key] = value.clone()
}

#[test]
fn contribution_kind_rules_for_v2_fields() {
    // entityPath and forwardable: message-content only.
    assert_eq!(
        rejected_field(edited(set(1, "entityPath", json!("/pollId")))),
        "clientExtensions.contributions.entityPath"
    );
    assert_eq!(
        rejected_field(edited(set(0, "entityPath", json!("pollId")))),
        "clientExtensions.contributions.entityPath"
    );
    assert_eq!(
        rejected_field(edited(set(1, "forwardable", json!(true)))),
        "clientExtensions.contributions.forwardable"
    );
    assert!(edited(set(0, "forwardable", json!(true))).is_ok());
    // audience: composer-action only, no duplicates, roles need groups.
    assert_eq!(
        rejected_field(edited(set(
            0,
            "audience",
            json!({"conversationTypes": ["group"]})
        ))),
        "clientExtensions.contributions.audience"
    );
    for audience in [
        json!({"conversationTypes": ["group", "group"]}),
        json!({"conversationTypes": ["single"], "groupRoles": ["owner"]}),
        json!({"conversationTypes": ["channel"]}),
        json!({"groupRoles": ["admin", "admin"]}),
    ] {
        assert!(
            edited(set(1, "audience", audience.clone())).is_err(),
            "{audience}"
        );
    }
    // detail: message-content only, declared operation, no rows.
    assert_eq!(
        rejected_field(edited(set(
            1,
            "detail",
            json!({"operation": "example.poll.details", "document": {"title": "d"}})
        ))),
        "clientExtensions.contributions.detail"
    );
    assert_eq!(
        rejected_field(edited(set(
            0,
            "detail",
            json!({"operation": "example.poll.nope", "document": {"title": "d"}})
        ))),
        "clientExtensions.contributions.detail.operation"
    );
    let rows = json!({"kind": "rows", "source": "example.poll.list", "path": "/i", "keyPath": "/k",
                      "columns": [{"label": "c", "path": "/c"}]});
    assert!(
        edited(set(
            0,
            "detail",
            json!({"operation": "example.poll.details", "document": {"title": "d", "blocks": [rows.clone()]}})
        ))
        .is_err()
    );
    // rows: page documents only (console pages are the admin section's).
    let push_block = |index: usize, block: Value| {
        move |raw: &mut Value| {
            let blocks = &mut raw["clientExtensions"]["contributions"][index]["schema"]["blocks"];
            let mut list = blocks.as_array().cloned().unwrap_or_default();
            list.push(block.clone());
            *blocks = Value::Array(list);
        }
    };
    for index in [0, 1] {
        assert_eq!(
            rejected_field(edited(push_block(index, rows.clone()))),
            "clientExtensions.blocks.rows"
        );
    }
    // An admin page reads and acts through admin operations only, `cursor` is
    // reserved, and every permission code fits 128 bytes.
    let page = |edit: fn(&mut Value)| {
        move |raw: &mut Value| edit(&mut raw["admin"]["pages"][0]["document"])
    };
    for edit in [
        page(|d| d["blocks"][0]["source"] = json!("example.poll.nope")),
        page(|d| d["blocks"][0]["source"] = json!("example.poll.details")),
        page(|d| d["inputs"][0]["id"] = json!("cursor")),
        page(|d| {
            d["actions"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "snap", "label": "拍照", "target": {"kind": "native", "capability": "camera"}}))
        }),
    ] {
        assert_eq!(
            rejected_field(edited(move |raw| {
                raw["clientExtensions"]["nativeCapabilities"] = json!(["camera"]);
                edit(raw)
            })),
            "admin"
        );
    }
    let renamed = |id: String| {
        move |raw: &mut Value| {
            let text = serde_json::to_string(raw)
                .unwrap()
                .replace("example.poll", &id);
            *raw = serde_json::from_str(&text).unwrap();
        }
    };
    // `plugin:` + id + `:manage` is 129 bytes; one byte shorter fits.
    let long_id = format!("example.{}", "p".repeat(107));
    assert_eq!(rejected_field(edited(renamed(long_id))), "admin");
    assert!(edited(renamed(format!("example.{}", "p".repeat(106)))).is_ok());
}

#[test]
fn inputs_declare_kind_specific_fields_only() {
    let check = |input: Value| {
        doc(json!({"title": "t", "inputs": [input]}))
            .validate()
            .is_ok()
    };
    for good in [
        json!({"id": "a", "kind": "text", "label": "a", "maxLength": 1024, "placeholder": "p", "default": "x"}),
        json!({"id": "a", "kind": "textarea", "label": "a", "maxLength": 8000, "default": "l1\nl2"}),
        json!({"id": "a", "kind": "number", "label": "a", "min": -1.5, "max": 10, "default": 3}),
        json!({"id": "a", "kind": "toggle", "label": "a", "default": true}),
        json!({"id": "a", "kind": "select", "label": "a", "default": "y", "choices": [{"value": "x", "label": "X"}, {"value": "y", "label": "Y"}]}),
        json!({"id": "a", "kind": "datetime", "label": "a", "required": true, "default": 0}),
        json!({"id": "a", "kind": "options", "label": "a", "min": 2, "max": 50, "maxLength": 256, "default": ["", ""]}),
        json!({"id": "a", "kind": "members", "label": "a", "min": 1, "max": 500, "defaultPath": "/assignees"}),
    ] {
        assert!(check(good.clone()), "{good} must be accepted");
    }
    for bad in [
        json!({"id": "bad-id", "kind": "text", "label": "a"}),
        json!({"id": "a", "kind": "text", "label": ""}),
        json!({"id": "a", "kind": "text", "label": "a", "maxLength": 1025}),
        json!({"id": "a", "kind": "text", "label": "a", "maxLength": 0}),
        json!({"id": "a", "kind": "text", "label": "a", "min": 1}),
        json!({"id": "a", "kind": "text", "label": "a", "default": 5}),
        json!({"id": "a", "kind": "text", "label": "a", "default": "line\nbreak"}),
        json!({"id": "a", "kind": "textarea", "label": "a", "maxLength": 8001}),
        json!({"id": "a", "kind": "number", "label": "a", "min": 5, "max": 1}),
        json!({"id": "a", "kind": "number", "label": "a", "max": 1, "default": 2}),
        json!({"id": "a", "kind": "number", "label": "a", "maxLength": 3}),
        json!({"id": "a", "kind": "toggle", "label": "a", "required": true}),
        json!({"id": "a", "kind": "select", "label": "a"}),
        json!({"id": "a", "kind": "select", "label": "a", "default": "z", "choices": [{"value": "x", "label": "X"}]}),
        json!({"id": "a", "kind": "select", "label": "a", "choices": [{"value": "x", "label": "X"}, {"value": "x", "label": "Y"}]}),
        json!({"id": "a", "kind": "text", "label": "a", "choices": [{"value": "x", "label": "X"}]}),
        json!({"id": "a", "kind": "datetime", "label": "a", "default": -1}),
        json!({"id": "a", "kind": "options", "label": "a", "max": 51}),
        json!({"id": "a", "kind": "options", "label": "a", "min": 3, "max": 2}),
        json!({"id": "a", "kind": "options", "label": "a", "min": 1.5}),
        json!({"id": "a", "kind": "members", "label": "a", "max": 501}),
        json!({"id": "a", "kind": "members", "label": "a", "default": ["u1"]}),
        json!({"id": "a", "kind": "text", "label": "a", "defaultPath": "a"}),
    ] {
        assert!(!check(bad.clone()), "{bad} must be rejected");
    }
    assert!(
        serde_json::from_value::<UiDocument>(
            json!({"title": "t", "inputs": [{"id": "a", "kind": "file", "label": "a"}]})
        )
        .is_err()
    );
    let duplicate = doc(json!({"title": "t", "inputs": [
        {"id": "a", "kind": "text", "label": "a"}, {"id": "a", "kind": "toggle", "label": "b"}
    ]}));
    assert!(duplicate.validate().is_err());
}

#[test]
fn form_values_are_validated_per_kind_with_defaults() {
    let create = contribution("example.poll.create");
    let action = &create.schema.actions[0];
    let ok = create
        .schema
        .validate_form(
            action,
            &json!({"question": "午饭？", "options": ["面", " 饭 "], "deadlineMs": 1700000000000_i64}),
        )
        .unwrap();
    assert_eq!(
        Value::Object(ok.clone()),
        json!({"question": "午饭？", "options": ["面", " 饭 "], "multiple": false, "deadlineMs": 1700000000000_i64})
    );
    assert_eq!(
        bind_action_input_with(action, &json!({}), ok).unwrap()["multiple"],
        json!(false)
    );
    let issues = |values: Value| -> Vec<(String, UiFormIssue)> {
        create
            .schema
            .validate_form(action, &values)
            .unwrap_err()
            .into_iter()
            .map(|e| (e.input, e.issue))
            .collect()
    };
    use UiFormIssue::*;
    // The default ["", ""] is a placeholder, not an answer.
    assert_eq!(
        issues(json!({})),
        [
            ("question".into(), Required),
            ("options".into(), InvalidItem)
        ]
    );
    assert_eq!(
        issues(json!({"question": "   ", "options": ["a"], "extra": 1})),
        [
            ("extra".into(), Unknown),
            ("question".into(), Required),
            ("options".into(), TooFewItems)
        ]
    );
    assert_eq!(
        issues(
            json!({"question": "x".repeat(201), "options": ["a", "A ", "a"], "multiple": "yes", "deadlineMs": -5})
        ),
        [
            ("question".into(), TooLong),
            ("options".into(), DuplicateItem),
            ("multiple".into(), InvalidType),
            ("deadlineMs".into(), OutOfRange)
        ]
    );
    assert_eq!(
        issues(
            json!({"question": "q", "options": vec!["o"; 21].iter().enumerate().map(|(i, _)| format!("o{i}")).collect::<Vec<_>>()})
        ),
        [("options".into(), TooManyItems)]
    );
    assert_eq!(
        issues(json!({"question": "a\u{0}b", "options": ["a", "b"], "deadlineMs": 1.5})),
        [
            ("question".into(), InvalidType),
            ("deadlineMs".into(), InvalidType)
        ]
    );
    assert!(create.schema.validate_form(action, &json!([])).is_err());

    // The remaining kinds.
    let document = doc(json!({"title": "t", "inputs": [
        {"id": "body", "kind": "textarea", "label": "b", "maxLength": 5},
        {"id": "n", "kind": "number", "label": "n", "min": 0, "max": 10},
        {"id": "pick", "kind": "select", "label": "p", "required": true, "choices": [{"value": "x", "label": "X"}]},
        {"id": "who", "kind": "members", "label": "w", "min": 1, "max": 2}
    ], "actions": [{"id": "go", "label": "go", "target": op("go"), "form": ["body", "n", "pick", "who"]}]}));
    document.validate().unwrap();
    let go = &document.actions[0];
    assert_eq!(
        Value::Object(
            document
                .validate_form(
                    go,
                    &json!({"body": "a\nb", "n": 2.5, "pick": "x", "who": ["u1", "u2"]})
                )
                .unwrap()
        ),
        json!({"body": "a\nb", "n": 2.5, "pick": "x", "who": ["u1", "u2"]})
    );
    let errors: Vec<_> = document
        .validate_form(
            go,
            &json!({"body": "123456", "n": 11, "pick": "y", "who": ["u1", "u1"]}),
        )
        .unwrap_err()
        .into_iter()
        .map(|e| e.issue)
        .collect();
    assert_eq!(errors, [TooLong, OutOfRange, NotAChoice, DuplicateItem]);
    let errors: Vec<_> = document
        .validate_form(go, &json!({"who": ["u1", "u2", "u3"]}))
        .unwrap_err()
        .into_iter()
        .map(|e| e.issue)
        .collect();
    assert_eq!(errors, [Required, TooManyItems]);
    let errors: Vec<_> = document
        .validate_form(go, &json!({"pick": "x", "who": ["bad id"]}))
        .unwrap_err()
        .into_iter()
        .map(|e| e.issue)
        .collect();
    assert_eq!(errors, [InvalidItem]);
    assert_eq!(
        serde_json::to_value(UiFormError {
            input: "who".into(),
            issue: TooManyItems
        })
        .unwrap(),
        json!({"input": "who", "issue": "tooManyItems"})
    );
}

#[test]
fn resolved_inputs_translate_and_take_defaults_from_data() {
    let document = doc(json!({"title": "接龙", "inputs": [
        {"id": "line", "kind": "text", "label": "内容", "placeholder": "写点什么", "defaultPath": "/myLine", "default": ""},
        {"id": "who", "kind": "members", "label": "负责人", "defaultPath": "/assignees"},
        {"id": "pick", "kind": "select", "label": "类型", "choices": [{"value": "a", "label": "甲"}]}
    ], "translations": {"en-US": {"title": "Solitaire", "fields": [], "actions": {},
        "texts": {"内容": "Entry", "写点什么": "Write something", "甲": "A"}}}}));
    let contribution: UiContribution = serde_json::from_value(json!({
        "kind": "composer-action", "id": "x.create", "schemaVersion": 1, "requiredPermissions": [],
        "schema": document
    }))
    .unwrap();
    let view = contribution.resolve(
        &json!({"myLine": "两箱", "assignees": ["u1", "u2"]}),
        "en-US",
        "u1",
    );
    let raw = serde_json::to_value(&view.inputs).unwrap();
    assert_eq!(
        raw,
        json!([
            {"id": "line", "kind": "text", "label": "Entry", "placeholder": "Write something", "default": "两箱"},
            {"id": "who", "kind": "members", "label": "负责人", "default": ["u1", "u2"]},
            {"id": "pick", "kind": "select", "label": "类型", "choices": [{"value": "a", "label": "A"}]}
        ])
    );
    // A wrong-typed value at defaultPath keeps the declared default.
    let view = contribution.resolve(&json!({"myLine": 7, "assignees": ["bad id"]}), "", "u1");
    assert_eq!(view.inputs[0].default, Some(json!("")));
    assert_eq!(view.inputs[1].default, None);
}

#[test]
fn action_forms_confirm_style_and_exists_conditions() {
    let base = |action: Value| {
        doc(
            json!({"title": "t", "inputs": [{"id": "reason", "kind": "text", "label": "原因"}],
            "actions": [action]}),
        )
    };
    assert!(base(json!({"id": "a", "label": "a", "target": op("a"), "form": ["reason"], "confirm": "确定？", "style": "danger"})).validate().is_ok());
    for bad in [
        json!({"id": "a", "label": "a", "target": op("a"), "form": ["missing"]}),
        json!({"id": "a", "label": "a", "target": op("a"), "form": ["reason", "reason"]}),
        json!({"id": "a", "label": "a", "target": op("a"), "form": ["reason"], "input": {"reason": "/r"}}),
        json!({"id": "a", "label": "a", "target": {"kind": "page", "page": "x.p"}, "form": ["reason"]}),
        json!({"id": "a", "label": "a", "target": op("a"), "confirm": ""}),
        json!({"id": "a", "label": "a", "target": op("a"), "hiddenWhen": [{"path": "/x", "exists": true, "viewer": true}]}),
    ] {
        assert!(base(bad.clone()).validate().is_err(), "{bad}");
    }
    assert!(
        serde_json::from_value::<UiAction>(
            json!({"id": "a", "label": "a", "target": op("a"), "style": "ghost"})
        )
        .is_err()
    );
    // `exists` hides on presence; negated, it hides on absence (even missing).
    let document = doc(json!({"title": "t", "actions": [
        {"id": "join", "label": "参与", "target": op("join"), "hiddenWhen": [{"path": "/myLine", "exists": true}]},
        {"id": "edit", "label": "修改", "target": op("edit"), "hiddenWhen": [{"path": "/myLine", "exists": true, "negate": true}]}
    ]}));
    let ids = |data: Value| {
        resolve(document.clone(), data)
            .actions
            .into_iter()
            .map(|a| a.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(json!({})), ["join"]);
    assert_eq!(ids(json!({"myLine": ""})), ["join"]);
    assert_eq!(ids(json!({"myLine": []})), ["join"]);
    assert_eq!(ids(json!({"myLine": "两箱"})), ["edit"]);
    assert_eq!(ids(json!({"myLine": 0})), ["edit"]);
    // Collected values never replace declared input.
    let action: UiAction = serde_json::from_value(
        json!({"id": "a", "label": "a", "target": op("a"), "input": {"id": "/id"}}),
    )
    .unwrap();
    let payload = json!({"id": "p"});
    for extra in [json!({"id": "forged"}), json!({"bad-name": 1})] {
        assert!(
            bind_action_input_with(&action, &payload, extra.as_object().unwrap().clone()).is_err()
        );
    }
    assert!(
        bind_action_input_with(
            &action,
            &payload,
            [("big".to_string(), json!("x".repeat(16384)))]
                .into_iter()
                .collect()
        )
        .is_err()
    );
}

#[test]
fn document_bounds_and_translated_texts() {
    let actions: Vec<Value> = (0..17)
        .map(|i| json!({"id": format!("a{i}"), "label": "a", "target": op("a")}))
        .collect();
    assert!(
        doc(json!({"title": "t", "actions": actions[..16]}))
            .validate()
            .is_ok()
    );
    assert!(
        doc(json!({"title": "t", "actions": actions}))
            .validate()
            .is_err()
    );
    let blocks = vec![json!({"kind": "divider"}); 25];
    assert!(
        doc(json!({"title": "t", "blocks": blocks[..24]}))
            .validate()
            .is_ok()
    );
    assert!(
        doc(json!({"title": "t", "blocks": blocks}))
            .validate()
            .is_err()
    );
    let inputs: Vec<Value> = (0..33)
        .map(|i| json!({"id": format!("i{i}"), "kind": "toggle", "label": "i"}))
        .collect();
    assert!(
        doc(json!({"title": "t", "inputs": inputs[..32]}))
            .validate()
            .is_ok()
    );
    assert!(
        doc(json!({"title": "t", "inputs": inputs}))
            .validate()
            .is_err()
    );
    let texts = |n: usize| -> Value {
        (0..n)
            .map(|i| (format!("k{i}"), json!("v")))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    let with_texts = |texts: Value| {
        doc(
            json!({"title": "t", "translations": {"en": {"title": "t", "fields": [], "actions": {}, "texts": texts}}}),
        )
    };
    assert!(with_texts(texts(256)).validate().is_ok());
    assert!(with_texts(texts(257)).validate().is_err());
    assert!(with_texts(json!({"k": ""})).validate().is_err());
    assert!(with_texts(json!({"k": "a\tb"})).validate().is_err());
    // Unused texts are harmless: trimming a document never invalidates it.
    assert!(with_texts(json!({"unused": "x"})).validate().is_ok());
}

#[test]
fn detail_is_declared_bound_and_resolved_through_the_catalog() {
    let catalog = catalog();
    let detail = catalog
        .detail(
            "201",
            "example.poll",
            "example.poll.card",
            ClientPlatform::Web,
            true,
            1000,
        )
        .unwrap();
    assert_eq!(detail.operation, "example.poll.details");
    assert_eq!(
        detail
            .bind_input(&json!({"pollId": "p-1", "x": 1}))
            .unwrap(),
        json!({"pollId": "p-1"})
    );
    assert!(detail.bind_input(&json!({})).is_err());
    for (tenant, gesture, now) in [
        ("202", true, 1000),
        ("201", false, 1000),
        ("201", true, 10000),
    ] {
        assert!(
            catalog
                .detail(
                    tenant,
                    "example.poll",
                    "example.poll.card",
                    ClientPlatform::Web,
                    gesture,
                    now
                )
                .is_err()
        );
    }
    assert!(
        catalog
            .detail(
                "201",
                "example.poll",
                "example.poll.create",
                ClientPlatform::Web,
                true,
                1000
            )
            .is_err()
    );
    let mut trimmed = catalog.clone();
    trimmed.plugins[0].operations.remove("example.poll.details");
    assert!(
        trimmed
            .detail(
                "201",
                "example.poll",
                "example.poll.card",
                ClientPlatform::Web,
                true,
                1000
            )
            .is_err()
    );
    let view = catalog.resolve_detail(
        &card(json!({"pollId": "p-1"})),
        &json!({"voters": []}),
        context("en-US", "u1"),
    );
    assert_eq!(view.title, "Poll details");
    let ResolvedUiBlock::Entries(entries) = only_block(&view) else {
        panic!()
    };
    assert_eq!(entries.empty_text.as_deref(), Some("No votes yet"));
    let foreign = catalog.resolve_detail(
        &card(json!({"pollId": "p-1"})),
        &json!({}),
        UiRenderContext {
            tenant: "202",
            ..context("", "u1")
        },
    );
    assert!(!foreign.available);
}

#[test]
fn audience_admits_by_conversation_type_and_role() {
    use UiConversationType::*;
    use UiGroupRole::*;
    let everyone = UiAudience::default();
    assert!(everyone.admits(Single, None) && everyone.admits(Group, None));
    let groups = contribution("example.poll.create").audience.unwrap();
    assert!(groups.admits(Group, None));
    assert!(!groups.admits(Single, Some(Owner)));
    let admins = UiAudience {
        conversation_types: vec![Group],
        group_roles: vec![Owner, Admin],
    };
    admins.validate().unwrap();
    assert!(admins.admits(Group, Some(Admin)));
    assert!(!admins.admits(Group, Some(Member)));
    assert!(
        !admins.admits(Group, None),
        "an unknown role hides a role-restricted entry"
    );
    let roles_anywhere = UiAudience {
        conversation_types: vec![],
        group_roles: vec![Owner],
    };
    assert!(
        roles_anywhere.admits(Single, None),
        "roles apply in groups only"
    );
    assert_eq!(
        serde_json::to_value(&admins).unwrap(),
        json!({"conversationTypes": ["group"], "groupRoles": ["owner", "admin"]})
    );
}

#[test]
fn retain_targets_drops_actions_and_references_but_keeps_documents_valid() {
    let mut card = contribution("example.poll.card");
    card.retain_targets(|target| {
        !matches!(target, UiActionTarget::Operation { operation } if operation == "example.poll.vote")
    });
    card.schema.validate().unwrap();
    let ids: Vec<_> = card.schema.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["close"]);
    assert!(card.schema.choices_for("vote").is_none());
    assert!(
        card.schema.translations["en-US"]
            .actions
            .keys()
            .all(|id| id == "close")
    );
    assert!(card.detail.is_some());
    card.retain_targets(|target| {
        !matches!(target, UiActionTarget::Operation { operation } if operation == "example.poll.details")
    });
    assert!(card.detail.is_none());

    let mut admin = admin_page();
    admin.retain_targets(|target| {
        !matches!(target, UiActionTarget::Operation { operation } if operation == "example.poll.remove")
    });
    admin.document.validate().unwrap();
    assert_eq!(admin.document.rows().unwrap().actions, ["close"]);
    assert!(admin.detail.is_some());
    admin.retain_targets(|target| {
        !matches!(target, UiActionTarget::Operation { operation } if operation == "example.poll.inspect")
    });
    assert!(
        admin.detail.is_none(),
        "an unapproved detail operation drops the detail"
    );
    admin.retain_targets(|target| {
        !matches!(target, UiActionTarget::Operation { operation } if operation == "example.poll.list")
    });
    assert!(admin.document.rows().is_none());
    admin.document.validate().unwrap();
}

#[test]
fn schema_export_describes_the_v2_contract() {
    let schema = serde_json::to_string(&schemars::schema_for!(PluginManifest)).unwrap();
    for needle in [
        "AdminPage",
        "entityPath",
        "forwardable",
        "audience",
        "UiDetail",
        "UiBlock",
        "UiInput",
        "closedWhen",
        "im.plugin.entity_state.publish",
        "texts",
    ] {
        assert!(schema.contains(needle), "{needle} missing from the schema");
    }
}

// ---- hiddenWhen on blocks and on the detail; string entries ----

const CARD_KINDS: [&str; 10] = [
    "text", "fields", "choices", "entries", "people", "status", "progress", "time", "divider",
    "note",
];
/// One block of every card kind, with data for each of them.
fn every_card_block() -> (Value, Value) {
    let document = json!({"title": "t",
    "actions": [{"id": "vote", "label": "投", "target": op("vote")}],
    "blocks": [
        {"kind": "text", "text": "说明"},
        {"kind": "fields", "fields": [{"label": "地点", "path": "/place"}]},
        {"kind": "choices", "optionsPath": "/options", "action": "vote"},
        {"kind": "entries", "itemsPath": "/lines"},
        {"kind": "people", "groups": [{"label": "已确认", "usersPath": "/users"}]},
        {"kind": "status", "path": "/state"},
        {"kind": "progress", "valuePath": "/done", "totalPath": "/all"},
        {"kind": "time", "path": "/start"},
        {"kind": "divider"},
        {"kind": "note", "text": "匿名投票"}
    ]});
    let data = json!({"place": "3F", "options": ["A", "B"], "lines": ["a"], "users": ["u1"],
        "state": "open", "done": 1, "all": 2, "start": 1700000000000_i64});
    (document, data)
}
fn kinds(view: &ResolvedPluginView) -> Vec<String> {
    view.blocks
        .iter()
        .map(|b| {
            serde_json::to_value(b).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn every_block_kind_is_hidden_by_its_conditions() {
    let (document, data) = every_card_block();
    assert_eq!(
        kinds(&resolve(doc(document.clone()), data.clone())),
        CARD_KINDS
    );
    let mut hidden = data.clone();
    hidden["hide"] = json!(true);
    for index in 0..CARD_KINDS.len() {
        let mut one = document.clone();
        one["blocks"][index]["hiddenWhen"] = json!([{"path": "/hide", "anyOf": ["true"]}]);
        // Round trip keeps the conditions; other blocks serialize none.
        let parsed = doc(one.clone());
        let raw = serde_json::to_value(&parsed).unwrap();
        assert_eq!(raw["blocks"][index], one["blocks"][index]);
        assert_eq!(parsed.blocks[index].hidden_when().len(), 1);
        for (other, block) in raw["blocks"].as_array().unwrap().iter().enumerate() {
            assert_eq!(block.get("hiddenWhen").is_some(), other == index);
        }
        // The condition does not match: the block shows.
        assert_eq!(kinds(&resolve(parsed.clone(), data.clone())), CARD_KINDS);
        // It matches: that block alone is gone.
        let view = resolve(parsed, hidden.clone());
        let expected: Vec<_> = CARD_KINDS
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, kind)| kind.to_string())
            .collect();
        assert_eq!(kinds(&view), expected, "{}", CARD_KINDS[index]);
        // A hidden choices block takes its action along: never a card button.
        assert!(view.actions.is_empty());
    }
    // Every condition form applies: viewer, exists and its negation.
    let note = |conditions: Value| {
        doc(json!({"title": "t", "blocks": [
            {"kind": "note", "text": "仅发起人", "hiddenWhen": conditions}
        ]}))
    };
    let shown = |conditions: Value, data: Value| !resolve(note(conditions), data).blocks.is_empty();
    let creator = json!([{"path": "/creatorId", "viewer": true, "negate": true}]);
    assert!(shown(creator.clone(), json!({"creatorId": "u1"})));
    assert!(!shown(creator.clone(), json!({"creatorId": "u2"})));
    // A missing value never matches anyOf or viewer, even negated.
    assert!(shown(creator, json!({})));
    let present = json!([{"path": "/entries", "exists": true}]);
    assert!(!shown(present.clone(), json!({"entries": ["a"]})));
    assert!(shown(present.clone(), json!({"entries": []})));
    assert!(shown(present, json!({})));
    let absent = json!([{"path": "/entries", "exists": true, "negate": true}]);
    assert!(shown(absent.clone(), json!({"entries": ["a"]})));
    assert!(!shown(absent, json!({})));
    // Any one of several conditions hides the block.
    let either = json!([
        {"path": "/a", "anyOf": ["x"]},
        {"path": "/b", "anyOf": ["y"]}
    ]);
    assert!(shown(either.clone(), json!({"a": "z", "b": "z"})));
    assert!(!shown(either.clone(), json!({"a": "z", "b": "y"})));
    assert!(!shown(either, json!({"a": "x"})));
    // Bounded and valid like action conditions, on every kind.
    let five = json!(vec![json!({"path": "/a", "anyOf": ["x"]}); 5]);
    for bad in [
        five,
        json!([{"path": "/a"}]),
        json!([{"path": "/a", "anyOf": ["x"], "exists": true}]),
        json!([{"path": "a", "anyOf": ["x"]}]),
        json!([{"path": "/a", "anyOf": [""]}]),
    ] {
        for index in 0..CARD_KINDS.len() {
            let mut one = document.clone();
            one["blocks"][index]["hiddenWhen"] = bad.clone();
            assert!(
                doc(one).validate().is_err(),
                "{} accepted {bad}",
                CARD_KINDS[index]
            );
        }
    }
    let mut four = document.clone();
    for block in four["blocks"].as_array_mut().unwrap() {
        block["hiddenWhen"] = json!(vec![json!({"path": "/a", "anyOf": ["x"]}); 4]);
    }
    doc(four).validate().unwrap();
}

#[test]
fn block_conditions_read_the_merged_card_data() {
    let card: UiContribution = serde_json::from_value(json!({
        "kind": "message-content", "id": "x.card", "schemaVersion": 1,
        "requiredPermissions": [], "entityPath": "/id",
        "schema": {"title": "t", "blocks": [
            {"kind": "note", "text": "已结束",
             "hiddenWhen": [{"path": "/closed", "anyOf": ["true"], "negate": true}]},
            {"kind": "note", "text": "你还没有参与",
             "hiddenWhen": [{"path": "/mine", "exists": true}]},
            {"kind": "note", "text": "匿名", "hiddenWhen": [
                {"path": "/anonymous", "anyOf": ["true"], "negate": true},
                {"path": "/anonymous", "exists": true, "negate": true}
            ]}
        ]}
    }))
    .unwrap();
    card.schema.validate().unwrap();
    let notes = |payload: Value, layers: UiStateLayers<'_>| -> Vec<String> {
        card.resolve(&layers.merge(&card, &payload), "", "u1")
            .blocks
            .iter()
            .map(|b| match b {
                ResolvedUiBlock::Note(note) => note.text.clone(),
                other => panic!("{other:?}"),
            })
            .collect()
    };
    let payload = json!({"id": "e1", "closed": false, "anonymous": true});
    assert_eq!(
        notes(payload.clone(), UiStateLayers::default()),
        ["你还没有参与", "匿名"]
    );
    // Shared state closes it; the viewer's own state says they took part.
    let shared = SharedState {
        entity: "e1".into(),
        version: 2,
        values: json!({"closed": true}).as_object().unwrap().clone(),
    };
    let local = PluginMessageState {
        version: 2,
        values: json!({"mine": [0]}).as_object().unwrap().clone(),
        based_on_version: None,
    };
    assert_eq!(
        notes(
            payload.clone(),
            UiStateLayers {
                shared: Some(&shared),
                local: None
            }
        ),
        ["已结束", "你还没有参与", "匿名"]
    );
    assert_eq!(
        notes(
            payload.clone(),
            UiStateLayers {
                shared: Some(&shared),
                local: Some(&local)
            }
        ),
        ["已结束", "匿名"]
    );
    // Shared state of another entity does not apply.
    let foreign = SharedState {
        entity: "e2".into(),
        ..shared.clone()
    };
    assert_eq!(
        notes(
            payload,
            UiStateLayers {
                shared: Some(&foreign),
                local: None
            }
        ),
        ["你还没有参与", "匿名"]
    );
    // "Not true" takes both conditions: false and missing both hide.
    for anonymous in [json!(false), json!("false"), Value::Null] {
        let payload = json!({"id": "e1", "closed": false, "mine": [1], "anonymous": anonymous});
        assert!(
            notes(payload, UiStateLayers::default()).is_empty(),
            "{anonymous}"
        );
    }
    assert!(
        notes(
            json!({"id": "e1", "closed": false, "mine": [1]}),
            UiStateLayers::default()
        )
        .is_empty()
    );
}

#[test]
fn rows_block_conditions_read_the_page_not_a_row() {
    let manifest = edited(|raw| {
        raw["admin"]["pages"][0]["document"]["blocks"][0]["hiddenWhen"] = json!([
            {"path": "/status", "anyOf": ["open"]},
            {"path": "/suspended", "anyOf": ["true"]}
        ])
    })
    .unwrap();
    let admin = manifest.admin.as_ref().unwrap().page("polls").unwrap();
    let page = json!({"items": [{"pollId": "p1", "status": "open"}]});
    let view = admin.resolve(&page, "", "s");
    let ResolvedUiBlock::Rows(rows) = only_block(&view) else {
        panic!()
    };
    assert_eq!(rows.rows.len(), 1, "a row's status does not hide the block");
    let mut suspended = page.clone();
    suspended["suspended"] = json!(true);
    assert!(admin.resolve(&suspended, "", "s").blocks.is_empty());
    // The page-level status hides it too.
    let mut open = page;
    open["status"] = json!("open");
    assert!(admin.resolve(&open, "", "s").blocks.is_empty());
}

#[test]
fn detail_hidden_when_removes_view_all_more_and_row_detail_input() {
    let with_detail = |hidden_when: Value| -> UiContribution {
        serde_json::from_value(json!({
            "kind": "message-content", "id": "x.card", "schemaVersion": 1,
            "requiredPermissions": [],
            "detail": {"operation": "x.op.details", "input": {"id": "/id"},
                "hiddenWhen": hidden_when,
                "document": {"title": "详情", "blocks": [{"kind": "entries", "itemsPath": "/all"}]}},
            "schema": {"title": "t", "blocks": [
                {"kind": "entries", "itemsPath": "/lines", "limit": 2}
            ]}
        }))
        .unwrap()
    };
    let anonymous = json!([{"path": "/anonymous", "anyOf": ["true"]}]);
    let entries = |view: &ResolvedPluginView| match only_block(view) {
        ResolvedUiBlock::Entries(e) => (e.items.len(), e.total, e.more),
        other => panic!("{other:?}"),
    };
    let lines = json!(["a", "b", "c"]);
    let public = with_detail(anonymous.clone());
    public.detail.as_ref().unwrap().validate().unwrap();
    let view = public.resolve(
        &json!({"id": "p", "lines": lines, "anonymous": false}),
        "",
        "u1",
    );
    assert!(view.detail);
    assert_eq!(entries(&view), (2, 3, true));
    let view = public.resolve(
        &json!({"id": "p", "lines": lines, "anonymous": true}),
        "",
        "u1",
    );
    assert!(!view.detail, "no view-all entry");
    assert_eq!(entries(&view), (2, 3, false), "more follows the detail");
    assert!(
        serde_json::to_value(&view).unwrap().get("detail").is_none(),
        "a hidden detail serializes like none"
    );
    // The detail document itself still resolves (the plugin decides access);
    // it has no detail of its own, so its entries never offer more.
    let all: Vec<String> = (0..12).map(|i| format!("第 {i} 条")).collect();
    let inside = public
        .resolve_detail(&json!({"all": all}), "", "u1")
        .unwrap();
    assert_eq!(entries(&inside), (10, 12, false));
    // Without a detail there is nothing to open either.
    let mut plain = public.clone();
    plain.detail = None;
    for anonymous in [false, true] {
        let view = plain.resolve(&json!({"lines": lines, "anonymous": anonymous}), "", "u1");
        assert_eq!(entries(&view), (2, 3, false));
        assert!(!view.detail);
    }
    // A detail the host may not open (trimmed by retain_targets) is none.
    let mut trimmed = public.clone();
    trimmed.retain_targets(|target| {
        !matches!(target, UiActionTarget::Operation { operation } if operation == "x.op.details")
    });
    let view = trimmed.resolve(&json!({"lines": lines}), "", "u1");
    assert_eq!(entries(&view), (2, 3, false));
    // Bounded and valid.
    for bad in [
        json!(vec![json!({"path": "/a", "anyOf": ["x"]}); 5]),
        json!([{"path": "/a"}]),
        json!([{"path": "a", "exists": true}]),
    ] {
        assert!(
            with_detail(bad.clone()).detail.unwrap().validate().is_err(),
            "{bad}"
        );
        let manifest = edited(|raw| {
            raw["clientExtensions"]["contributions"][0]["detail"]["hiddenWhen"] = bad.clone()
        });
        assert!(manifest.is_err(), "{bad}");
    }
    // Through the catalog: the poll card hides its detail on anonymous polls.
    let manifest = edited(|raw| {
        raw["clientExtensions"]["contributions"][0]["detail"]["hiddenWhen"] = anonymous.clone()
    })
    .unwrap();
    let mut catalog = catalog();
    catalog.plugins[0].extensions = manifest.client_extensions.clone();
    let options = json!(["面", "饭"]);
    let view = catalog.resolve_with_context(
        &card(
            json!({"pollId": "p-1", "question": "午饭？", "options": options, "anonymous": true}),
        ),
        context("", "u1"),
    );
    assert!(view.available && !view.detail);
    let view = catalog.resolve_with_context(
        &card(json!({"pollId": "p-1", "question": "午饭？", "options": options})),
        context("", "u1"),
    );
    assert!(view.available && view.detail);
    // Admin page: conditions read the page; a hidden detail binds no row.
    let manifest = edited(|raw| {
        raw["admin"]["pages"][0]["detail"]["hiddenWhen"] =
            json!([{"path": "/archived", "anyOf": ["true"]}])
    })
    .unwrap();
    let admin = manifest.admin.as_ref().unwrap().page("polls").unwrap();
    let row_detail = |page: Value| {
        let view = admin.resolve(&page, "", "s");
        let ResolvedUiBlock::Rows(rows) = only_block(&view) else {
            panic!()
        };
        (view.detail, rows.rows[0].detail_input.clone())
    };
    assert_eq!(
        row_detail(json!({"items": [{"pollId": "p1", "archived": true}]})),
        (true, Some(json!({"pollId": "p1"})))
    );
    assert_eq!(
        row_detail(json!({"items": [{"pollId": "p1"}], "archived": true})),
        (false, None)
    );
}

#[test]
fn entries_take_strings_as_text_without_text_path() {
    let card: UiContribution = serde_json::from_value(json!({
        "kind": "message-content", "id": "x.card", "schemaVersion": 1,
        "requiredPermissions": [], "entityPath": "/id",
        "detail": {"operation": "x.op.lines", "input": {"id": "/id"},
            "document": {"title": "全部接龙", "blocks": [{"kind": "entries", "itemsPath": "/lines"}]}},
        "schema": {"title": "接龙", "blocks": [
            {"kind": "entries", "itemsPath": "/recent", "countPath": "/count",
             "emptyText": "还没有人接龙"}
        ]}
    }))
    .unwrap();
    card.schema.validate().unwrap();
    // The latest lines live in shared state as a plain string array.
    let shared = SharedState {
        entity: "s1".into(),
        version: 3,
        values: json!({"recent": ["Alice 两箱", "Bob 一箱"], "count": 12})
            .as_object()
            .unwrap()
            .clone(),
    };
    shared.validate().unwrap();
    let payload = json!({"id": "s1", "recent": [], "count": 0});
    let layers = UiStateLayers {
        shared: Some(&shared),
        local: None,
    };
    let view = card.resolve(&layers.merge(&card, &payload), "", "u1");
    assert_eq!(
        serde_json::to_value(only_block(&view)).unwrap(),
        json!({"kind": "entries", "items": [
            {"index": 1, "text": "Alice 两箱"}, {"index": 2, "text": "Bob 一箱"}
        ], "numbered": true, "total": 12, "more": true, "emptyText": "还没有人接龙"})
    );
    // As sent, before any line.
    let view = card.resolve(&payload, "", "u1");
    assert_eq!(
        serde_json::to_value(only_block(&view)).unwrap(),
        json!({"kind": "entries", "items": [], "numbered": true, "total": 0, "more": false,
               "emptyText": "还没有人接龙"})
    );
    // Strings and objects side by side: textPath decides which form is read.
    let mixed = json!({"lines": ["一", {"text": "二", "user": "u2"}, "  ", "四"]});
    let texts = |block: Value| -> Vec<(u32, String)> {
        let view = resolve(doc(json!({"title": "t", "blocks": [block]})), mixed.clone());
        match only_block(&view) {
            ResolvedUiBlock::Entries(e) => e
                .items
                .iter()
                .map(|item| (item.index, item.text.clone()))
                .collect(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        texts(json!({"kind": "entries", "itemsPath": "/lines"})),
        [(1, "一".to_owned()), (4, "四".to_owned())]
    );
    assert_eq!(
        texts(json!({"kind": "entries", "itemsPath": "/lines", "userPath": "/user"})),
        [
            (1, "一".to_owned()),
            (2, String::new()),
            (4, "四".to_owned())
        ]
    );
    assert_eq!(
        texts(json!({"kind": "entries", "itemsPath": "/lines", "textPath": "/text"})),
        [(2, "二".to_owned())]
    );
}

#[test]
fn manifests_without_display_conditions_parse_and_serialize_as_before() {
    for (name, raw) in [
        (
            "red packet 0.4.1",
            &include_bytes!("fixtures/red-packet-0.4.1.json")[..],
        ),
        (
            "ui v2 fixture",
            &include_bytes!("fixtures/plugin-ui-v2-manifest.json")[..],
        ),
    ] {
        let manifest = PluginManifest::from_json(raw).unwrap();
        let reserialized = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(
            PluginManifest::from_json(&reserialized).unwrap(),
            manifest,
            "{name}"
        );
        // Only the action conditions that were declared come back out.
        let conditions = |bytes: &[u8]| {
            String::from_utf8_lossy(bytes)
                .matches("\"hiddenWhen\"")
                .count()
        };
        assert_eq!(conditions(&reserialized), conditions(raw), "{name}");
        for contribution in &manifest.client_extensions.contributions {
            let documents = std::iter::once(&contribution.schema)
                .chain(contribution.detail.as_ref().map(|d| &d.document));
            for document in documents {
                assert!(document.blocks.iter().all(|b| b.hidden_when().is_empty()));
            }
            assert!(
                contribution
                    .detail
                    .as_ref()
                    .is_none_or(|d| d.hidden_when.is_empty())
            );
        }
    }
    // The exported schema describes the conditions on every block kind (each
    // inlined in UiBlock) and on the detail.
    let schema = serde_json::to_value(schemars::schema_for!(PluginManifest)).unwrap();
    let variants = schema["$defs"]["UiBlock"]["oneOf"].as_array().unwrap();
    // Every card kind, plus the page-only `rows` and `protected-media`.
    assert_eq!(variants.len(), CARD_KINDS.len() + 2);
    for variant in variants {
        assert!(
            variant["properties"]["hiddenWhen"].is_object(),
            "{} lacks hiddenWhen",
            variant["properties"]["kind"]
        );
    }
    assert!(schema["$defs"]["UiDetail"]["properties"]["hiddenWhen"].is_object());
}
