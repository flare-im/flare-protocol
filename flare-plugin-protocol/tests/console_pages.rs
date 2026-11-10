//! Console page declarations: field formats, references, rows cards and
//! protected media.
use flare_plugin_protocol::{
    admin::{AdminPage, AdminReferenceSurface},
    ui::{
        ResolvedUiBlock, ResolvedUiFormat, UiDocument, UiInput, UiReference, UiRowsPresentation,
        validate_form_values,
    },
};
use serde_json::{Value, json};

fn document(value: Value) -> UiDocument {
    serde_json::from_value(value).unwrap()
}
fn page(document: Value) -> AdminPage {
    serde_json::from_value(json!({
        "id": "page", "kind": "detail", "permission": "read", "document": document
    }))
    .unwrap()
}
fn fields(field: Value, data: Value) -> flare_plugin_protocol::ui::ResolvedUiField {
    let page = page(json!({"title": "Page", "fields": [field]}));
    page.document.validate().unwrap();
    page.resolve(&data, "en-US", "staff").fields.remove(0)
}

#[test]
fn a_money_field_keeps_plain_text_and_scales_from_its_source() {
    let field = fields(
        json!({"label": "Amount", "path": "/amount",
               "format": {"kind": "money", "assetPath": "/assetId", "scalePath": "/scale"}}),
        json!({"amount": "-1205", "assetId": "cny", "scale": 2}),
    );
    assert_eq!(field.value, "-12.05");
    let Some(ResolvedUiFormat::Money(money)) = field.format else {
        panic!("money format");
    };
    assert_eq!(money.minor, "-1205");
    assert_eq!(money.scale, Some(2));
    assert_eq!(money.asset_id.as_deref(), Some("cny"));
}

#[test]
fn a_money_field_without_a_scale_leaves_the_asset_to_the_host() {
    let field = fields(
        json!({"label": "Amount", "path": "/amount",
               "format": {"kind": "money", "assetPath": "/assetId"}}),
        json!({"amount": 7, "assetId": "points"}),
    );
    assert_eq!(field.value, "7");
    let Some(ResolvedUiFormat::Money(money)) = field.format else {
        panic!("money format");
    };
    assert_eq!(money.scale, None);
    assert_eq!(money.asset_id.as_deref(), Some("points"));
}

#[test]
fn a_value_the_format_does_not_fit_stays_plain() {
    let field = fields(
        json!({"label": "Amount", "path": "/amount", "format": {"kind": "money", "scale": 2}}),
        json!({"amount": "not money"}),
    );
    assert_eq!(field.value, "not money");
    assert_eq!(field.format, None);
}

#[test]
fn enum_person_reference_and_time_formats_resolve() {
    let state = fields(
        json!({"label": "State", "path": "/state",
               "format": {"kind": "enum", "labels": {"open": "Open"}}}),
        json!({"state": "open"}),
    );
    assert_eq!(state.value, "Open");
    let person = fields(
        json!({"label": "User", "path": "/user", "format": {"kind": "person"}}),
        json!({"user": "user-1"}),
    );
    assert!(matches!(person.format, Some(ResolvedUiFormat::Person(p)) if p.user_id == "user-1"));
    let labelled = fields(
        json!({"label": "Asset", "path": "/assetId",
               "format": {"kind": "reference", "labelPath": "/assetLabel"}}),
        json!({"assetId": "cny", "assetLabel": "Yuan"}),
    );
    assert_eq!(labelled.value, "Yuan");
    let bare = fields(
        json!({"label": "Asset", "path": "/assetId",
               "format": {"kind": "reference", "labelPath": "/assetLabel"}}),
        json!({"assetId": "cny"}),
    );
    assert!(matches!(bare.format, Some(ResolvedUiFormat::Reference(r)) if !r.labelled));
    let time = fields(
        json!({"label": "At", "path": "/at", "format": {"kind": "time"}}),
        json!({"at": "1700000000000"}),
    );
    assert!(matches!(time.format, Some(ResolvedUiFormat::Time(t)) if t.at_ms == 1_700_000_000_000));
}

#[test]
fn a_format_without_a_source_or_with_two_is_refused() {
    for format in [
        json!({"kind": "money"}),
        json!({"kind": "money", "scale": 2, "scalePath": "/scale"}),
        json!({"kind": "money", "scale": 19}),
        json!({"kind": "enum", "labels": {}}),
        json!({"kind": "reference", "labelPath": "label"}),
    ] {
        let document = document(json!({
            "title": "Page",
            "fields": [{"label": "A", "path": "/a", "format": format}]
        }));
        assert!(document.validate().is_err());
    }
}

