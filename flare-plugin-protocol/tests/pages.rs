//! Plugin pages with a data source and paging, rows and entries on page
//! documents, the choices selection limit and per-item badges
//! (`valueLabels`).
use flare_plugin_protocol::{
    PluginManifest,
    manifest::{ClientPlatform, UiContributionKind},
    ui::*,
};
use serde_json::{Map, Value, json};

fn manifest_value() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap()
}
fn parse(manifest: &Value) -> Result<PluginManifest, flare_plugin_protocol::ProtocolError> {
    PluginManifest::from_json(&serde_json::to_vec(manifest).unwrap())
}

/// The ledger page of a wallet: a paged `history` query, an entries block
/// with badges, and a rows block of the same source with a row action.
fn history_page() -> Value {
    json!({
        "kind": "page", "id": "example.poll.history", "schemaVersion": 1,
        "requiredPermissions": [],
        "source": {"operation": "example.poll.history", "input": {"currency": "/currency"},
                   "cursorPath": "/nextCursor"},
        "schema": {
            "title": "明细",
            "fields": [{"label": "余额", "path": "/balanceText"}],
            "blocks": [
                {"kind": "entries", "itemsPath": "/lines", "textPath": "/amountText",
                 "userPath": "/counterpartyId", "numbered": false,
                 "valueLabels": {"path": "/kind", "values": {
                     "send": {"label": "发出红包"},
                     "claim": {"label": "收到红包", "tone": "success"},
                     "refund": {"label": "红包退回", "tone": "warning"},
                     "deposit": {"label": "入账", "tone": "info"}}}},
                {"kind": "rows", "source": "example.poll.history", "path": "/lines",
                 "keyPath": "/id", "filters": ["kind"],
                 "columns": [{"label": "金额", "path": "/amountText"},
                             {"label": "时间", "path": "/atMs", "format": "time"}],
                 "actions": ["open"],
                 "valueLabels": {"path": "/kind", "values": {
                     "send": {"label": "发出红包"},
                     "claim": {"label": "收到红包", "tone": "success"}}}}
            ],
            "inputs": [{"id": "kind", "kind": "select", "label": "类型",
                        "choices": [{"value": "send", "label": "发出"},
                                    {"value": "claim", "label": "收到"}]}],
            "actions": [
                {"id": "open", "label": "查看", "input": {"packetId": "/packetId"},
                 "target": {"kind": "operation", "operation": "example.poll.view"}},
                {"id": "wallet", "label": "钱包",
                 "target": {"kind": "page", "page": "example.poll.wallet"}}
            ],
            "translations": {"en": {"title": "History", "fields": ["Balance"],
                                    "texts": {"发出红包": "Sent", "收到红包": "Received"}}}
        }
    })
}
fn wallet_page() -> Value {
    json!({
        "kind": "page", "id": "example.poll.wallet", "schemaVersion": 1,
        "requiredPermissions": [],
        "source": {"operation": "example.poll.wallet"},
        "schema": {"title": "钱包", "fields": [{"label": "余额", "path": "/balanceText"}],
                   "actions": [{"id": "history", "label": "全部明细",
                                "target": {"kind": "page", "page": "example.poll.history"}}]}
    })
}
/// The fixture with `view`, `wallet` and `history` queries, a `transfer`
/// command and the two pages.
fn page_manifest() -> Value {
    let mut manifest = manifest_value();
    let operations = manifest["capabilities"][0]["operations"]
        .as_array_mut()
        .unwrap();
    for (id, mode) in [
        ("view", Some("query")),
        ("wallet", Some("query")),
        ("history", Some("query")),
        ("transfer", None),
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
    let contributions = manifest["clientExtensions"]["contributions"]
        .as_array_mut()
        .unwrap();
    contributions.push(wallet_page());
    contributions.push(history_page());
    manifest
}
fn page_index(manifest: &Value, id: &str) -> usize {
    manifest["clientExtensions"]["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c["id"] == id)
        .unwrap()
}
fn reject(edit: &dyn Fn(&mut Value)) -> bool {
    let mut manifest = page_manifest();
    edit(&mut manifest);
    parse(&manifest).is_err()
}
fn history(manifest: &mut Value) -> &mut Value {
    let index = page_index(manifest, "example.poll.history");
    &mut manifest["clientExtensions"]["contributions"][index]
}

// ---- page source ----

#[test]
fn a_page_reads_a_declared_query_and_may_hold_rows_and_entries() {
    let manifest = parse(&page_manifest()).unwrap();
    let page = manifest.contribution("example.poll.history").unwrap();
    let source = page.source.as_ref().unwrap();
    assert_eq!(source.operation, "example.poll.history");
    assert_eq!(source.cursor_path.as_deref(), Some("/nextCursor"));
    assert!(
        manifest
            .client_operation_refs()
            .contains("example.poll.wallet"),
        "a page source is a client operation"
    );
    // Round trip keeps the source.
    let raw = serde_json::to_value(page).unwrap();
    assert_eq!(raw["source"]["input"], json!({"currency": "/currency"}));
    // A page without a source is a static page.
    assert!(!reject(&|m| {
        let index = page_index(m, "example.poll.wallet");
        m["clientExtensions"]["contributions"][index]
            .as_object_mut()
            .unwrap()
            .remove("source");
    }));
}

#[test]
fn page_sources_and_page_rows_are_validated() {
    // The source is a declared query.
    assert!(reject(
        &|m| history(m)["source"]["operation"] = json!("example.poll.transfer")
    ));
    assert!(reject(
        &|m| history(m)["source"]["operation"] = json!("example.poll.missing")
    ));
    // Bindings are pointers; `cursor` is the paging input.
    assert!(reject(
        &|m| history(m)["source"]["input"] = json!({"kind": "kind"})
    ));
    assert!(reject(
        &|m| history(m)["source"]["input"] = json!({"cursor": "/c"})
    ));
    assert!(reject(
        &|m| history(m)["source"]["cursorPath"] = json!("next")
    ));
    // Only pages have a source.
    assert!(reject(&|m| {
        let card = page_index(m, "example.poll.card");
        m["clientExtensions"]["contributions"][card]["source"] =
            json!({"operation": "example.poll.view"});
    }));
    // Page rows read the page source and page with it.
    assert!(reject(
        &|m| history(m)["schema"]["blocks"][1]["source"] = json!("example.poll.wallet")
    ));
    assert!(reject(
        &|m| history(m)["schema"]["blocks"][1]["cursorPath"] = json!("/nextCursor")
    ));
    assert!(reject(&|m| {
        history(m).as_object_mut().unwrap().remove("source");
    }));
    // One rows block.
    assert!(reject(&|m| {
        let rows = history(m)["schema"]["blocks"][1].clone();
        history(m)["schema"]["blocks"]
            .as_array_mut()
            .unwrap()
            .push(rows);
    }));
    // A filter may not repeat a source input name.
    assert!(reject(
        &|m| history(m)["source"]["input"] = json!({"kind": "/kind"})
    ));
    // Rows stay off cards and composer forms.
    assert!(reject(&|m| {
        let card = page_index(m, "example.poll.card");
        let rows = json!({"kind": "rows", "source": "example.poll.view", "path": "/x",
                          "keyPath": "/id", "columns": [{"label": "a", "path": "/a"}]});
        m["clientExtensions"]["contributions"][card]["schema"]["blocks"]
            .as_array_mut()
            .unwrap()
            .push(rows);
    }));
}

#[test]
fn a_page_source_binds_arguments_filters_and_cursors() {
    let manifest = parse(&page_manifest()).unwrap();
    let page = manifest.contribution("example.poll.history").unwrap();
    let source = page.source.as_ref().unwrap();
    // A missing argument is left out.
    assert_eq!(
        source.bind_input(&json!({}), Map::new()).unwrap(),
        json!({})
    );
    assert_eq!(
        source
            .bind_input(&json!({"currency": "CNY", "other": 1}), Map::new())
            .unwrap(),
        json!({"currency": "CNY"})
    );
    // Filter values (validated by the document) join the input.
    let filters = page
        .schema
        .validate_inputs(&json!({"kind": "send"}))
        .unwrap();
    assert!(
        page.schema
            .validate_inputs(&json!({"kind": "gift"}))
            .is_err()
    );
    let first = source
        .bind_input(&json!({"currency": "CNY"}), filters.clone())
        .unwrap();
    assert_eq!(first, json!({"currency": "CNY", "kind": "send"}));
    // A filter never replaces a bound input, nor the cursor.
    let colliding = UiPageSource {
        input: [("kind".to_owned(), "/kind".to_owned())].into(),
        ..source.clone()
    };
    assert!(
        colliding
            .bind_input(&json!({"kind": "claim"}), filters)
            .is_err()
    );
    let mut cursor = Map::new();
    cursor.insert("cursor".into(), json!("x"));
    assert!(source.bind_input(&json!({}), cursor).is_err());
    // Paging adds the cursor.
    assert_eq!(
        source.page_input(first.clone(), "c2").unwrap(),
        json!({"currency": "CNY", "kind": "send", "cursor": "c2"})
    );
    assert!(source.page_input(first.clone(), " ").is_err());
    assert!(source.page_input(first, &"c".repeat(300)).is_err());
    let unpaged = UiPageSource {
        cursor_path: None,
        ..source.clone()
    };
    assert!(unpaged.page_input(json!({}), "c2").is_err());
}

fn catalog(manifest: &PluginManifest) -> UiCatalog {
    UiCatalog {
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
    }
}
fn context() -> UiRenderContext<'static> {
    UiRenderContext {
        tenant: "201",
        platform: ClientPlatform::Web,
        kind: UiContributionKind::Page,
        locale: "zh-CN",
        viewer: "u1",
        now_ms: 1000,
    }
}
fn ledger(lines: Value, next: Option<&str>) -> Value {
    let mut data = json!({"balanceText": "¥12.30", "lines": lines});
    if let Some(next) = next {
        data["nextCursor"] = json!(next);
    }
    data
}

#[test]
fn the_catalog_says_why_a_page_cannot_be_shown() {
    let manifest = parse(&page_manifest()).unwrap();
    let catalog = catalog(&manifest);
    let found = |catalog: &UiCatalog, ctx: &UiRenderContext<'_>| {
        catalog
            .page("example.poll", "example.poll.history", ctx)
            .map(|(_, c)| c.id.clone())
    };
    assert_eq!(
        found(&catalog, &context()).as_deref(),
        Ok("example.poll.history")
    );
    let other_tenant = UiRenderContext {
        tenant: "202",
        ..context()
    };
    assert_eq!(
        found(&catalog, &other_tenant),
        Err(UiUnavailableReason::NotInstalled)
    );
    let expired = UiRenderContext {
        now_ms: 10000,
        ..context()
    };
    assert_eq!(
        found(&catalog, &expired),
        Err(UiUnavailableReason::CatalogUnreachable)
    );
    assert_eq!(
        catalog
            .page("example.poll", "example.poll.card", &context())
            .err(),
        Some(UiUnavailableReason::IncompatibleVersion),
        "a card is not a page"
    );
    assert_eq!(
        catalog
            .page("example.other", "example.other.page", &context())
            .err(),
        Some(UiUnavailableReason::NotInstalled)
    );
    let mut disabled = catalog.clone();
    disabled.plugins.clear();
    disabled.unavailable.push(UiUnavailablePlugin {
        plugin_id: "example.poll".into(),
        reason: UiUnavailableReason::Disabled,
    });
    assert_eq!(
        found(&disabled, &context()),
        Err(UiUnavailableReason::Disabled)
    );
    let mut other_platform = catalog.clone();
    other_platform.plugins[0].extensions.platforms = vec![ClientPlatform::Ios];
    assert_eq!(
        found(&other_platform, &context()),
        Err(UiUnavailableReason::UnsupportedPlatform)
    );
    let mut no_source = catalog.clone();
    no_source.plugins[0]
        .operations
        .remove("example.poll.history");
    assert_eq!(
        found(&no_source, &context()),
        Err(UiUnavailableReason::NoPermission)
    );
    let inert = ResolvedPluginView::unavailable_page(UiUnavailableReason::Disabled);
    assert!(!inert.available && inert.title.is_empty() && inert.blocks.is_empty());
}

