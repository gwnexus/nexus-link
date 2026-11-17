use std::collections::HashMap;

use nexus_link_core::heartbeat::{
    HeartbeatPayload, HeartbeatResponse, MAX_MODEL_ID_LEN, MAX_MODELS, ModelEntry,
    apply_backing_models, is_valid_model_id, model_info_endpoint, models_endpoint,
    parse_model_info, parse_model_list, strip_provider_prefix,
};
use serde_json::json;

const QWEN: &str = "Qwen/Qwen3.6-35B-A3B-NVFP4";

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
        ModelEntry::new("nexus-review-local".into(), Some(131072), None),
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
    // An absolute local path as `root` is not reported as backing model.
    assert_eq!(
        serde_json::to_value(&models).unwrap(),
        json!([{ "id": "Qwen/Qwen3-Coder-30B", "max_model_len": 131072 }])
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

#[test]
fn test_model_entry_new_without_details_is_plain_id() {
    let e = ModelEntry::new("a".into(), None, None);
    assert_eq!(e, ModelEntry::Id("a".into()));
    // Zero length and invalid backing model count as unknown.
    let e = ModelEntry::new("a".into(), Some(0), Some("bad name".into()));
    assert_eq!(e, ModelEntry::Id("a".into()));
}

#[test]
fn test_model_entry_serializes_only_known_details() {
    let e = ModelEntry::new("a".into(), None, Some(QWEN.into()));
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({ "id": "a", "backing_model": QWEN })
    );
    let e = ModelEntry::new("a".into(), Some(8192), Some(QWEN.into()));
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        json!({ "id": "a", "max_model_len": 8192, "backing_model": QWEN })
    );
}

#[test]
fn test_parse_vllm_model_list_reports_root_as_backing_model() {
    let body = json!({ "data": [
        { "id": "coder", "owned_by": "vllm", "max_model_len": 131072, "root": QWEN }
    ]})
    .to_string();
    let models = parse_model_list(&body).unwrap();
    assert_eq!(models[0].max_model_len(), Some(131072));
    assert_eq!(models[0].backing_model(), Some(QWEN));
}

#[test]
fn test_model_info_endpoint() {
    assert_eq!(
        model_info_endpoint("http://127.0.0.1:4000"),
        "http://127.0.0.1:4000/model/info"
    );
    assert_eq!(
        model_info_endpoint("http://127.0.0.1:4000/v1/"),
        "http://127.0.0.1:4000/model/info"
    );
}

#[test]
fn test_strip_provider_prefix() {
    assert_eq!(strip_provider_prefix(&format!("hosted_vllm/{QWEN}")), QWEN);
    assert_eq!(strip_provider_prefix("openai/gpt-4o"), "gpt-4o");
    // Unknown first segment is an HF org, not a provider.
    assert_eq!(strip_provider_prefix(QWEN), QWEN);
    assert_eq!(strip_provider_prefix("gpt-4o"), "gpt-4o");
    assert_eq!(strip_provider_prefix("openai/"), "openai/");
}

#[test]
fn test_parse_model_info_maps_aliases_without_secrets() {
    let body = json!({ "data": [
        { "model_name": "nexus-coder-long", "litellm_params": {
            "model": format!("hosted_vllm/{QWEN}"),
            "api_base": "http://10.0.0.2:8000/v1", "api_key": "sk-test" } },
        { "model_name": "nexus-review-local", "litellm_params": { "model": format!("openai/{QWEN}") } },
        { "model_name": "nexus-architect-local", "litellm_params": { "model": format!("hosted_vllm/{QWEN}") } },
        { "model_name": "no-params" }
    ]})
    .to_string();
    let info = parse_model_info(&body).unwrap();
    assert_eq!(info.len(), 3);
    assert!(info.values().all(|b| b == QWEN));
    let dump = format!("{info:?}");
    assert!(!dump.contains("sk-test") && !dump.contains("10.0.0.2"));
}

#[test]
fn test_parse_model_info_disagreeing_deployments_are_unknown() {
    let body = json!({ "data": [
        { "model_name": "lb", "litellm_params": { "model": "hosted_vllm/org/one" } },
        { "model_name": "lb", "litellm_params": { "model": "hosted_vllm/org/two" } },
        { "model_name": "same", "litellm_params": { "model": "hosted_vllm/org/one" } },
        { "model_name": "same", "litellm_params": { "model": "openai/org/one" } }
    ]})
    .to_string();
    let info = parse_model_info(&body).unwrap();
    assert!(!info.contains_key("lb"));
    assert_eq!(info["same"], "org/one");
}

#[test]
fn test_parse_model_info_rejects_non_litellm_body() {
    assert!(parse_model_info(r#"{"detail":"Not Found"}"#).is_err());
    assert!(parse_model_info("not json").is_err());
}

#[test]
fn test_apply_backing_models() {
    let models = vec![
        ModelEntry::new("nexus-coder-long".into(), Some(131072), None),
        ModelEntry::new("vllm-served".into(), None, Some("org/root".into())),
        ModelEntry::Id("unknown".into()),
    ];
    let info = HashMap::from([
        ("nexus-coder-long".to_string(), QWEN.to_string()),
        ("vllm-served".to_string(), "org/other".to_string()),
    ]);
    let out = apply_backing_models(models, &info);
    assert_eq!(
        serde_json::to_value(&out).unwrap(),
        json!([
            { "id": "nexus-coder-long", "max_model_len": 131072, "backing_model": QWEN },
            { "id": "vllm-served", "backing_model": "org/root" },
            "unknown"
        ])
    );
}
