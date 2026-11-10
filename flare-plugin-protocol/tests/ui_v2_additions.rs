//! UiDocument v2 additions: admin page filters, static choices options,
//! composer data and `maxPath`, `allOf` conditions, entries `userIdsPath` and
//! detail paging. Every addition is optional; documents without them resolve
//! as before.
use flare_plugin_protocol::{
    PluginManifest,
    manifest::{ClientPlatform, UiContribution, UiContributionKind},
    ui::*,
};
use serde_json::{Map, Value, json};

fn doc(raw: Value) -> UiDocument {
    serde_json::from_value(raw).unwrap()
}
fn contribution(kind: &str, schema: Value, detail: Option<Value>) -> UiContribution {
    let mut raw = json!({
        "kind": kind, "id": "x.thing", "schemaVersion": 1,
        "requiredPermissions": [], "schema": schema
    });
    if let Some(detail) = detail {
        raw["detail"] = detail;
    }
    serde_json::from_value(raw).unwrap()
}
/// Resolve a standalone message-content document against `data` for `u1`.
fn resolve(document: Value, data: Value) -> ResolvedPluginView {
    let contribution = contribution("message-content", document, None);
    contribution.schema.validate().unwrap();
    contribution.resolve(&data, "en-US", "u1")
}
fn op(id: &str) -> Value {
    json!({"kind": "operation", "operation": format!("x.op.{id}")})
}
fn only_block(view: &ResolvedPluginView) -> &ResolvedUiBlock {
    assert_eq!(view.blocks.len(), 1, "{view:#?}");
    &view.blocks[0]
}
fn invalid(document: Value) -> bool {
    serde_json::from_value::<UiDocument>(document).map_or(true, |d| d.validate().is_err())
}
fn issues(result: Result<Map<String, Value>, Vec<UiFormError>>) -> Vec<(String, UiFormIssue)> {
    result
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.input, e.issue))
        .collect()
}

// ---- admin page filters ----

/// A console page (`admin.pages`) over `document`.
fn admin_page(document: Value) -> flare_plugin_protocol::admin::AdminPage {
    serde_json::from_value(json!({
        "id": "things", "kind": "list", "permission": "read", "document": document
    }))
    .unwrap()
}

fn admin_document(filters: Option<Value>) -> Value {
    let mut rows = json!({
        "kind": "rows", "source": "x.op.list", "path": "/items", "keyPath": "/id",
        "columns": [{"label": "名称", "path": "/name"}],
        "actions": ["rename"]
    });
    if let Some(filters) = filters {
        rows["filters"] = filters;
    }
    json!({
        "title": "管理",
        "inputs": [
            {"id": "status", "kind": "select", "label": "状态",
             "choices": [{"value": "open", "label": "进行中"}, {"value": "closed", "label": "已结束"}]},
            {"id": "keyword", "kind": "text", "label": "关键字"},
            {"id": "name", "kind": "text", "label": "新名称", "required": true}
        ],
        "blocks": [rows],
        "actions": [{"id": "rename", "label": "改名", "target": op("rename"),
                     "input": {"id": "/id"}, "form": ["name"]}]
    })
}

#[test]
fn admin_page_filters_are_the_listed_inputs_only() {
    let document = doc(admin_document(Some(json!(["status", "keyword"]))));
    document.validate().unwrap();
    assert_eq!(
        document
            .filter_inputs()
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["status", "keyword"]
    );
    // The required form input of the row action no longer blocks filtering.
    assert_eq!(
        document.validate_inputs(&json!({})).unwrap(),
        Map::new(),
        "no filter is required"
    );
    assert_eq!(
        Value::Object(
            document
                .validate_inputs(&json!({"status": "open", "keyword": "午饭"}))
                .unwrap()
        ),
        json!({"status": "open", "keyword": "午饭"})
    );
    assert_eq!(
        issues(document.validate_inputs(&json!({"name": "x"}))),
        [("name".to_owned(), UiFormIssue::Unknown)],
        "a form input is not a filter"
    );
    // The row action's form still validates its own input.
    let rename = &document.actions[0];
    assert_eq!(
        issues(document.validate_form(rename, &json!({}))),
        [("name".to_owned(), UiFormIssue::Required)]
    );
    assert_eq!(
        Value::Object(
            document
                .validate_form(rename, &json!({"name": "新"}))
                .unwrap()
        ),
        json!({"name": "新"})
    );
    // The resolved rows block names its filters; every input is still listed.
    let admin = admin_page(admin_document(Some(json!(["status"]))));
    let view = admin.resolve(&json!({"items": [{"id": "a", "name": "A"}]}), "", "s");
    assert_eq!(view.inputs.len(), 3);
    let ResolvedUiBlock::Rows(rows) = only_block(&view) else {
        panic!()
    };
    assert_eq!(rows.filters.as_deref(), Some(&["status".to_owned()][..]));
    // `[]`: no filters at all.
    let none = doc(admin_document(Some(json!([]))));
    none.validate().unwrap();
    assert!(none.filter_inputs().is_empty());
    assert_eq!(
        issues(none.validate_inputs(&json!({"status": "open"}))),
        [("status".to_owned(), UiFormIssue::Unknown)]
    );
}

