//! Private file values and conditional forms share one validation contract.
use flare_plugin_protocol::{PluginManifest, ui::*};
use serde_json::{Value, json};

fn file_input() -> Value {
    json!({"id":"proofFile","kind":"file","label":"Receipt","required":true,
        "file":{"accept":["image/png","image/jpeg","application/pdf"],"maxBytes":5242880,
            "upload":{"prepareOperation":"example.poll.material_prepare",
                      "completeOperation":"example.poll.material_complete","purpose":"proof"}}})
}
fn document(inputs: Vec<Value>, form: Vec<&str>) -> UiDocument {
    serde_json::from_value(json!({"title":"Upload","inputs":inputs,
        "actions":[{"id":"submit","label":"Submit","form":form,
            "target":{"kind":"operation","operation":"example.poll.vote"}}]}))
    .unwrap()
}
fn conditional_document() -> UiDocument {
    let mut file = file_input();
    file["showWhen"] = json!({"field":"method","anyOf":["qr","alternateQr"]});
    file["defaultPath"] = json!("/oldProof");
    document(
        vec![
            json!({"id":"method","kind":"select","label":"Method","default":"bank",
        "choices":[{"value":"bank","label":"Bank"},{"value":"qr","label":"QR"},
                   {"value":"alternateQr","label":"Other QR"}]}),
            file,
        ],
        vec!["method", "proofFile"],
    )
}

#[test]
fn file_values_accept_only_one_opaque_reference() {
    let doc = document(vec![file_input()], vec!["proofFile"]);
    doc.validate().unwrap();
    assert_eq!(
        doc.validate_form(
            &doc.actions[0],
            &json!({"proofFile":{"fileId":"private_file:1"}})
        )
        .unwrap()["proofFile"],
        json!({"fileId":"private_file:1"})
    );
    for value in [
        json!("private_file:1"),
        json!({"fileId":""}),
        json!({"fileId":"https://host/file"}),
        json!({"fileId":"a/b"}),
        json!({"fileId":"id","url":"https://host"}),
        json!({"fileId":"id","bytes":[1]}),
    ] {
        assert!(
            doc.validate_form(&doc.actions[0], &json!({"proofFile":value}))
                .is_err()
        );
    }
    assert!(doc.validate_form(&doc.actions[0], &json!({})).is_err());
}

#[test]
fn file_declarations_are_bounded_exact_and_exclusive_to_file_kind() {
    for (path, value) in [
        ("/file/maxBytes", json!(0)),
        ("/file/maxBytes", json!(5242881)),
        ("/file/accept", json!([])),
        ("/file/accept", json!(["image/*"])),
        ("/file/accept", json!(["image/png; charset=utf8"])),
        ("/file/accept", json!(["image/png", "image/png"])),
        ("/file/upload/purpose", json!("../proof")),
        (
            "/file/upload/completeOperation",
            json!("example.poll.material_prepare"),
        ),
        ("/kind", json!("text")),
    ] {
        let mut file = file_input();
        *file.pointer_mut(path).unwrap() = value;
        assert!(
            document(vec![file], vec!["proofFile"]).validate().is_err(),
            "{path}"
        );
    }
    let mut file = file_input();
    file.as_object_mut().unwrap().remove("file");
    assert!(document(vec![file], vec!["proofFile"]).validate().is_err());
}

#[test]
fn hidden_inputs_are_not_required_validated_or_forwarded_but_unknown_ids_fail() {
    let doc = conditional_document();
    doc.validate().unwrap();
    let result = doc
        .validate_form(
            &doc.actions[0],
            &json!({"method":"bank","proofFile":{"bad":"old"}}),
        )
        .unwrap();
    assert_eq!(Value::Object(result), json!({"method":"bank"}));
    assert!(
        doc.validate_form(
            &doc.actions[0],
            &json!({"method":"bank","undeclared":"value"})
        )
        .is_err()
    );
    assert!(
        doc.validate_form(&doc.actions[0], &json!({"method":"qr"}))
            .is_err()
    );
    assert_eq!(
        doc.validate_form(
            &doc.actions[0],
            &json!({"method":"qr","proofFile":{"fileId":"file-1"}})
        )
        .unwrap()["proofFile"],
        json!({"fileId":"file-1"})
    );
}

#[test]
fn conditional_defaults_cannot_resurrect_a_removed_or_switched_file() {
    let mut doc = conditional_document();
    doc.inputs[1].default = Some(json!({"fileId":"declared-old"}));
    let action = &doc.actions[0];
    let data = json!({"oldProof":{"fileId":"bound-old"}});
    for method in ["qr", "alternateQr"] {
        for submitted in [
            json!({"method":method}),
            json!({"method":method,"proofFile":null}),
        ] {
            assert!(doc.validate_form_over(action, &data, &submitted).is_err());
        }
    }
    assert_eq!(
        Value::Object(doc.validate_form_over(action, &data, &json!({})).unwrap()),
        json!({"method":"bank"})
    );
    // Ordinary, unconditional defaults keep their established behavior.
    doc.inputs[1].show_when = None;
    assert_eq!(
        doc.validate_form_over(action, &data, &json!({})).unwrap()["proofFile"],
        json!({"fileId":"bound-old"})
    );
}

