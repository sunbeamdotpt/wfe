use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::config::{ServerConfig, WebhookTrigger};

type HmacSha256 = Hmac<Sha256>;

/// Shared state for webhook handlers.
#[derive(Clone)]
pub struct WebhookState {
    pub host: Arc<wfe::WorkflowHost>,
    pub config: ServerConfig,
}

/// Generic event webhook.
///
/// POST /webhooks/events
/// Body: { "event_name": "...", "event_key": "...", "data": { ... } }
/// Requires bearer token authentication (same tokens as gRPC auth).
pub async fn handle_generic_event(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    Json(payload): Json<GenericEventPayload>,
) -> impl IntoResponse {
    // HIGH-07: Authenticate generic event endpoint.
    if !state.config.auth.tokens.is_empty() {
        let auth_header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = auth_header
            .strip_prefix("Bearer ")
            .or_else(|| auth_header.strip_prefix("bearer "))
            .unwrap_or("");
        if !crate::auth::check_static_tokens_pub(&state.config.auth.tokens, token) {
            return (StatusCode::UNAUTHORIZED, "invalid token");
        }
    }

    let data = payload.data.unwrap_or_else(|| serde_json::json!({}));

    match state
        .host
        .publish_event(&payload.event_name, &payload.event_key, data)
        .await
    {
        Ok(()) => (StatusCode::OK, "event published"),
        Err(e) => {
            tracing::warn!(error = %e, "failed to publish generic event");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to publish event")
        }
    }
}

/// GitHub webhook handler.
///
/// POST /webhooks/github
/// Verifies X-Hub-Signature-256, parses X-GitHub-Event header.
pub async fn handle_github_webhook(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. Verify HMAC signature if secret is configured.
    if let Some(secret) = state.config.auth.webhook_secrets.get("github") {
        let sig_header = headers
            .get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !verify_hmac_sha256(secret.as_bytes(), &body, sig_header) {
            return (StatusCode::UNAUTHORIZED, "invalid signature");
        }
    }

    // 2. Parse event type.
    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // 3. Parse payload.
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "invalid GitHub webhook JSON");
            return (StatusCode::BAD_REQUEST, "invalid JSON");
        }
    };

    tracing::info!(
        event = event_type,
        repo = payload["repository"]["full_name"].as_str().unwrap_or(""),
        "received GitHub webhook"
    );

    // 4. Map to WFE event + check triggers.
    let forge_event = map_forge_event(event_type, &payload);

    // Publish as event (for workflows waiting on events).
    if let Err(e) = state
        .host
        .publish_event(&forge_event.event_name, &forge_event.event_key, forge_event.data.clone())
        .await
    {
        tracing::error!(error = %e, "failed to publish forge event");
        return (StatusCode::INTERNAL_SERVER_ERROR, "failed to publish event");
    }

    // Check triggers and auto-start workflows.
    for trigger in &state.config.webhook.triggers {
        if trigger.source != "github" {
            continue;
        }
        if trigger.event != event_type {
            continue;
        }
        if let Some(ref match_ref) = trigger.match_ref {
            let payload_ref = payload["ref"].as_str().unwrap_or("");
            if payload_ref != match_ref {
                continue;
            }
        }

        let data = map_trigger_data(trigger, &payload);
        match state
            .host
            .start_workflow(&trigger.workflow_id, trigger.version, data)
            .await
        {
            Ok(id) => {
                tracing::info!(
                    workflow_id = %id,
                    trigger = %trigger.workflow_id,
                    "webhook triggered workflow"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    trigger = %trigger.workflow_id,
                    "failed to start triggered workflow"
                );
            }
        }
    }

    (StatusCode::OK, "ok")
}

/// Gitea webhook handler.
///
/// POST /webhooks/gitea
/// Verifies X-Gitea-Signature, parses X-Gitea-Event (or X-GitHub-Event) header.
/// Gitea payloads are intentionally compatible with GitHub's format.
pub async fn handle_gitea_webhook(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. Verify HMAC signature if secret is configured.
    if let Some(secret) = state.config.auth.webhook_secrets.get("gitea") {
        // Gitea uses X-Gitea-Signature (raw hex, no sha256= prefix in older versions).
        let sig_header = headers
            .get("x-gitea-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        // Handle both raw hex and sha256= prefixed formats.
        if !verify_hmac_sha256(secret.as_bytes(), &body, sig_header)
            && !verify_hmac_sha256_raw(secret.as_bytes(), &body, sig_header)
        {
            return (StatusCode::UNAUTHORIZED, "invalid signature");
        }
    }

    // 2. Parse event type (try Gitea header first, fall back to GitHub compat header).
    let event_type = headers
        .get("x-gitea-event")
        .or_else(|| headers.get("x-github-event"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // 3. Parse payload.
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "invalid Gitea webhook JSON");
            return (StatusCode::BAD_REQUEST, "invalid JSON");
        }
    };

    tracing::info!(
        event = event_type,
        repo = payload["repository"]["full_name"].as_str().unwrap_or(""),
        "received Gitea webhook"
    );

    // 4. Map to WFE event + check triggers (same logic as GitHub).
    let forge_event = map_forge_event(event_type, &payload);

    if let Err(e) = state
        .host
        .publish_event(&forge_event.event_name, &forge_event.event_key, forge_event.data.clone())
        .await
    {
        tracing::error!(error = %e, "failed to publish forge event");
        return (StatusCode::INTERNAL_SERVER_ERROR, "failed to publish event");
    }

    for trigger in &state.config.webhook.triggers {
        if trigger.source != "gitea" {
            continue;
        }
        if trigger.event != event_type {
            continue;
        }
        if let Some(ref match_ref) = trigger.match_ref {
            let payload_ref = payload["ref"].as_str().unwrap_or("");
            if payload_ref != match_ref {
                continue;
            }
        }

        let data = map_trigger_data(trigger, &payload);
        match state
            .host
            .start_workflow(&trigger.workflow_id, trigger.version, data)
            .await
        {
            Ok(id) => {
                tracing::info!(workflow_id = %id, trigger = %trigger.workflow_id, "webhook triggered workflow");
            }
            Err(e) => {
                tracing::warn!(error = %e, trigger = %trigger.workflow_id, "failed to start triggered workflow");
            }
        }
    }

    (StatusCode::OK, "ok")
}