#[test]
fn admin_page_without_filters_keeps_every_input_a_filter() {
    let document = doc(admin_document(None));
    document.validate().unwrap();
    assert_eq!(document.filter_inputs().len(), 3);
    assert_eq!(
        issues(document.validate_inputs(&json!({"status": "open"}))),
        [("name".to_owned(), UiFormIssue::Required)],
        "as before: every input is a filter"
    );
    let admin = admin_page(admin_document(None));
    let view = admin.resolve(&json!({"items": []}), "", "s");
    let block = serde_json::to_value(only_block(&view)).unwrap();
    assert!(block.get("filters").is_none());
    let declared = serde_json::to_value(&admin.document).unwrap();
    assert!(declared["blocks"][0].get("filters").is_none());
}

#[test]
fn admin_page_filters_name_unique_document_inputs() {
    for filters in [
        json!(["missing"]),
        json!(["status", "status"]),
        json!(["status", ""]),
        json!("status"),
    ] {
        assert!(invalid(admin_document(Some(filters.clone()))), "{filters}");
    }
    assert!(!invalid(admin_document(Some(json!([
        "keyword", "status", "name"
    ])))));
}

// ---- choices: declared options ----

fn rsvp(extra: Value) -> Value {
    let mut block = json!({
        "kind": "choices",
        "options": [{"label": "接受"}, {"label": "待定"}, {"label": "拒绝"}],
        "countsPath": "/counts",
        "selectedPath": "/myResponse",
        "showResults": "always",
        "action": "respond",
        "inputName": "response"
    });
    for (key, value) in extra.as_object().unwrap() {
        block[key] = value.clone();
    }
    json!({
        "title": "日程",
        "blocks": [block],
        "actions": [{"id": "respond", "label": "回复", "target": op("respond"),
                     "input": {"scheduleId": "/scheduleId"}}],
        "translations": {"en-US": {"title": "Event", "fields": [], "actions": {"respond": "Reply"},
            "texts": {"接受": "Accept", "待定": "Maybe", "拒绝": "Decline"}}}
    })
}

#[test]
fn choices_with_declared_options_translate_labels_and_read_counts_from_data() {
    let data = json!({"scheduleId": "s1", "counts": [3, 1, "2"], "myResponse": 0});
    let view = resolve(rsvp(json!({})), data.clone());
    let ResolvedUiBlock::Choices(choices) = only_block(&view) else {
        panic!()
    };
    assert_eq!(
        choices
            .options
            .iter()
            .map(|o| (o.label.as_str(), o.count, o.selected))
            .collect::<Vec<_>>(),
        [
            ("Accept", Some(3), true),
            ("Maybe", Some(1), false),
            ("Decline", Some(2), false)
        ]
    );
    assert!(choices.voted && !choices.multiple);
    assert_eq!(choices.total, Some(6));
    assert_eq!(choices.input_name, "response");
    assert_eq!(choices.action.as_ref().unwrap().label, "Reply");
    // Untranslated, the declared label shows; no data at all still shows the
    // options (nothing chosen, zero counts).
    let contribution = contribution("message-content", rsvp(json!({})), None);
    let plain = contribution.resolve(&json!({"scheduleId": "s1"}), "zh-CN", "u1");
    let ResolvedUiBlock::Choices(plain) = only_block(&plain) else {
        panic!()
    };
    assert_eq!(
        plain
            .options
            .iter()
            .map(|o| (o.label.as_str(), o.count))
            .collect::<Vec<_>>(),
        [("接受", Some(0)), ("待定", Some(0)), ("拒绝", Some(0))]
    );
    assert!(!plain.voted);
    // The selection is checked against the declared options.
    let block = contribution.schema.choices_for("respond").unwrap();
    assert_eq!(
        block.selection_input(&data, &[2]).unwrap(),
        ("response".to_owned(), json!([2]))
    );
    assert!(block.selection_input(&data, &[3]).is_err());
    assert!(block.selection_input(&data, &[0, 1]).is_err());
    // Declared options serialize as declared, without an optionsPath.
    let declared = serde_json::to_value(&contribution.schema.blocks[0]).unwrap();
    assert!(declared.get("optionsPath").is_none());
    assert_eq!(declared["options"][2], json!({"label": "拒绝"}));
}

#[test]
fn choices_take_options_from_exactly_one_source() {
    assert!(!invalid(rsvp(json!({}))));
    assert!(invalid(rsvp(json!({"optionsPath": "/options"}))));
    assert!(invalid(rsvp(json!({"options": []}))), "neither source");
    assert!(invalid(rsvp(json!({"labelPath": "/text"}))));
    assert!(invalid(rsvp(json!({"options": [{"label": " "}]}))));
    assert!(invalid(rsvp(json!({"options": [{"label": "a\nb"}]}))));
    assert!(invalid(rsvp(
        json!({"options": [{"label": "x", "value": "x"}]})
    )));
    let fifty: Vec<Value> = (0..50).map(|i| json!({"label": format!("o{i}")})).collect();
    assert!(!invalid(rsvp(json!({"options": fifty}))));
    let fifty_one: Vec<Value> = (0..51).map(|i| json!({"label": format!("o{i}")})).collect();
    assert!(invalid(rsvp(json!({"options": fifty_one}))));
    // optionsPath alone keeps its meaning.
    let mut from_data = rsvp(json!({"optionsPath": "/labels"}));
    from_data["blocks"][0]
        .as_object_mut()
        .unwrap()
        .remove("options");
    let view = resolve(from_data, json!({"labels": ["甲", "乙"], "counts": [1, 1]}));
    let ResolvedUiBlock::Choices(choices) = only_block(&view) else {
        panic!()
    };
    assert_eq!(choices.options.len(), 2);
    assert_eq!(choices.options[0].label, "甲");
}