#[test]
fn conditions_require_an_earlier_unconditional_select_in_the_same_form() {
    let good = conditional_document();
    good.validate().unwrap();
    let mut missing = good.clone();
    missing.actions[0].form = vec!["proofFile".into()];
    assert!(missing.validate().is_err());
    let mut later = good.clone();
    later.actions[0].form.reverse();
    assert!(later.validate().is_err());
    let mut wrong_kind = good.clone();
    wrong_kind.inputs[0] =
        serde_json::from_value(json!({"id":"method","kind":"text","label":"Method"})).unwrap();
    assert!(wrong_kind.validate().is_err());
    for when in [
        json!({"field":"proofFile","anyOf":["qr"]}),
        json!({"field":"unknown","anyOf":["qr"]}),
        json!({"field":"method","anyOf":[]}),
        json!({"field":"method","anyOf":["qr","qr"]}),
    ] {
        let mut raw = serde_json::to_value(&good).unwrap();
        raw["inputs"][1]["showWhen"] = when;
        assert!(
            serde_json::from_value::<UiDocument>(raw)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut chained = good;
    let mut third = chained.inputs[1].clone();
    third.id = "another".into();
    third.show_when.as_mut().unwrap().field = "proofFile".into();
    chained.inputs.push(third);
    chained.actions[0].form.push("another".into());
    assert!(chained.validate().is_err());
}

fn upload_manifest() -> Value {
    let mut raw: Value =
        serde_json::from_slice(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap();
    for (id, mode) in [
        ("material_prepare", "command"),
        ("material_complete", "command"),
        ("material_list", "query"),
    ] {
        raw["capabilities"][0]["operations"]
            .as_array_mut()
            .unwrap()
            .push(json!({
            "id":id,"mode":mode,"idempotent":true,"timeoutMs":5000,
            "inputSchema":{"type":"object"},"outputSchema":{"type":"object"},
            "permissionIds":["example.poll.read"]}));
    }
    raw["clientExtensions"]["contributions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
        "kind":"page","id":"example.poll.upload","schemaVersion":1,
        "requiredPermissions":[],"schema":document(vec![file_input()],vec!["proofFile"])}));
    raw
}

#[test]
fn upload_commands_must_be_owned_and_declared_by_the_same_plugin() {
    let good = upload_manifest();
    let manifest = PluginManifest::from_json(&serde_json::to_vec(&good).unwrap()).unwrap();
    assert!(
        manifest
            .client_operation_refs()
            .contains("example.poll.material_prepare")
    );
    assert!(
        manifest
            .client_operation_refs()
            .contains("example.poll.material_complete")
    );
    for operation in [
        "other.plugin.prepare",
        "example.poll.unknown",
        "example.poll.material_list",
        "example.poll.end",
    ] {
        let mut raw = good.clone();
        raw["clientExtensions"]["contributions"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["schema"]["inputs"][0]["file"]["upload"]["prepareOperation"] =
            json!(operation);
        assert!(
            PluginManifest::from_json(&serde_json::to_vec(&raw).unwrap()).is_err(),
            "{operation}"
        );
    }
}

#[test]
fn file_uploads_require_page_main_root_forms_not_rows_filters_or_other_surfaces() {
    let good = upload_manifest();
    let last = good["clientExtensions"]["contributions"]
        .as_array()
        .unwrap()
        .len()
        - 1;
    for kind in ["composer-action", "message-content"] {
        let mut raw = good.clone();
        raw["clientExtensions"]["contributions"][last]["kind"] = json!(kind);
        assert!(PluginManifest::from_json(&serde_json::to_vec(&raw).unwrap()).is_err());
    }
    let mut detail = good.clone();
    detail["clientExtensions"]["contributions"][last]["detail"] = json!({
        "operation":"example.poll.details","document":document(vec![file_input()],vec!["proofFile"])});
    assert!(PluginManifest::from_json(&serde_json::to_vec(&detail).unwrap()).is_err());
    for (actions, filters) in [
        (json!(["submit"]), json!([])),
        (json!([]), json!(["proofFile"])),
    ] {
        let mut raw = good.clone();
        let page = &mut raw["clientExtensions"]["contributions"][last];
        page["source"] = json!({"operation":"example.poll.material_list"});
        page["schema"]["blocks"] = json!([{"kind":"rows","source":"example.poll.material_list","path":"/items",
            "keyPath":"/id","columns":[{"label":"ID","path":"/id"}],"actions":actions,"filters":filters}]);
        assert!(PluginManifest::from_json(&serde_json::to_vec(&raw).unwrap()).is_err());
    }
    // A rows display may coexist with a root form when file inputs are not filters.
    let mut root_form = good.clone();
    let page = &mut root_form["clientExtensions"]["contributions"][last];
    page["source"] = json!({"operation":"example.poll.material_list"});
    page["schema"]["blocks"] = json!([{"kind":"rows","source":"example.poll.material_list","path":"/items",
        "keyPath":"/id","columns":[{"label":"ID","path":"/id"}],"filters":[]}]);
    PluginManifest::from_json(&serde_json::to_vec(&root_form).unwrap()).unwrap();
    let mut admin = good;
    admin["admin"]["pages"][0]["document"]["inputs"]
        .as_array_mut()
        .unwrap()
        .push(file_input());
    assert!(PluginManifest::from_json(&serde_json::to_vec(&admin).unwrap()).is_err());
}