#[test]
fn a_page_resolves_against_its_source_result_with_badges_and_paging() {
    let manifest = parse(&page_manifest()).unwrap();
    let catalog = catalog(&manifest);
    let (_, page) = catalog
        .page("example.poll", "example.poll.history", &context())
        .unwrap();
    let data = ledger(
        json!([
            {"id": "l1", "kind": "claim", "amountText": "+¥1.00", "counterpartyId": "u2",
             "packetId": "p1", "atMs": 900},
            {"id": "l2", "kind": "bonus", "amountText": "+¥2.00", "packetId": "p2"},
            {"id": "l3", "amountText": "-¥3.00", "packetId": "p3"}
        ]),
        Some("c2"),
    );
    let view = page.resolve_page(&data, "en-US", "u1").unwrap();
    assert!(view.available && view.unavailable_reason.is_none());
    assert_eq!(view.title, "History");
    assert_eq!(view.next_cursor.as_deref(), Some("c2"));
    assert!(!view.detail);
    let ResolvedUiBlock::Entries(entries) = &view.blocks[0] else {
        panic!("entries first")
    };
    assert!(!entries.more, "a page has no detail");
    assert_eq!(
        entries.items[0].badge,
        Some(ResolvedUiBadge {
            text: "Received".into(),
            tone: UiTone::Success
        }),
        "mapped, translated through texts, with its tone"
    );
    assert_eq!(
        entries.items[1].badge,
        Some(ResolvedUiBadge {
            text: "bonus".into(),
            tone: UiTone::Neutral
        }),
        "an unmapped value is shown as sent"
    );
    assert_eq!(entries.items[2].badge, None, "no value, no badge");
    let ResolvedUiBlock::Rows(rows) = &view.blocks[1] else {
        panic!("rows second")
    };
    assert_eq!(rows.next_cursor, None, "rows page with the page");
    assert_eq!(rows.rows.len(), 3);
    assert_eq!(rows.rows[0].badge.as_ref().unwrap().text, "Received");
    assert_eq!(rows.rows[0].detail_input, None);
    assert_eq!(
        rows.rows[0].actions,
        vec![ResolvedUiRowAction {
            id: "open".into(),
            input: json!({"packetId": "p1"})
        }]
    );
    // The page-level action is a page target; row actions stay in the rows.
    assert_eq!(
        view.actions
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["wallet"]
    );
    let zh = page.resolve_page(&data, "zh-CN", "u1").unwrap();
    let ResolvedUiBlock::Entries(entries) = &zh.blocks[0] else {
        panic!()
    };
    assert_eq!(entries.items[0].badge.as_ref().unwrap().text, "收到红包");

    // The next page appends rows and entries; a row already shown is dropped.
    let mut merged = view.clone();
    let next = page
        .resolve_page(
            &ledger(
                json!([
                    {"id": "l3", "amountText": "-¥3.00", "packetId": "p3"},
                    {"id": "l4", "kind": "send", "amountText": "-¥4.00", "packetId": "p4"}
                ]),
                None,
            ),
            "en-US",
            "u1",
        )
        .unwrap();
    merged.append_page(next);
    assert_eq!(merged.next_cursor, None);
    let ResolvedUiBlock::Rows(rows) = &merged.blocks[1] else {
        panic!()
    };
    assert_eq!(
        rows.rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
        ["l1", "l2", "l3", "l4"]
    );
    let ResolvedUiBlock::Entries(entries) = &merged.blocks[0] else {
        panic!()
    };
    assert_eq!(entries.items.len(), 5);
    assert_eq!(entries.items[4].badge.as_ref().unwrap().text, "Sent");

    // Only a page resolves as one.
    let card = manifest.contribution("example.poll.card").unwrap();
    assert!(card.resolve_page(&data, "en", "u1").is_none());
    // Without a source the page reads its arguments and does not page.
    let (_, wallet) = catalog
        .page("example.poll", "example.poll.wallet", &context())
        .unwrap();
    let mut static_page = wallet.clone();
    static_page.source = None;
    let view = static_page
        .resolve_page(&json!({"balanceText": "¥0", "nextCursor": "x"}), "zh", "u1")
        .unwrap();
    assert_eq!(view.fields[0].value, "¥0");
    assert_eq!(view.next_cursor, None);
}