// ---- composer-action data and maxPath ----

fn composer_manifest() -> PluginManifest {
    let manifest = json!({
        "manifestVersion": 1, "pluginId": "example.poll", "version": "1.0.0", "apiVersion": "1.0",
        "displayName": "Poll", "publisher": {"publisherId": "example", "displayName": "Example"},
        "runtime": {"kind": "client"},
        "capabilities": [{"id": "example.poll", "operations": [{
            "id": "create", "inputSchema": {"type": "object"}, "outputSchema": {"type": "object"},
            "permissionIds": ["example.poll.use"], "idempotent": true, "timeoutMs": 1000
        }]}],
        "permissions": [{"id": "example.poll.use", "description": "use", "dataScopes": [], "riskLevel": "low"}],
        "features": [{"id": "create", "label": {"en-US": "Create"}, "gates": {"operations": ["example.poll.create"]}}],
        "configurationSchema": {"type": "object", "properties": {
            "maxOptions": {"type": "integer", "x-flare-client-visible": true},
            "requireConfirmDefault": {"type": "boolean", "x-flare-client-visible": true},
            "maxAssignees": {"type": "integer", "x-flare-client-visible": true}
        }},
        "clientExtensions": {"platforms": ["web"], "contributions": [{
            "kind": "composer-action", "id": "example.poll.compose", "schemaVersion": 1,
            "requiredPermissions": ["example.poll.use"],
            "schema": {
                "title": "发起",
                "inputs": [
                    {"id": "options", "kind": "options", "label": "选项", "min": 2, "max": 20,
                     "maxPath": "/config/maxOptions", "default": ["", ""]},
                    {"id": "requireConfirm", "kind": "toggle", "label": "需要确认", "default": true,
                     "defaultPath": "/config/requireConfirmDefault"},
                    {"id": "assignees", "kind": "members", "label": "负责人", "min": 1, "max": 50,
                     "maxPath": "/config/maxAssignees"}
                ],
                "actions": [{"id": "create", "label": "发起", "target":
                    {"kind": "operation", "operation": "example.poll.create"},
                    "form": ["options", "requireConfirm", "assignees"]}]
            }
        }]}
    });
    PluginManifest::from_json(&serde_json::to_vec(&manifest).unwrap()).unwrap()
}
fn composer_catalog(configuration: Value) -> UiCatalog {
    let manifest = composer_manifest();
    UiCatalog {
        tenant_id: "201".into(),
        version: 1,
        valid_until_ms: 10000,
        plugins: vec![ClientPlugin {
            query_operations: Default::default(),
            operations: manifest.operation_ids(),
            configuration: manifest
                .client_visible_configuration(&configuration)
                .unwrap(),
            plugin_id: manifest.plugin_id,
            version: manifest.version,
            manifest_digest: format!("sha256:{}", "a".repeat(64)),
            grant_revision: 1,
            extensions: manifest.client_extensions,
        }],
        unavailable: vec![],
    }
}
fn composer_view(catalog: &UiCatalog, payload: Value) -> ResolvedPluginView {
    catalog.resolve_with_state(
        &PluginMessage {
            plugin_id: "example.poll".into(),
            contribution_id: "example.poll.compose".into(),
            schema_version: 1,
            summary: "发起".into(),
            payload,
            display_name: None,
        },
        UiStateLayers::default(),
        UiRenderContext {
            tenant: "201",
            platform: ClientPlatform::Web,
            kind: UiContributionKind::ComposerAction,
            locale: "",
            viewer: "u1",
            now_ms: 1000,
        },
    )
}
fn input<'a>(view: &'a ResolvedPluginView, id: &str) -> &'a UiInput {
    view.inputs.iter().find(|i| i.id == id).unwrap()
}

