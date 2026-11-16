use nexus_link_core::heartbeat::{
    HeartbeatPayload, HeartbeatResponse, MAX_MODEL_ID_LEN, MAX_MODELS, ModelEntry,
    is_valid_model_id, models_endpoint, parse_model_list,
};
use serde_json::json;

#[test]
fn test_payload_without_models_omits_field() {
    // Runtime unreachable: `models` must be absent, not an empty list.
    let body = serde_json::to_value(HeartbeatPayload::online(None)).unwrap();
    assert_eq!(body, json!({ "status": "online" }));
}

#[test]
fn test_payload_with_empty_models_sends_empty_list() {
    // Runtime reachable but serves nothing: explicit empty list.
    let body = serde_json::to_value(HeartbeatPayload::online(Some(vec![]))).unwrap();
    assert_eq!(body, json!({ "status": "online", "models": [] }));
}

#[test]
fn test_payload_serializes_hub_format() {
    let payload = HeartbeatPayload::online(Some(vec![
        ModelEntry::Id("nexus-coder-long".into()),
        ModelEntry::Detailed {
            id: "nexus-review-local".into(),
            max_model_len: 131072,
        },
    ]));
    let body = serde_json::to_value(payload).unwrap();
    assert_eq!(
        body,
        json!({
            "status": "online",
            "models": [
                "nexus-coder-long",
                { "id": "nexus-review-local", "max_model_len": 131072 }
            ]
        })
    );
}

#[test]
fn test_parse_litellm_model_list() {
    // LiteLLM: aliases only, no context window.
    let body = r#"{"object":"list","data":[
        {"id":"nexus-coder-long","object":"model","created":1,"owned_by":"openai"},
        {"id":"nexus-embed","object":"model","created":1,"owned_by":"openai"}
    ]}"#;
    let models = parse_model_list(body).unwrap();
    assert_eq!(
        models,
        vec![
            ModelEntry::Id("nexus-coder-long".into()),
            ModelEntry::Id("nexus-embed".into()),
        ]
    );
}

#[test]
fn test_parse_vllm_model_list_with_max_model_len() {
    let body = r#"{"object":"list","data":[
        {"id":"Qwen/Qwen3-Coder-30B","object":"model","owned_by":"vllm","max_model_len":131072,"root":"/models/qwen"}
    ]}"#;
    let models = parse_model_list(body).unwrap();
    assert_eq!(
        models,
        vec![ModelEntry::Detailed {
            id: "Qwen/Qwen3-Coder-30B".into(),
            max_model_len: 131072
        }]
    );
}

#[test]
fn test_parse_drops_invalid_and_duplicate_ids() {
    let long_id = "a".repeat(MAX_MODEL_ID_LEN + 1);
    let body = json!({
        "data": [
            { "id": "good-1" },
            { "id": "has space" },
            { "id": "" },
            { "id": long_id },
            { "id": "good-1", "max_model_len": 4096 },
            { "id": "ok.v2:latest@x+y" }
        ]
    })
    .to_string();
    let models = parse_model_list(&body).unwrap();
    assert_eq!(
        models,
        vec![
            ModelEntry::Id("good-1".into()),
            ModelEntry::Id("ok.v2:latest@x+y".into()),
        ]
    );
}

#[test]
fn test_parse_caps_at_max_models() {
    let data: Vec<_> = (0..MAX_MODELS + 10)
        .map(|i| json!({ "id": format!("m-{i}") }))
        .collect();
    let body = json!({ "data": data }).to_string();
    let models = parse_model_list(&body).unwrap();
    assert_eq!(models.len(), MAX_MODELS);
    assert_eq!(models[0].id(), "m-0");
}

#[test]
fn test_parse_zero_max_model_len_is_plain_id() {
    let models = parse_model_list(r#"{"data":[{"id":"m","max_model_len":0}]}"#).unwrap();
    assert_eq!(models, vec![ModelEntry::Id("m".into())]);
}

#[test]
fn test_parse_rejects_non_openai_body() {
    assert!(parse_model_list(r#"{"error":"unauthorized"}"#).is_err());
    assert!(parse_model_list("not json").is_err());
}

#[test]
fn test_parse_empty_list() {
    assert_eq!(parse_model_list(r#"{"data":[]}"#).unwrap(), vec![]);
}

#[test]
fn test_is_valid_model_id() {
    assert!(is_valid_model_id("nexus-coder-long"));
    assert!(is_valid_model_id("org/model_v1.5:q4@rev+1"));
    assert!(is_valid_model_id(&"a".repeat(MAX_MODEL_ID_LEN)));
    assert!(!is_valid_model_id(""));
    assert!(!is_valid_model_id("with space"));
    assert!(!is_valid_model_id("http://host?x=1"));
    assert!(!is_valid_model_id("ümlaut"));
    assert!(!is_valid_model_id(&"a".repeat(MAX_MODEL_ID_LEN + 1)));
}

#[test]
fn test_models_endpoint() {
    assert_eq!(
        models_endpoint("http://127.0.0.1:4000"),
        "http://127.0.0.1:4000/v1/models"
    );
    assert_eq!(
        models_endpoint("http://127.0.0.1:4000/"),
        "http://127.0.0.1:4000/v1/models"
    );
    assert_eq!(
        models_endpoint("http://127.0.0.1:8000/v1"),
        "http://127.0.0.1:8000/v1/models"
    );
    assert_eq!(
        models_endpoint("http://127.0.0.1:8000/v1/"),
        "http://127.0.0.1:8000/v1/models"
    );
}

#[test]
fn test_heartbeat_response_models_recorded() {
    let r: HeartbeatResponse = serde_json::from_str(r#"{"ok":true,"models_recorded":2}"#).unwrap();
    assert_eq!(r.models_recorded, Some(2));
    let r: HeartbeatResponse = serde_json::from_str(r#"{"ok":true}"#).unwrap();
    assert_eq!(r.models_recorded, None);
}