// ---- row actions that open a page ----

/// The history page with a third page, a packet page read by `view`, that
/// each ledger row opens with its packet id.
fn row_page_manifest() -> Value {
    let mut manifest = page_manifest();
    manifest["clientExtensions"]["contributions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind": "page", "id": "example.poll.packet", "schemaVersion": 1,
            "requiredPermissions": [],
            "source": {"operation": "example.poll.view", "input": {"packetId": "/packetId"}},
            "schema": {"title": "红包", "fields": [{"label": "金额", "path": "/amountText"}]}
        }));
    let page = history(&mut manifest);
    page["schema"]["actions"].as_array_mut().unwrap().push(
        json!({"id": "packet", "label": "详情", "input": {"packetId": "/packetId"},
                     "target": {"kind": "page", "page": "example.poll.packet"}}),
    );
    page["schema"]["blocks"][1]["actions"] = json!(["open", "packet"]);
    manifest
}

#[test]
fn a_page_row_action_opens_a_page_with_the_row_as_its_arguments() {
    let manifest = parse(&row_page_manifest()).unwrap();
    let catalog = catalog(&manifest);
    let (_, page) = catalog
        .page("example.poll", "example.poll.history", &context())
        .unwrap();
    let data = ledger(
        json!([
            {"id": "l1", "kind": "claim", "amountText": "+¥1.00", "packetId": "p1"},
            {"id": "l2", "kind": "deposit", "amountText": "+¥5.00"}
        ]),
        None,
    );
    let view = page.resolve_page(&data, "zh-CN", "u1").unwrap();
    let ResolvedUiBlock::Rows(rows) = &view.blocks[1] else {
        panic!("rows second")
    };
    // The block lists the action with its target, so a host tells opening a
    // page from invoking an operation.
    let packet = rows.actions.iter().find(|a| a.id == "packet").unwrap();
    assert_eq!(
        packet.target,
        UiActionTarget::Page {
            plugin_id: None,
            page: "example.poll.packet".into()
        }
    );
    assert!(
        view.actions.iter().all(|a| a.id != "packet"),
        "the rows block owns it"
    );
    // The row binds the page's arguments.
    assert_eq!(
        rows.rows[0].actions,
        vec![
            ResolvedUiRowAction {
                id: "open".into(),
                input: json!({"packetId": "p1"})
            },
            ResolvedUiRowAction {
                id: "packet".into(),
                input: json!({"packetId": "p1"})
            }
        ]
    );
    // The catalog answers the action with its page target.
    assert_eq!(
        catalog
            .action(
                "201",
                "example.poll",
                "example.poll.history",
                "packet",
                ClientPlatform::Web,
                &Default::default(),
                true,
                1000
            )
            .unwrap(),
        UiActionTarget::Page {
            plugin_id: None,
            page: "example.poll.packet".into()
        }
    );
    // The arguments feed the packet page's source.
    let (_, packet_page) = catalog
        .page("example.poll", "example.poll.packet", &context())
        .unwrap();
    assert_eq!(
        packet_page
            .source
            .as_ref()
            .unwrap()
            .bind_input(&rows.rows[0].actions[1].input, Map::new())
            .unwrap(),
        json!({"packetId": "p1"})
    );

    let reject = |edit: &dyn Fn(&mut Value)| {
        let mut manifest = row_page_manifest();
        edit(&mut manifest);
        parse(&manifest).is_err()
    };
    // The page must be declared.
    assert!(reject(&|m| {
        let actions = history(m)["schema"]["actions"].as_array_mut().unwrap();
        actions[2]["target"] = json!({"kind": "page", "page": "example.poll.missing"});
    }));
    // A row never runs a native capability, even a declared one.
    assert!(reject(&|m| {
        m["clientExtensions"]["nativeCapabilities"] = json!(["camera"]);
        let actions = history(m)["schema"]["actions"].as_array_mut().unwrap();
        actions[2]["target"] = json!({"kind": "native", "capability": "camera"});
    }));
    let native: UiDocument = serde_json::from_value(json!({"title": "t",
        "blocks": [{"kind": "rows", "source": "x.op.list", "path": "/rows", "keyPath": "/id",
                    "columns": [{"label": "a", "path": "/a"}], "actions": ["shot"]}],
        "actions": [{"id": "shot", "label": "拍", "target": {"kind": "native", "capability": "camera"}}]
    }))
    .unwrap();
    assert!(native.validate().is_err());
}