#[test]
fn composer_documents_resolve_against_the_client_visible_configuration() {
    assert_eq!(composer_data(&Map::new()), json!({"config": {}}));
    let catalog = composer_catalog(json!({
        "maxOptions": 30, "requireConfirmDefault": false, "maxAssignees": 200
    }));
    assert_eq!(
        catalog.plugins[0].composer_data(),
        json!({"config": {"maxOptions": 30, "requireConfirmDefault": false, "maxAssignees": 200}})
    );
    // The payload and its keys play no part: only the configuration is read.
    let view = composer_view(&catalog, json!({"config": {"maxOptions": 3}}));
    assert!(view.available);
    let options = input(&view, "options");
    assert_eq!(options.max, Some(30.into()));
    assert_eq!(
        options.max_path, None,
        "resolved inputs carry values, not paths"
    );
    let confirm = input(&view, "requireConfirm");
    assert_eq!(confirm.default, Some(json!(false)));
    assert_eq!(confirm.default_path, None);
    assert_eq!(input(&view, "assignees").max, Some(200.into()));
    // Unconfigured keys fall back to the declared values.
    let bare = composer_view(&composer_catalog(json!({})), json!({}));
    assert_eq!(input(&bare, "options").max, Some(20.into()));
    assert_eq!(input(&bare, "requireConfirm").default, Some(json!(true)));
    assert_eq!(input(&bare, "assignees").max, Some(50.into()));
    // Out-of-bounds configuration: capped at the kind's ceiling, or the
    // declared max when it would make the input invalid.
    let capped = composer_view(
        &composer_catalog(json!({"maxOptions": 80, "maxAssignees": 9000})),
        json!({}),
    );
    assert_eq!(input(&capped, "options").max, Some(50.into()));
    assert_eq!(input(&capped, "assignees").max, Some(500.into()));
    for unusable in [json!(0), json!(1), json!(-3), json!(2.5)] {
        let view = composer_view(
            &composer_catalog(json!({"maxOptions": unusable.clone(), "maxAssignees": 0})),
            json!({}),
        );
        assert_eq!(input(&view, "options").max, Some(20.into()), "{unusable}");
        assert_eq!(input(&view, "assignees").max, Some(50.into()));
    }
}

#[test]
fn composer_forms_validate_against_the_resolved_bounds() {
    let catalog = composer_catalog(json!({"maxOptions": 3}));
    let plugin = &catalog.plugins[0];
    let compose = &plugin.extensions.contributions[0];
    let action = &compose.schema.actions[0];
    let data = plugin.document_data(compose, &json!({"ignored": true}), UiStateLayers::default());
    assert_eq!(data, json!({"config": {"maxOptions": 3}}));
    let four = json!({"options": ["a", "b", "c", "d"], "assignees": ["u1"]});
    // The declared form allows 20 options; the configured form only 3.
    assert!(compose.schema.validate_form(action, &four).is_ok());
    assert_eq!(
        issues(compose.schema.validate_form_over(action, &data, &four)),
        [("options".to_owned(), UiFormIssue::TooManyItems)]
    );
    let ok = compose
        .schema
        .validate_form_over(
            action,
            &data,
            &json!({"options": ["a", "b"], "assignees": ["u1"]}),
        )
        .unwrap();
    // An absent toggle takes the default resolved from the data.
    assert_eq!(ok["requireConfirm"], json!(true));
    let off = composer_data(&Map::from_iter([(
        "requireConfirmDefault".to_owned(),
        json!(false),
    )]));
    let ok = compose
        .schema
        .validate_form_over(
            action,
            &off,
            &json!({"options": ["a", "b"], "assignees": ["u1"]}),
        )
        .unwrap();
    assert_eq!(ok["requireConfirm"], json!(false));
}

#[test]
fn max_path_is_for_options_and_members_only() {
    let form = |input: Value| {
        json!({"title": "t", "inputs": [input],
               "actions": [{"id": "go", "label": "go", "target": op("go"), "form": ["x"]}]})
    };
    assert!(!invalid(form(
        json!({"id": "x", "kind": "options", "label": "l", "maxPath": "/config/max"})
    )));
    assert!(!invalid(form(
        json!({"id": "x", "kind": "members", "label": "l", "maxPath": "/config/max"})
    )));
    for kind in ["text", "textarea", "number", "toggle", "datetime"] {
        assert!(
            invalid(form(
                json!({"id": "x", "kind": kind, "label": "l", "maxPath": "/config/max"})
            )),
            "{kind}"
        );
    }
    assert!(invalid(form(
        json!({"id": "x", "kind": "options", "label": "l", "maxPath": "config"})
    )));
    // A default longer than the configured max keeps the declared max.
    let view = resolve(
        form(json!({"id": "x", "kind": "options", "label": "l", "max": 5,
                    "maxPath": "/limit", "default": ["", "", ""]})),
        json!({"limit": 2}),
    );
    assert_eq!(view.inputs[0].max, Some(5.into()));
    let view = resolve(
        form(json!({"id": "x", "kind": "options", "label": "l", "max": 5,
                    "maxPath": "/limit", "default": ["", "", ""]})),
        json!({"limit": 4}),
    );
    assert_eq!(view.inputs[0].max, Some(4.into()));
    // Without a declared max the kind's ceiling is the fallback.
    let view = resolve(
        form(json!({"id": "x", "kind": "members", "label": "l", "maxPath": "/limit"})),
        json!({"limit": "8"}),
    );
    assert_eq!(view.inputs[0].max, None);
}

// ---- allOf ----

fn with_hidden(conditions: Value) -> Value {
    json!({
        "title": "t",
        "blocks": [
            {"kind": "note", "text": "仅限参与人", "hiddenWhen": conditions.clone()},
            {"kind": "divider"}
        ],
        "actions": [{"id": "reply", "label": "回复", "target": op("reply"), "hiddenWhen": conditions}]
    })
}
/// Whether the note block and the action are shown on `data` for `u1`.
fn shown(conditions: Value, data: Value) -> (bool, bool) {
    let view = resolve(with_hidden(conditions), data);
    (
        view.blocks
            .iter()
            .any(|b| matches!(b, ResolvedUiBlock::Note(_))),
        !view.actions.is_empty(),
    )
}