/// Health check endpoint.
pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

// ── Types ───────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct GenericEventPayload {
    pub event_name: String,
    pub event_key: String,
    pub data: Option<serde_json::Value>,
}

struct ForgeEvent {
    event_name: String,
    event_key: String,
    data: serde_json::Value,
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Verify HMAC-SHA256 signature with `sha256=<hex>` prefix (GitHub format).
fn verify_hmac_sha256(secret: &[u8], body: &[u8], signature: &str) -> bool {
    let hex_sig = signature.strip_prefix("sha256=").unwrap_or("");
    if hex_sig.is_empty() {
        return false;
    }
    let expected = match hex::decode(hex_sig) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

/// Verify HMAC-SHA256 signature as raw hex (no prefix, Gitea legacy format).
fn verify_hmac_sha256_raw(secret: &[u8], body: &[u8], signature: &str) -> bool {
    let expected = match hex::decode(signature) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

/// Map a git forge event type + payload to a WFE event.
fn map_forge_event(event_type: &str, payload: &serde_json::Value) -> ForgeEvent {
    let repo = payload["repository"]["full_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    match event_type {
        "push" => {
            let git_ref = payload["ref"].as_str().unwrap_or("").to_string();
            ForgeEvent {
                event_name: "git.push".to_string(),
                event_key: format!("{repo}/{git_ref}"),
                data: serde_json::json!({
                    "repo": repo,
                    "ref": git_ref,
                    "before": payload["before"].as_str().unwrap_or(""),
                    "after": payload["after"].as_str().unwrap_or(""),
                    "commit": payload["head_commit"]["id"].as_str().unwrap_or(""),
                    "message": payload["head_commit"]["message"].as_str().unwrap_or(""),
                    "sender": payload["sender"]["login"].as_str().unwrap_or(""),
                }),
            }
        }
        "pull_request" => {
            let number = payload["number"].as_u64().unwrap_or(0);
            ForgeEvent {
                event_name: "git.pr".to_string(),
                event_key: format!("{repo}/{number}"),
                data: serde_json::json!({
                    "repo": repo,
                    "action": payload["action"].as_str().unwrap_or(""),
                    "number": number,
                    "title": payload["pull_request"]["title"].as_str().unwrap_or(""),
                    "head_ref": payload["pull_request"]["head"]["ref"].as_str().unwrap_or(""),
                    "base_ref": payload["pull_request"]["base"]["ref"].as_str().unwrap_or(""),
                    "sender": payload["sender"]["login"].as_str().unwrap_or(""),
                }),
            }
        }
        "create" => {
            let ref_name = payload["ref"].as_str().unwrap_or("").to_string();
            let ref_type = payload["ref_type"].as_str().unwrap_or("").to_string();
            ForgeEvent {
                event_name: format!("git.{ref_type}"),
                event_key: format!("{repo}/{ref_name}"),
                data: serde_json::json!({
                    "repo": repo,
                    "ref": ref_name,
                    "ref_type": ref_type,
                    "sender": payload["sender"]["login"].as_str().unwrap_or(""),
                }),
            }
        }
        _ => ForgeEvent {
            event_name: format!("git.{event_type}"),
            event_key: repo.clone(),
            data: serde_json::json!({
                "repo": repo,
                "event_type": event_type,
            }),
        },
    }
}

/// Extract data fields from payload using simple JSONPath-like mapping.
/// Supports `$.field.nested` syntax.
fn map_trigger_data(
    trigger: &WebhookTrigger,
    payload: &serde_json::Value,
) -> serde_json::Value {
    let mut data = serde_json::Map::new();
    for (key, path) in &trigger.data_mapping {
        if let Some(value) = resolve_json_path(payload, path) {
            data.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(data)
}

/// Resolve a simple JSONPath expression like `$.repository.full_name`.
fn resolve_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_github_hmac_valid() {
        let secret = b"mysecret";
        let body = b"hello world";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_hmac_sha256(secret, body, &sig));
    }

    #[test]
    fn verify_github_hmac_invalid() {
        assert!(!verify_hmac_sha256(b"secret", b"body", "sha256=deadbeef"));
    }

    #[test]
    fn verify_github_hmac_missing_prefix() {
        assert!(!verify_hmac_sha256(b"secret", b"body", "not-a-signature"));
    }

    #[test]
    fn verify_gitea_hmac_raw_valid() {
        let secret = b"giteasecret";
        let body = b"payload";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        assert!(verify_hmac_sha256_raw(secret, body, &sig));
    }

    #[test]
    fn verify_gitea_hmac_raw_invalid() {
        assert!(!verify_hmac_sha256_raw(b"secret", b"body", "badhex"));
    }

    #[test]
    fn map_push_event() {
        let payload = serde_json::json!({
            "ref": "refs/heads/main",
            "before": "aaa",
            "after": "bbb",
            "head_commit": { "id": "bbb", "message": "fix: stuff" },
            "repository": { "full_name": "studio/wfe" },
            "sender": { "login": "sienna" }
        });
        let event = map_forge_event("push", &payload);
        assert_eq!(event.event_name, "git.push");
        assert_eq!(event.event_key, "studio/wfe/refs/heads/main");
        assert_eq!(event.data["commit"], "bbb");
        assert_eq!(event.data["sender"], "sienna");
    }

    #[test]
    fn map_pull_request_event() {
        let payload = serde_json::json!({
            "action": "opened",
            "number": 42,
            "pull_request": {
                "title": "Add feature",
                "head": { "ref": "feature-branch" },
                "base": { "ref": "main" }
            },
            "repository": { "full_name": "studio/wfe" },
            "sender": { "login": "sienna" }
        });
        let event = map_forge_event("pull_request", &payload);
        assert_eq!(event.event_name, "git.pr");
        assert_eq!(event.event_key, "studio/wfe/42");
        assert_eq!(event.data["action"], "opened");
        assert_eq!(event.data["title"], "Add feature");
        assert_eq!(event.data["head_ref"], "feature-branch");
    }

    #[test]
    fn map_create_tag_event() {
        let payload = serde_json::json!({
            "ref": "v1.5.0",
            "ref_type": "tag",
            "repository": { "full_name": "studio/wfe" },
            "sender": { "login": "sienna" }
        });
        let event = map_forge_event("create", &payload);
        assert_eq!(event.event_name, "git.tag");
        assert_eq!(event.event_key, "studio/wfe/v1.5.0");
    }

    #[test]
    fn map_create_branch_event() {
        let payload = serde_json::json!({
            "ref": "feature-x",
            "ref_type": "branch",
            "repository": { "full_name": "studio/wfe" },
            "sender": { "login": "sienna" }
        });
        let event = map_forge_event("create", &payload);
        assert_eq!(event.event_name, "git.branch");
        assert_eq!(event.event_key, "studio/wfe/feature-x");
    }

    #[test]
    fn map_unknown_event() {
        let payload = serde_json::json!({
            "repository": { "full_name": "studio/wfe" }
        });
        let event = map_forge_event("release", &payload);
        assert_eq!(event.event_name, "git.release");
        assert_eq!(event.event_key, "studio/wfe");
    }

    #[test]
    fn resolve_json_path_simple() {
        let v = serde_json::json!({"a": {"b": {"c": "value"}}});
        assert_eq!(resolve_json_path(&v, "$.a.b.c").unwrap(), "value");
    }

    #[test]
    fn resolve_json_path_no_prefix() {
        let v = serde_json::json!({"repo": "test"});
        assert_eq!(resolve_json_path(&v, "repo").unwrap(), "test");
    }

    #[test]
    fn resolve_json_path_missing() {
        let v = serde_json::json!({"a": 1});
        assert!(resolve_json_path(&v, "$.b.c").is_none());
    }

    #[test]
    fn map_trigger_data_extracts_fields() {
        let trigger = WebhookTrigger {
            source: "github".to_string(),
            event: "push".to_string(),
            match_ref: None,
            workflow_id: "ci".to_string(),
            version: 1,
            data_mapping: [
                ("repo".to_string(), "$.repository.full_name".to_string()),
                ("commit".to_string(), "$.head_commit.id".to_string()),
            ]
            .into(),
        };
        let payload = serde_json::json!({
            "repository": { "full_name": "studio/wfe" },
            "head_commit": { "id": "abc123" }
        });
        let data = map_trigger_data(&trigger, &payload);
        assert_eq!(data["repo"], "studio/wfe");
        assert_eq!(data["commit"], "abc123");
    }

    #[test]
    fn map_trigger_data_missing_field_skipped() {
        let trigger = WebhookTrigger {
            source: "github".to_string(),
            event: "push".to_string(),
            match_ref: None,
            workflow_id: "ci".to_string(),
            version: 1,
            data_mapping: [("missing".to_string(), "$.nonexistent.field".to_string())].into(),
        };
        let payload = serde_json::json!({"repo": "test"});
        let data = map_trigger_data(&trigger, &payload);
        assert!(data.get("missing").is_none());
    }
}