fn reference() -> Value {
    json!({
        "plugin": "flare.wallet", "operation": "flare.wallet.admin.references",
        "valuePath": "/value", "labelPath": "/label", "search": "search",
        "params": {"kind": "definitions"}, "parents": {"assetId": "asset"}
    })
}
fn input(kind: &str, reference: Value) -> Value {
    json!({"id": "pick", "kind": kind, "label": "Pick", "reference": reference})
}

#[test]
fn a_reference_replaces_the_choices_of_a_select_and_checks_shape_only() {
    let document = document(json!({
        "title": "Form", "fields": [],
        "inputs": [input("select", reference()),
                   {"id": "asset", "kind": "text", "label": "Asset"}]
    }));
    document.validate().unwrap();
    let inputs: Vec<&UiInput> = document.inputs.iter().collect();
    let values = validate_form_values(&inputs, &json!({"pick": "anything"})).unwrap();
    assert_eq!(values["pick"], "anything");
    assert!(validate_form_values(&inputs, &json!({"pick": ""})).is_err());
}

#[test]
fn a_reference_goes_on_a_select_or_options_input_with_known_parents() {
    let without_parent = document(json!({
        "title": "Form", "fields": [], "inputs": [input("select", reference())]
    }));
    assert!(without_parent.validate().is_err());
    let text = document(json!({
        "title": "Form", "fields": [],
        "inputs": [input("text", json!({"operation": "op", "valuePath": "/v", "labelPath": "/l"}))]
    }));
    assert!(text.validate().is_err());
    let reserved = document(json!({
        "title": "Form", "fields": [],
        "inputs": [input("select", json!({"operation": "op", "valuePath": "/v",
                                          "labelPath": "/l", "search": "cursor"}))]
    }));
    assert!(reserved.validate().is_err());
}

#[test]
fn a_read_takes_only_declared_inputs() {
    let reference: UiReference = serde_json::from_value(reference()).unwrap();
    let parents = json!({"assetId": "cny"});
    let parents = parents.as_object().unwrap();
    assert_eq!(
        reference
            .input(Some("ab"), Some("next"), Some("d1"), parents)
            .unwrap(),
        json!({"kind": "definitions", "assetId": "cny", "search": "ab",
               "cursor": "next", "selected": "d1"})
    );
    assert!(
        reference
            .input(None, None, None, json!({}).as_object().unwrap())
            .is_err()
    );
    assert!(
        reference
            .input(None, None, None, json!({"other": "x"}).as_object().unwrap())
            .is_err()
    );
}

#[test]
fn a_directory_that_breaks_the_contract_is_refused_whole() {
    let reference: UiReference = serde_json::from_value(reference()).unwrap();
    let page = reference
        .page(&json!({
            "items": [{"value": "a", "label": "A"}],
            "nextCursor": "n",
            "selected": {"value": "b", "label": "B"}
        }))
        .unwrap();
    assert_eq!(page.items[0].label, "A");
    assert_eq!(page.next_cursor.as_deref(), Some("n"));
    assert_eq!(page.selected.unwrap().value, "b");
    assert!(
        reference
            .page(&json!({"items": [{"value": "a"}]}))
            .is_none()
    );
    assert!(reference.page(&json!({})).is_none());
}

#[test]
fn rows_carry_their_presentation_and_column_formats() {
    let page = page(json!({
        "title": "List", "fields": [],
        "actions": [],
        "blocks": [{
            "kind": "rows", "source": "op", "path": "/items", "keyPath": "/id",
            "presentation": "cards",
            "columns": [{"label": "Amount", "path": "/amount",
                         "valueFormat": {"kind": "money", "scalePath": "/scale"}}]
        }]
    }));
    page.document.validate().unwrap();
    let view = page.resolve(
        &json!({"items": [{"id": "1", "amount": 150, "scale": 2}]}),
        "en-US",
        "staff",
    );
    let ResolvedUiBlock::Rows(rows) = &view.blocks[0] else {
        panic!("rows");
    };
    assert_eq!(rows.presentation, UiRowsPresentation::Cards);
    assert_eq!(rows.rows[0].cells[0].text, "1.50");
}