#[test]
fn all_of_matches_when_every_condition_matches() {
    // Hidden only when the payload restricts participants and the viewer is
    // not one of them.
    let restricted = json!([{"allOf": [
        {"path": "/participantIds", "exists": true},
        {"path": "/participantIds", "viewer": true, "negate": true}
    ]}]);
    assert_eq!(shown(restricted.clone(), json!({})), (true, true));
    assert_eq!(
        shown(restricted.clone(), json!({"participantIds": []})),
        (true, true)
    );
    assert_eq!(
        shown(restricted.clone(), json!({"participantIds": ["u1", "u2"]})),
        (true, true)
    );
    assert_eq!(
        shown(restricted, json!({"participantIds": ["u2"]})),
        (false, false)
    );
    // Negated: matches unless every condition matches.
    let negated = json!([{"negate": true, "allOf": [
        {"path": "/status", "anyOf": ["open"]},
        {"path": "/creatorId", "viewer": true}
    ]}]);
    assert_eq!(
        shown(
            negated.clone(),
            json!({"status": "open", "creatorId": "u1"})
        ),
        (true, true)
    );
    assert_eq!(
        shown(
            negated.clone(),
            json!({"status": "closed", "creatorId": "u1"})
        ),
        (false, false)
    );
    assert_eq!(shown(negated, json!({})), (false, false));
    // Beside other conditions the list still hides on any match.
    let mixed = json!([
        {"path": "/closed", "anyOf": ["true"]},
        {"allOf": [{"path": "/a", "exists": true}, {"path": "/b", "exists": true}]}
    ]);
    assert_eq!(
        shown(mixed.clone(), json!({"closed": true})),
        (false, false)
    );
    assert_eq!(
        shown(mixed.clone(), json!({"a": 1, "b": 2})),
        (false, false)
    );
    assert_eq!(shown(mixed, json!({"a": 1})), (true, true));
}

#[test]
fn all_of_works_on_closed_when_and_the_detail() {
    let document = json!({
        "title": "t",
        "blocks": [{"kind": "choices", "optionsPath": "/options", "action": "vote",
                    "closedWhen": [{"allOf": [
                        {"path": "/deadlinePassed", "anyOf": ["true"]},
                        {"path": "/extended", "exists": true, "negate": true}
                    ]}]}],
        "actions": [{"id": "vote", "label": "投票", "target": op("vote")}]
    });
    let closed = |data: Value| {
        let view = resolve(document.clone(), data);
        let ResolvedUiBlock::Choices(choices) = only_block(&view) else {
            panic!()
        };
        choices.closed
    };
    assert!(closed(json!({"options": ["a"], "deadlinePassed": true})));
    assert!(!closed(
        json!({"options": ["a"], "deadlinePassed": true, "extended": 1})
    ));
    assert!(!closed(json!({"options": ["a"]})));
    let card = contribution(
        "message-content",
        json!({"title": "t"}),
        Some(
            json!({"operation": "x.op.details", "document": {"title": "d"},
            "hiddenWhen": [{"allOf": [
                {"path": "/anonymous", "anyOf": ["true"]},
                {"path": "/creatorId", "viewer": true, "negate": true}
            ]}]}),
        ),
    );
    card.detail.as_ref().unwrap().validate().unwrap();
    assert!(
        !card
            .resolve(&json!({"anonymous": true, "creatorId": "u2"}), "", "u1")
            .detail
    );
    assert!(
        card.resolve(&json!({"anonymous": true, "creatorId": "u1"}), "", "u1")
            .detail
    );
    assert!(card.resolve(&json!({}), "", "u1").detail);
}

#[test]
fn all_of_is_one_level_of_up_to_four_conditions_without_a_path() {
    let leaf = json!({"path": "/a", "exists": true});
    let valid = |condition: Value| !invalid(with_hidden(json!([condition])));
    assert!(valid(json!({"allOf": [leaf.clone()]})));
    assert!(valid(json!({"allOf": vec![leaf.clone(); 4]})));
    assert!(!valid(json!({"allOf": vec![leaf.clone(); 5]})));
    assert!(!valid(json!({"allOf": []})), "no test at all");
    assert!(
        !valid(json!({"allOf": [{"allOf": [leaf.clone()]}]})),
        "nested once only"
    );
    assert!(!valid(json!({"path": "/a", "allOf": [leaf.clone()]})));
    assert!(!valid(json!({"allOf": [leaf.clone()], "exists": true})));
    assert!(!valid(json!({"allOf": [leaf.clone()], "anyOf": ["x"]})));
    assert!(!valid(json!({"allOf": [leaf.clone()], "viewer": true})));
    assert!(
        !valid(json!({"allOf": [{"exists": true}]})),
        "leaves need a path"
    );
    assert!(!valid(json!({"allOf": [{"path": "a", "exists": true}]})));
    assert!(!valid(json!({"exists": true})), "a leaf needs a path");
    // Four conditions per list, an allOf counting as one.
    let four = json!([{"allOf": vec![leaf.clone(); 4]}, leaf.clone(), leaf.clone(), leaf.clone()]);
    assert!(!invalid(with_hidden(four)));
    let five = json!([{"allOf": [leaf.clone()]}, leaf.clone(), leaf.clone(), leaf.clone(), leaf]);
    assert!(invalid(with_hidden(five)));
    // Serialized: no path on an allOf, no allOf on a leaf.
    let document = doc(with_hidden(json!([
        {"allOf": [{"path": "/a", "exists": true}]},
        {"path": "/b", "anyOf": ["x"]}
    ])));
    assert_eq!(
        serde_json::to_value(&document.actions[0].hidden_when).unwrap(),
        json!([{"allOf": [{"path": "/a", "exists": true}]}, {"path": "/b", "anyOf": ["x"]}])
    );
}

