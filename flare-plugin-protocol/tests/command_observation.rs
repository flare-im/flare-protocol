use flare_plugin_protocol::command::{
    CommandObservation, CommandObservationRequest, ObservedCommandResult,
};
use serde_json::{Value, json};

#[test]
fn observation_request_and_result_keep_the_original_opaque_payload_and_rejection() {
    let raw = json!({"commandId":"original", "capabilityId":"example.removed.write", "payload":{"decimal":"1.000000000000000001","optional":null}});
    let request: CommandObservationRequest = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), raw);
    let receipt = json!({"commandId":"original","capabilityId":"example.removed.write","requestDigest":"sha256:opaque","outcome":"committed","result":{"success":false,"data":{"errorCode":"DECLINED"}}});
    let observed: CommandObservation<ObservedCommandResult> =
        serde_json::from_value(receipt.clone()).unwrap();
    assert!(!observed.result.as_ref().unwrap().success);
    assert_eq!(serde_json::to_value(observed).unwrap(), receipt);
    let mut unexpected = raw;
    unexpected["replacement"] = Value::Bool(true);
    assert!(serde_json::from_value::<CommandObservationRequest>(unexpected).is_err());
}