// ---- value labels ----

#[test]
fn value_labels_are_validated() {
    let entries = |labels: Value| -> Result<(), ()> {
        let document: UiDocument = serde_json::from_value(json!({"title": "t", "blocks": [
            {"kind": "entries", "itemsPath": "/items", "valueLabels": labels}]}))
        .map_err(|_| ())?;
        document.validate().map_err(|_| ())
    };
    assert!(entries(json!({"path": "/kind", "values": {"a": {"label": "A"}}})).is_ok());
    assert!(entries(json!({"path": "kind", "values": {"a": {"label": "A"}}})).is_err());
    assert!(entries(json!({"path": "/kind", "values": {}})).is_err());
    assert!(entries(json!({"path": "/kind", "values": {"": {"label": "A"}}})).is_err());
    assert!(entries(json!({"path": "/kind", "values": {"a".repeat(65): {"label": "A"}}})).is_err());
    assert!(entries(json!({"path": "/kind", "values": {"a": {"label": " "}}})).is_err());
    let many: Map<String, Value> = (0..33)
        .map(|i| (format!("v{i}"), json!({"label": "x"})))
        .collect();
    assert!(entries(json!({"path": "/kind", "values": many})).is_err());
    assert!(
        entries(json!({"path": "/kind", "values": {"a": {"label": "A", "tone": "pink"}}})).is_err()
    );
}