// ---- entries: aligned user ids ----

#[test]
fn entries_take_user_ids_aligned_by_position() {
    let document = json!({"title": "接龙", "blocks": [{
        "kind": "entries", "itemsPath": "/lines", "userIdsPath": "/lineUserIds", "limit": 4
    }]});
    let view = resolve(
        document.clone(),
        json!({
            "lines": ["一", "二", "三", "四", "五"],
            "lineUserIds": ["u1", "../bad", 7, "u4", "u5"]
        }),
    );
    let ResolvedUiBlock::Entries(entries) = only_block(&view) else {
        panic!()
    };
    assert_eq!(
        entries
            .items
            .iter()
            .map(|e| (e.index, e.text.as_str(), e.user_id.as_deref()))
            .collect::<Vec<_>>(),
        [
            (1, "一", Some("u1")),
            (2, "二", None),
            (3, "三", None),
            (4, "四", Some("u4"))
        ]
    );
    // Shorter or missing arrays leave the rest without a user.
    let view = resolve(
        document,
        json!({"lines": ["一", "二"], "lineUserIds": ["u1"]}),
    );
    let ResolvedUiBlock::Entries(entries) = only_block(&view) else {
        panic!()
    };
    assert_eq!(entries.items[1].user_id, None);
    // An entry's own user wins; the aligned id fills the gaps.
    let view = resolve(
        json!({"title": "t", "blocks": [{"kind": "entries", "itemsPath": "/items",
            "textPath": "/text", "userPath": "/userId", "userIdsPath": "/users"}]}),
        json!({
            "items": [{"text": "a", "userId": "own"}, {"text": "b"}],
            "users": ["aligned-0", "aligned-1"]
        }),
    );
    let ResolvedUiBlock::Entries(entries) = only_block(&view) else {
        panic!()
    };
    assert_eq!(entries.items[0].user_id.as_deref(), Some("own"));
    assert_eq!(entries.items[1].user_id.as_deref(), Some("aligned-1"));
    assert!(invalid(
        json!({"title": "t", "blocks": [{"kind": "entries", "itemsPath": "/l", "userIdsPath": "x"}]})
    ));
}

// ---- detail paging ----

fn paged_card() -> UiContribution {
    contribution(
        "message-content",
        json!({"title": "接龙", "blocks": [{"kind": "entries", "itemsPath": "/lines", "limit": 3}]}),
        Some(json!({
            "operation": "x.op.details",
            "input": {"solitaireId": "/solitaireId"},
            "cursorPath": "/nextCursor",
            "document": {"title": "接龙详情", "blocks": [
                {"kind": "text", "path": "/topic"},
                {"kind": "entries", "itemsPath": "/entries", "textPath": "/text",
                 "userPath": "/userId", "countPath": "/count", "limit": 50}
            ]}
        })),
    )
}
fn page(from: u32, to: u32, next: Option<&str>) -> Value {
    let mut data = json!({
        "topic": "周五聚餐",
        "count": 5,
        "entries": (from..=to).map(|i| json!({"text": format!("第{i}条"), "userId": format!("u{i}")})).collect::<Vec<_>>()
    });
    if let Some(next) = next {
        data["nextCursor"] = json!(next);
    }
    data
}
fn entries_of(view: &ResolvedPluginView) -> &ResolvedUiEntries {
    view.blocks
        .iter()
        .find_map(|b| match b {
            ResolvedUiBlock::Entries(e) => Some(e),
            _ => None,
        })
        .unwrap()
}