fn media_document() -> Value {
    json!({
        "title": "Review", "fields": [],
        "inputs": [{"id": "materialId", "kind": "text", "label": "Material"},
                   {"id": "reason", "kind": "text", "label": "Reason"}],
        "actions": [{"id": "read", "label": "Read",
                     "target": {"kind": "operation", "operation": "op.read"},
                     "input": {"applicationId": "/id"},
                     "form": ["materialId", "reason"]}],
        "blocks": [{"kind": "protected-media", "itemsPath": "/materials", "idPath": "/id",
                    "labelPath": "/name", "mediaTypePath": "/type", "action": "read",
                    "inputName": "materialId", "reasonInput": "reason", "urlPath": "/readUrl"}]
    })
}

#[test]
fn protected_media_lists_items_and_owns_its_read() {
    let page = page(media_document());
    page.document.validate().unwrap();
    let view = page.resolve(
        &json!({"id": "a1", "materials": [{"id": "m1", "name": "Front", "type": "image/jpeg"}]}),
        "en-US",
        "staff",
    );
    assert!(view.actions.is_empty());
    let ResolvedUiBlock::ProtectedMedia(media) = &view.blocks[0] else {
        panic!("protected media");
    };
    assert_eq!(media.items[0].label, "Front");
    assert_eq!(media.items[0].input, json!({"applicationId": "a1"}));
    assert_eq!(media.url_path, "/readUrl");
    let json = serde_json::to_value(&view.blocks[0]).unwrap();
    assert_eq!(json["kind"], "protected-media");
}

#[test]
fn protected_media_collects_only_the_item_and_the_reason() {
    let mut value = media_document();
    value["inputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "other", "kind": "text", "label": "Other"}));
    value["actions"][0]["form"] = json!(["materialId", "reason", "other"]);
    assert!(document(value).validate().is_err());
}

#[test]
fn losing_the_read_loses_the_block() {
    let mut document = document(media_document());
    document.retain_actions(&mut |_| false);
    document.validate().unwrap();
    assert!(document.blocks.is_empty());
}

#[test]
fn surfaces_name_where_a_reference_is_declared() {
    let surface: AdminReferenceSurface =
        serde_json::from_value(json!({"kind": "configuration", "field": "assets.assetId"}))
            .unwrap();
    assert_eq!(
        surface,
        AdminReferenceSurface::Configuration {
            field: "assets.assetId".into()
        }
    );
}

#[test]
fn a_staff_actor_reads_as_the_person_behind_the_prefix() {
    let field = fields(
        json!({"label": "By", "path": "/by", "format": {"kind": "person"}}),
        json!({"by": "$staff:9"}),
    );
    assert!(matches!(field.format, Some(ResolvedUiFormat::Person(p)) if p.user_id == "9"));
}

#[test]
fn options_with_choices_take_only_those_choices() {
    let input: UiInput = serde_json::from_value(json!({
        "id": "purposes", "kind": "options", "label": "Purposes", "max": 4,
        "choices": [{"value": "transfer", "label": "Transfer"}]
    }))
    .unwrap();
    let document = document(json!({"title": "Page", "fields": [], "inputs": [input]}));
    document.validate().unwrap();
    let inputs: Vec<&UiInput> = document.inputs.iter().collect();
    let values = |items: Value| validate_form_values(&inputs, &json!({"purposes": items}));
    assert!(values(json!(["transfer"])).is_ok());
    assert!(values(json!(["other"])).is_err());
}

#[test]
fn a_toggle_cannot_hold_choices() {
    let document = document(json!({"title": "Page", "fields": [], "inputs": [
        {"id": "on", "kind": "toggle", "label": "On",
         "choices": [{"value": "a", "label": "A"}]}
    ]}));
    assert!(document.validate().is_err());
}

#[test]
fn translated_hints_carry_label_description_and_option_labels() {
    use flare_plugin_protocol::admin::AdminUiHint;
    let hint: AdminUiHint = serde_json::from_value(json!({
        "group": "voting", "order": 1,
        "label": {"zh-CN": "改票规则", "en-US": "Vote change policy"},
        "description": {"zh-CN": "说明", "en-US": "Help"},
        "options": {"allow": {"zh-CN": "允许", "en-US": "Allow"}}
    }))
    .unwrap();
    assert_eq!(hint.label["en-US"], "Vote change policy");
    assert_eq!(hint.description["zh-CN"], "说明");
    assert_eq!(hint.options["allow"]["en-US"], "Allow");
    let bare: AdminUiHint = serde_json::from_value(json!({"group": "voting"})).unwrap();
    assert_eq!(
        serde_json::to_value(bare).unwrap(),
        json!({"group": "voting"})
    );
}