// ---- choices limit ----

fn choices_doc() -> UiDocument {
    serde_json::from_value(json!({
        "title": "投票",
        "blocks": [{"kind": "choices", "optionsPath": "/options", "multiplePath": "/multiple",
                    "maxSelectionsPath": "/maxSelections", "selectedPath": "/mySelection",
                    "action": "vote"}],
        "actions": [{"id": "vote", "label": "投票",
                     "target": {"kind": "operation", "operation": "x.op.vote"}}]
    }))
    .unwrap()
}
fn choices_of(document: &UiDocument, data: &Value) -> ResolvedUiChoices {
    let contribution: flare_plugin_protocol::manifest::UiContribution =
        serde_json::from_value(json!({
            "kind": "message-content", "id": "x.op.card", "schemaVersion": 1,
            "requiredPermissions": [], "schema": document
        }))
        .unwrap();
    let view = contribution.resolve(data, "zh", "u1");
    match &view.blocks[0] {
        ResolvedUiBlock::Choices(choices) => choices.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_multiple_choice_reports_and_enforces_its_limit() {
    let document = choices_doc();
    document.validate().unwrap();
    let block = document.choices_for("vote").unwrap();
    let data = json!({"options": ["a", "b", "c", "d"], "multiple": true, "maxSelections": 2});
    assert_eq!(choices_of(&document, &data).max_selections, Some(2));
    assert!(block.selection_input(&data, &[0, 3]).is_ok());
    assert!(block.selection_input(&data, &[0, 1, 2]).is_err());
    // Capped at the option count.
    let wide = json!({"options": ["a", "b"], "multiple": true, "maxSelections": 9});
    assert_eq!(choices_of(&document, &wide).max_selections, Some(2));
    assert!(block.selection_input(&wide, &[0, 1]).is_ok());
    // Zero, missing or not a count: no limit.
    for data in [
        json!({"options": ["a", "b", "c"], "multiple": true, "maxSelections": 0}),
        json!({"options": ["a", "b", "c"], "multiple": true}),
        json!({"options": ["a", "b", "c"], "multiple": true, "maxSelections": "many"}),
    ] {
        assert_eq!(choices_of(&document, &data).max_selections, None);
        assert!(block.selection_input(&data, &[0, 1, 2]).is_ok());
    }
    // A single choice has no limit to report.
    let single = json!({"options": ["a", "b"], "multiple": false, "maxSelections": 1});
    assert_eq!(choices_of(&document, &single).max_selections, None);
    assert!(block.selection_input(&single, &[0, 1]).is_err());
    // The path is a pointer.
    let mut bad = serde_json::to_value(&document).unwrap();
    bad["blocks"][0]["maxSelectionsPath"] = json!("maxSelections");
    let bad: UiDocument = serde_json::from_value(bad).unwrap();
    assert!(bad.validate().is_err());
}

#[test]
fn durable_page_commands_and_cross_plugin_continuations_are_bounded_and_page_only() {
    let mut value = page_manifest();
    let i = page_index(&value, "example.poll.wallet");
    let action = json!({"id":"submit","label":"Submit","target":{"kind":"operation","operation":"example.poll.transfer"},"command":{"slot":"example.poll.workflow","terminalErrorCodes":["DECLINED"]},"continuation":{"pluginId":"example.other","page":"example.other.result","input":{"id":"/result/id"}}});
    value["clientExtensions"]["contributions"][i]["schema"]["actions"]
        .as_array_mut()
        .unwrap()
        .push(action.clone());
    assert!(parse(&value).is_ok());
    for patch in [
        json!({"slot":"other.workflow"}),
        json!({"slot":"example.poll.workflow","terminalErrorCodes":["bad code"]}),
    ] {
        let mut bad = value.clone();
        bad["clientExtensions"]["contributions"][i]["schema"]["actions"][1]["command"] = patch;
        assert!(parse(&bad).is_err());
    }
    let card = page_index(&value, "example.poll.card");
    value["clientExtensions"]["contributions"][card]["schema"]["actions"] = json!([action]);
    assert!(parse(&value).is_err());
}

#[test]
fn observe_page_commands_require_a_declared_query_without_a_new_intent() {
    let mut value = page_manifest();
    let i = page_index(&value, "example.poll.wallet");
    let action = json!({
        "id":"observe", "label":"Check original operation",
        "target":{"kind":"operation","operation":"example.poll.wallet"},
        "command":{"slot":"example.poll.workflow","mode":"observe"}
    });
    value["clientExtensions"]["contributions"][i]["schema"]["actions"] = json!([action]);
    assert!(
        parse(&value).is_ok(),
        "a declared read-only observation is valid"
    );
    for patch in [
        json!({"target":{"kind":"operation","operation":"example.poll.transfer"}}),
        json!({"form":["amount"]}),
        json!({"confirm":"Create a new operation"}),
        json!({"command":{"slot":"example.poll.workflow","mode":"observe","terminalErrorCodes":["DECLINED"]}}),
    ] {
        let mut bad = value.clone();
        bad["clientExtensions"]["contributions"][i]["schema"]["inputs"] =
            json!([{"id":"amount","kind":"text","label":"Amount"}]);
        let target = bad["clientExtensions"]["contributions"][i]["schema"]["actions"][0]
            .as_object_mut()
            .unwrap();
        target.extend(patch.as_object().unwrap().clone());
        assert!(
            parse(&bad).is_err(),
            "observe cannot collect/replay a new intent: {patch}"
        );
    }
}
#[test]
fn dynamic_selects_resolve_authenticated_directory_choices_and_reject_outside_values() {
    let mut value = page_manifest();
    let i = page_index(&value, "example.poll.wallet");
    value["clientExtensions"]["contributions"][i]["schema"]["inputs"] = json!([{"id":"wallet","kind":"select","label":"Wallet","required":true,"choices":[{"value":"unavailable","label":"Unavailable"}],"choicesPath":"/wallets"}]);
    value["clientExtensions"]["contributions"][i]["schema"]["actions"] = json!([{"id":"transfer","label":"Send","form":["wallet"],"target":{"kind":"operation","operation":"example.poll.transfer"}}]);
    let manifest = parse(&value).unwrap();
    let page = &manifest.client_extensions.contributions[i];
    let id = "x".repeat(128);
    let data = json!({"wallets":[{"value":id,"label":"Primary"}]});
    let view = page.resolve_page(&data, "en-US", "alice").unwrap();
    assert_eq!(view.inputs[0].choices[0].value, id);
    assert!(view.inputs[0].choices_path.is_none());
    assert!(
        page.schema
            .validate_form_over(&page.schema.actions[0], &data, &json!({"wallet":id}))
            .is_ok()
    );
    assert!(
        page.schema
            .validate_form_over(&page.schema.actions[0], &data, &json!({"wallet":"foreign"}))
            .is_err()
    );
}

#[test]
fn dynamic_selects_can_start_empty_without_a_fictitious_fallback_value() {
    let mut value = page_manifest();
    let i = page_index(&value, "example.poll.wallet");
    value["clientExtensions"]["contributions"][i]["schema"]["inputs"] = json!([{"id":"wallet","kind":"select","label":"Wallet","required":true,"choicesPath":"/wallets"}]);
    value["clientExtensions"]["contributions"][i]["schema"]["actions"] = json!([{"id":"transfer","label":"Send","form":["wallet"],"target":{"kind":"operation","operation":"example.poll.transfer"}}]);
    let manifest = parse(&value).unwrap();
    let page = &manifest.client_extensions.contributions[i];
    for data in [
        json!({}),
        json!({"wallets":[]}),
        json!({"wallets":[{"value":"foreign","label":""}]}),
    ] {
        let view = page.resolve_page(&data, "en-US", "alice").unwrap();
        assert!(view.inputs[0].choices.is_empty());
        assert!(
            page.schema
                .validate_form_over(&page.schema.actions[0], &data, &json!({"wallet":"foreign"}))
                .is_err()
        );
    }
    let data = json!({"wallets":[{"value":"primary","label":"Primary"}]});
    let view = page.resolve_page(&data, "en-US", "alice").unwrap();
    assert_eq!(view.inputs[0].choices[0].value, "primary");
    page.schema
        .validate_form_over(&page.schema.actions[0], &data, &json!({"wallet":"primary"}))
        .unwrap();
    value["clientExtensions"]["contributions"][i]["schema"]["inputs"][0]
        .as_object_mut()
        .unwrap()
        .remove("choicesPath");
    assert!(
        parse(&value).is_err(),
        "a static select still requires explicit choices"
    );
}

#[test]
fn password_fields_have_no_prefill_and_never_enter_durable_commands() {
    let mut value = page_manifest();
    let i = page_index(&value, "example.poll.wallet");
    value["clientExtensions"]["contributions"][i]["schema"]["inputs"] = json!([{"id":"password","kind":"password","label":"Payment password","required":true,"maxLength":6}]);
    value["clientExtensions"]["contributions"][i]["schema"]["actions"] = json!([{"id":"pay","label":"Pay","form":["password"],"target":{"kind":"operation","operation":"example.poll.transfer"},"continuation":{"pluginId":"example.other","page":"example.other.result","actionId":"commit","input":{"id":"/id"}}}]);
    assert!(parse(&value).is_ok());
    for (field, content) in [
        ("default", json!("123456")),
        ("defaultPath", json!("/password")),
    ] {
        let mut bad = value.clone();
        bad["clientExtensions"]["contributions"][i]["schema"]["inputs"][0][field] = content;
        assert!(parse(&bad).is_err());
    }
    let mut bad = value.clone();
    bad["clientExtensions"]["contributions"][i]["schema"]["actions"][0]["command"] =
        json!({"slot":"example.poll.workflow"});
    assert!(parse(&bad).is_err());
    value["clientExtensions"]["contributions"][i]["schema"]["actions"][0]["continuation"]["actionId"] =
        json!("bad action");
    assert!(parse(&value).is_err());
}