#[test]
fn paged_detail_carries_the_next_cursor_and_appends_pages() {
    let card = paged_card();
    let detail = card.detail.as_ref().unwrap();
    detail.validate().unwrap();
    let first_input = detail
        .bind_input(&json!({"solitaireId": "s1", "lines": []}))
        .unwrap();
    assert_eq!(
        first_input,
        json!({"solitaireId": "s1"}),
        "no cursor on page one"
    );
    let mut view = card
        .resolve_detail(&page(1, 2, Some("c-2")), "", "u1")
        .unwrap();
    assert_eq!(view.next_cursor.as_deref(), Some("c-2"));
    assert_eq!(
        serde_json::to_value(&view).unwrap()["nextCursor"],
        json!("c-2")
    );
    let second_input = detail.page_input(first_input.clone(), "c-2").unwrap();
    assert_eq!(second_input, json!({"solitaireId": "s1", "cursor": "c-2"}));
    let second = card
        .resolve_detail(&page(1, 2, Some("c-3")), "", "u1")
        .unwrap();
    view.append_page(second);
    assert_eq!(view.next_cursor.as_deref(), Some("c-3"));
    let last = card.resolve_detail(&page(1, 1, None), "", "u1").unwrap();
    assert_eq!(last.next_cursor, None);
    view.append_page(last);
    assert_eq!(view.next_cursor, None, "the last page ends paging");
    let entries = entries_of(&view);
    assert_eq!(
        entries
            .items
            .iter()
            .map(|e| (e.index, e.user_id.as_deref().unwrap()))
            .collect::<Vec<_>>(),
        [(1, "u1"), (2, "u2"), (3, "u1"), (4, "u2"), (5, "u1")],
        "numbered on across pages"
    );
    assert_eq!(entries.total, 5);
    assert!(!entries.more);
    assert_eq!(view.blocks.len(), 2, "other blocks stay as on page one");
    // Without countPath the total grows with the merged entries.
    let mut uncounted = card
        .resolve_detail(
            &json!({"entries": [{"text": "a"}, {"text": "b"}]}),
            "",
            "u1",
        )
        .unwrap();
    uncounted.append_page(
        card.resolve_detail(&json!({"entries": [{"text": "c"}]}), "", "u1")
            .unwrap(),
    );
    assert_eq!(entries_of(&uncounted).total, 3);
    // An unavailable page stops paging and leaves the entries.
    let mut stopped = card
        .resolve_detail(&page(1, 2, Some("c-2")), "", "u1")
        .unwrap();
    let mut unavailable = stopped.clone();
    unavailable.available = false;
    stopped.append_page(unavailable);
    assert_eq!(stopped.next_cursor, None);
    assert_eq!(entries_of(&stopped).items.len(), 2);
}

#[test]
fn detail_cursors_are_bounded_and_need_cursor_path() {
    let card = paged_card();
    let detail = card.detail.as_ref().unwrap();
    for (next, expected) in [
        (json!("c"), Some("c".to_owned())),
        (json!(42), Some("42".to_owned())),
        (json!(""), None),
        (json!("  "), None),
        (json!(null), None),
        (json!(["c"]), None),
        (json!("c".repeat(257)), None),
        (json!("c\u{1}"), None),
    ] {
        let mut data = page(1, 1, None);
        data["nextCursor"] = next.clone();
        assert_eq!(
            card.resolve_detail(&data, "", "u1").unwrap().next_cursor,
            expected,
            "{next}"
        );
    }
    let bound = json!({"solitaireId": "s1"});
    assert!(detail.page_input(bound.clone(), &"c".repeat(256)).is_ok());
    for cursor in ["", " ", "a\nb"] {
        assert!(
            detail.page_input(bound.clone(), cursor).is_err(),
            "{cursor:?}"
        );
    }
    assert!(detail.page_input(bound.clone(), &"c".repeat(257)).is_err());
    assert!(
        detail
            .page_input(json!({"solitaireId": "s1", "cursor": "x"}), "c")
            .is_err()
    );
    assert!(detail.page_input(json!(["s1"]), "c").is_err());
    // A detail without cursorPath does not page.
    let mut unpaged = detail.clone();
    unpaged.cursor_path = None;
    assert!(unpaged.page_input(bound, "c").is_err());
    let plain = contribution(
        "message-content",
        json!({"title": "t"}),
        Some(json!({"operation": "x.op.details", "document": {"title": "d"}})),
    );
    let view = plain
        .resolve_detail(&json!({"nextCursor": "c"}), "", "u1")
        .unwrap();
    assert_eq!(view.next_cursor, None);
    assert!(
        serde_json::to_value(&view)
            .unwrap()
            .get("nextCursor")
            .is_none()
    );
    // cursorPath is a path, and `cursor` is then the host's to send.
    let mut bad = detail.clone();
    bad.cursor_path = Some("nextCursor".into());
    assert!(bad.validate().is_err());
    let mut clash = detail.clone();
    clash.input.insert("cursor".into(), "/cursor".into());
    assert!(clash.validate().is_err());
    clash.cursor_path = None;
    assert!(
        clash.validate().is_ok(),
        "without paging `cursor` may be bound"
    );
}

#[test]
fn paged_detail_resolves_through_the_catalog_and_pages_admin_rows() {
    let mut manifest: Value =
        serde_json::from_slice(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap();
    manifest["clientExtensions"]["contributions"][0]["detail"]["cursorPath"] = json!("/nextCursor");
    manifest["admin"]["pages"][0]["detail"]["cursorPath"] = json!("/nextCursor");
    let manifest = PluginManifest::from_json(&serde_json::to_vec(&manifest).unwrap()).unwrap();
    let catalog = UiCatalog {
        tenant_id: "201".into(),
        version: 1,
        valid_until_ms: 10000,
        plugins: vec![ClientPlugin {
            query_operations: Default::default(),
            operations: manifest.operation_ids(),
            plugin_id: manifest.plugin_id.clone(),
            version: manifest.version.clone(),
            manifest_digest: format!("sha256:{}", "a".repeat(64)),
            grant_revision: 1,
            extensions: manifest.client_extensions.clone(),
            configuration: Default::default(),
        }],
        unavailable: vec![],
    };
    let message = PluginMessage {
        plugin_id: "example.poll".into(),
        contribution_id: "example.poll.card".into(),
        schema_version: 1,
        summary: "投票".into(),
        payload: json!({"pollId": "p-1"}),
        display_name: None,
    };
    let context = UiRenderContext {
        tenant: "201",
        platform: ClientPlatform::Web,
        kind: UiContributionKind::MessageContent,
        locale: "",
        viewer: "u1",
        now_ms: 1000,
    };
    let view = catalog.resolve_detail(
        &message,
        &json!({"voters": [{"option": "面", "userId": "u1"}], "nextCursor": "v-2"}),
        context,
    );
    assert_eq!(view.next_cursor.as_deref(), Some("v-2"));
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
    assert_eq!(
        detail
            .page_input(detail.bind_input(&message.payload).unwrap(), "v-2")
            .unwrap(),
        json!({"pollId": "p-1", "cursor": "v-2"})
    );
    // An admin row's detail input pages the same way.
    let admin = manifest.admin.as_ref().unwrap().page("polls").unwrap();
    let page = admin.resolve(
        &json!({"items": [{"pollId": "p9", "question": "q", "status": "open"}]}),
        "",
        "s",
    );
    let ResolvedUiBlock::Rows(rows) = &page.blocks[0] else {
        panic!()
    };
    let row_input = rows.rows[0].detail_input.clone().unwrap();
    assert_eq!(
        admin
            .detail
            .as_ref()
            .unwrap()
            .page_input(row_input, "v-2")
            .unwrap(),
        json!({"pollId": "p9", "cursor": "v-2"})
    );
}

// ---- older manifests ----

#[test]
fn manifests_without_the_additions_parse_and_serialize_as_before() {
    for (name, raw) in [
        (
            "red packet 0.4.1",
            &include_bytes!("fixtures/red-packet-0.4.1.json")[..],
        ),
        (
            "ui v2 fixture",
            &include_bytes!("fixtures/plugin-ui-v2-manifest.json")[..],
        ),
        (
            "ui v1 fixture",
            &include_bytes!("fixtures/plugin-ui-manifest.json")[..],
        ),
    ] {
        let manifest = PluginManifest::from_json(raw).unwrap();
        let reserialized = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(
            PluginManifest::from_json(&reserialized).unwrap(),
            manifest,
            "{name}"
        );
        let reserialized = String::from_utf8(reserialized).unwrap();
        for key in ["filters", "options\":[{", "allOf", "maxPath", "userIdsPath"] {
            assert!(!reserialized.contains(key), "{name}: {key} leaked");
        }
        // (The v2 fixture's rows block has its own cursorPath.)
        assert!(
            manifest
                .client_extensions
                .contributions
                .iter()
                .filter_map(|c| c.detail.as_ref())
                .all(|d| d.cursor_path.is_none()),
            "{name}"
        );
    }
    // Red packet 0.4.1 serializes back to its own clientExtensions, and its
    // resolved cards gain no key.
    let raw = include_bytes!("fixtures/red-packet-0.4.1.json");
    let manifest = PluginManifest::from_json(raw).unwrap();
    let original: Value = serde_json::from_slice(raw).unwrap();
    assert_eq!(
        serde_json::to_value(&manifest.client_extensions).unwrap(),
        original["clientExtensions"]
    );
    let card = manifest.contribution("flare.redpacket.card").unwrap();
    let view = card.resolve(
        &json!({"packetId": "p", "greeting": "恭喜", "kind": "lucky", "status": "open"}),
        "en-US",
        "u1",
    );
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(
        json.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["actions", "available", "fields", "title"]
    );
}

#[test]
fn schema_export_describes_the_additions() {
    let schema = serde_json::to_value(schemars::schema_for!(PluginManifest)).unwrap();
    let defs = &schema["$defs"];
    let block = |kind: &str| {
        defs["UiBlock"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["properties"]["kind"]["const"] == json!(kind))
            .cloned()
            .unwrap_or_else(|| panic!("{kind} block missing"))
    };
    assert!(block("rows")["properties"]["filters"].is_object());
    let choices = block("choices");
    assert!(choices["properties"]["options"].is_object());
    assert!(
        !choices["required"]
            .as_array()
            .is_some_and(|r| r.contains(&json!("optionsPath"))),
        "optionsPath is optional now"
    );
    assert!(block("entries")["properties"]["userIdsPath"].is_object());
    assert!(defs["UiCondition"]["properties"]["allOf"].is_object());
    assert!(
        !defs["UiCondition"]["required"]
            .as_array()
            .is_some_and(|r| r.contains(&json!("path")))
    );
    assert!(defs["UiInput"]["properties"]["maxPath"].is_object());
    assert!(defs["UiDetail"]["properties"]["cursorPath"].is_object());
    let text = schema.to_string();
    assert!(text.contains("im.conversation.members.check"));
}
