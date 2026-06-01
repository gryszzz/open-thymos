//! End-to-end integration tests for the Thymos HTTP server.
//!
//! Uses `axum_test::TestServer` to spin up the full app stack in-process
//! and exercise the API endpoints.

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use axum_test::TestServer;
use serde_json::{json, Value};

use thymos_server::{app, auth, default_runtime, middleware, AppState};

fn test_state() -> Arc<AppState> {
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    Arc::new(AppState {
        runtime_mode: thymos_server::RuntimeMode::Reference,
        cors_allowed_origins: None,
        max_concurrent_runs_per_tenant: thymos_server::MAX_CONCURRENT_RUNS_PER_TENANT,
        max_concurrent_runs_global: thymos_server::MAX_CONCURRENT_RUNS_GLOBAL,
        runtime: default_runtime(),
        runs: Mutex::new(HashMap::new()),
        event_channels: Mutex::new(HashMap::new()),
        cognition_channels: Mutex::new(HashMap::new()),
        execution_sessions: Mutex::new(HashMap::new()),
        execution_channels: Mutex::new(HashMap::new()),
        gateway: None,
        jwt_config: None,
        pending_approvals: Mutex::new(HashMap::new()),
        cancellation_tokens: Mutex::new(HashMap::new()),
        run_store: None,
        shutdown_tx,
        active_runs: AtomicU32::new(0),
        marketplace: Arc::new(thymos_marketplace::MarketplaceService::in_memory()),
        default_cognition: thymos_server::CognitionConfig::default(),
    })
}

fn test_server(state: Arc<AppState>) -> TestServer {
    TestServer::new(app(state))
}

#[tokio::test]
async fn health_check() {
    let server = test_server(test_state());
    let resp = server.get("/health").await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn ready_check() {
    let server = test_server(test_state());
    let resp = server.get("/ready").await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["status"], "ready");
    assert_eq!(body["checks"]["run_store"], false);
}

#[tokio::test]
async fn create_run_returns_accepted() {
    let server = test_server(test_state());
    let resp = server
        .post("/runs")
        .json(&json!({
            "task": "say hello",
            "max_steps": 2
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
    let body: Value = resp.json();
    assert_eq!(body["status"], "running");
    assert!(body["run_id"].is_string());
    assert_eq!(body["task"], "say hello");
}

#[tokio::test]
async fn get_run_returns_status() {
    let state = test_state();
    let server = test_server(state.clone());

    // Create a run.
    let resp = server
        .post("/runs")
        .json(&json!({ "task": "test task" }))
        .await;
    let body: Value = resp.json();
    let run_id = body["run_id"].as_str().unwrap().to_string();

    // Poll the run (it may still be running or already completed).
    // Give the mock cognition a moment to finish.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let resp = server.get(&format!("/runs/{run_id}")).await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["task"], "test task");
    // Status should be either "running" or "completed".
    let status = body["status"].as_str().unwrap();
    assert!(status == "running" || status == "completed");
}

#[tokio::test]
async fn get_nonexistent_run_returns_404() {
    let server = test_server(test_state());
    let resp = server.get("/runs/nonexistent-id").await;
    resp.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn revoke_and_restore_writ_endpoints() {
    let server = test_server(test_state());
    let writ_hex = "0".repeat(64); // valid 32-byte hex writ id

    let resp = server.post(&format!("/writs/{writ_hex}/revoke")).await;
    resp.assert_status_ok();
    assert_eq!(resp.json::<Value>()["revoked"], true);

    let resp = server.post(&format!("/writs/{writ_hex}/restore")).await;
    resp.assert_status_ok();
    assert_eq!(resp.json::<Value>()["restored"], true);
}

#[tokio::test]
async fn revoke_writ_invalid_id_returns_400() {
    let server = test_server(test_state());
    let resp = server.post("/writs/not-hex/revoke").await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn routed_submit_commits_and_records_routing_evidence() {
    let server = test_server(test_state());
    let resp = server
        .post("/routed-submit")
        .json(&json!({
            "tool": "kv_set",
            "args": { "key": "k", "value": "v" },
            "rationale": "wisepick route",
            "routing_evidence": {
                "decision_hash": "abc123",
                "selected": "anthropic:claude",
                "alternatives": ["openai:gpt"],
                "confidence_bps": 9500,
                "reason_codes": ["cost_optimal"],
                "latency_estimate_ms": 800,
                "cost_estimate_millicents": 4200
            }
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["status"], "committed");
    assert_eq!(body["routing_evidence_recorded"], true);
    assert!(body["commit_id"].is_string());
}

#[tokio::test]
async fn routed_submit_unknown_tool_is_rejected_not_executed() {
    let server = test_server(test_state());
    let resp = server
        .post("/routed-submit")
        .json(&json!({ "tool": "ghost_tool", "args": {} }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["status"], "rejected");
}

#[tokio::test]
async fn create_run_with_cognition_config() {
    let server = test_server(test_state());
    let resp = server
        .post("/runs")
        .json(&json!({
            "task": "test with mock",
            "cognition": {
                "provider": "mock"
            }
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
    let body: Value = resp.json();
    assert_eq!(body["status"], "running");
}

#[tokio::test]
async fn run_completes_with_mock_cognition() {
    let state = test_state();
    let server = test_server(state.clone());

    let resp = server
        .post("/runs")
        .json(&json!({
            "task": "mock run",
            "cognition": { "provider": "mock" }
        }))
        .await;
    let body: Value = resp.json();
    let run_id = body["run_id"].as_str().unwrap().to_string();

    // Wait for the mock cognition to complete (should be near-instant).
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let resp = server.get(&format!("/runs/{run_id}")).await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["status"], "completed");
    assert!(body["summary"].is_object());
    assert_eq!(body["summary"]["terminated_by"], "CognitionDone");
}

#[tokio::test]
async fn usage_endpoint_works_without_gateway() {
    let server = test_server(test_state());
    let resp = server.get("/usage").await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["message"], "API gateway not configured");
}

#[tokio::test]
async fn approval_on_nonexistent_run_returns_404() {
    let server = test_server(test_state());
    let resp = server
        .post("/runs/nonexistent/approvals/test-channel")
        .json(&json!({ "approve": true }))
        .await;
    resp.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn world_on_nonexistent_run_returns_404() {
    let server = test_server(test_state());
    let resp = server.get("/runs/nonexistent/world").await;
    resp.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn world_on_completed_run_returns_projection() {
    let state = test_state();
    let server = test_server(state);

    let resp = server
        .post("/runs")
        .json(&json!({
            "task": "world projection test",
            "cognition": { "provider": "mock" }
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
    let body: Value = resp.json();
    let run_id = body["run_id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let resp = server.get(&format!("/runs/{run_id}/world")).await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert!(
        body["resources"].is_array(),
        "unexpected world body: {body}"
    );
}

#[tokio::test]
async fn replay_on_completed_run_verifies_ledger() {
    let state = test_state();
    let server = test_server(state);

    let resp = server
        .post("/runs")
        .json(&json!({
            "task": "replay projection test",
            "cognition": { "provider": "mock" }
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
    let body: Value = resp.json();
    let run_id = body["run_id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let resp = server.get(&format!("/runs/{run_id}/replay")).await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["run_id"], run_id);
    assert!(body["trajectory_id"].is_string());
    assert!(body["entries_seen"].as_u64().unwrap() >= 1);
    assert!(body["head_seq"].as_u64().is_some());
    assert!(body["final_world_hash"].is_string());
    assert!(body["tool_calls"].is_array());
}

#[tokio::test]
async fn create_run_with_tenant_headers() {
    let server = test_server(test_state());
    let resp = server
        .post("/runs")
        .add_header(
            axum::http::HeaderName::from_static("x-thymos-tenant-id"),
            axum::http::HeaderValue::from_static("tenant-abc"),
        )
        .add_header(
            axum::http::HeaderName::from_static("x-thymos-user-id"),
            axum::http::HeaderValue::from_static("user-123"),
        )
        .json(&json!({
            "task": "tenant-scoped run",
            "cognition": { "provider": "mock" }
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);

    let body: Value = resp.json();
    let run_id = body["run_id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Must pass matching tenant header to access the run.
    let resp = server
        .get(&format!("/runs/{run_id}"))
        .add_header(
            axum::http::HeaderName::from_static("x-thymos-tenant-id"),
            axum::http::HeaderValue::from_static("tenant-abc"),
        )
        .await;
    let body: Value = resp.json();
    assert_eq!(body["status"], "completed");

    // Without tenant header, should get 404 (isolation).
    let resp = server.get(&format!("/runs/{run_id}")).await;
    resp.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn audit_entries_returns_json() {
    let state = test_state();
    let server = test_server(state.clone());

    // Create and wait for a run so there are ledger entries.
    let resp = server
        .post("/runs")
        .json(&json!({
            "task": "audit test",
            "cognition": { "provider": "mock" }
        }))
        .await;
    let body: Value = resp.json();
    let _run_id = body["run_id"].as_str().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Query all audit entries (no filters).
    let resp = server.get("/audit/entries").await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert!(body["count"].as_u64().unwrap() > 0);
    assert!(body["entries"].is_array());
}

#[tokio::test]
async fn audit_entries_csv_format() {
    let state = test_state();
    let server = test_server(state.clone());

    server
        .post("/runs")
        .json(&json!({
            "task": "csv test",
            "cognition": { "provider": "mock" }
        }))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let resp = server.get("/audit/entries?format=csv").await;
    resp.assert_status_ok();
    let text = resp.text();
    assert!(text.starts_with("id,trajectory_id,seq,kind,created_at,payload\n"));
}

#[tokio::test]
async fn audit_entries_filter_by_kind() {
    let state = test_state();
    let server = test_server(state.clone());

    server
        .post("/runs")
        .json(&json!({
            "task": "kind filter test",
            "cognition": { "provider": "mock" }
        }))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Only roots.
    let resp = server.get("/audit/entries?kind=root").await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let entries = body["entries"].as_array().unwrap();
    for entry in entries {
        assert_eq!(entry["kind"], "root");
    }
}

#[tokio::test]
async fn audit_count_endpoint() {
    let state = test_state();
    let server = test_server(state.clone());

    server
        .post("/runs")
        .json(&json!({
            "task": "count test",
            "cognition": { "provider": "mock" }
        }))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let resp = server.get("/audit/entries/count").await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert!(body["count"].as_u64().unwrap() > 0);
}

// ---- Auth E2E tests ----

fn jwt_test_state() -> Arc<AppState> {
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let jwt_config = Arc::new(auth::JwtConfig::from_secret(b"test-secret-key"));
    Arc::new(AppState {
        runtime_mode: thymos_server::RuntimeMode::Reference,
        cors_allowed_origins: None,
        max_concurrent_runs_per_tenant: thymos_server::MAX_CONCURRENT_RUNS_PER_TENANT,
        max_concurrent_runs_global: thymos_server::MAX_CONCURRENT_RUNS_GLOBAL,
        runtime: default_runtime(),
        runs: Mutex::new(HashMap::new()),
        event_channels: Mutex::new(HashMap::new()),
        cognition_channels: Mutex::new(HashMap::new()),
        execution_sessions: Mutex::new(HashMap::new()),
        execution_channels: Mutex::new(HashMap::new()),
        gateway: None,
        jwt_config: Some(jwt_config),
        pending_approvals: Mutex::new(HashMap::new()),
        cancellation_tokens: Mutex::new(HashMap::new()),
        run_store: None,
        shutdown_tx,
        active_runs: AtomicU32::new(0),
        marketplace: Arc::new(thymos_marketplace::MarketplaceService::in_memory()),
        default_cognition: thymos_server::CognitionConfig::default(),
    })
}

fn gateway_test_state() -> Arc<AppState> {
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    let mut gw = middleware::ApiGateway::new();
    gw.add_key(middleware::ApiKey {
        key: "test-key-123".into(),
        tenant_id: "tenant-test".into(),
        name: "TestKey".into(),
        rate_limit_rpm: 10,
    })
    .unwrap();
    Arc::new(AppState {
        runtime_mode: thymos_server::RuntimeMode::Reference,
        cors_allowed_origins: None,
        max_concurrent_runs_per_tenant: thymos_server::MAX_CONCURRENT_RUNS_PER_TENANT,
        max_concurrent_runs_global: thymos_server::MAX_CONCURRENT_RUNS_GLOBAL,
        runtime: default_runtime(),
        runs: Mutex::new(HashMap::new()),
        event_channels: Mutex::new(HashMap::new()),
        cognition_channels: Mutex::new(HashMap::new()),
        execution_sessions: Mutex::new(HashMap::new()),
        execution_channels: Mutex::new(HashMap::new()),
        gateway: Some(Arc::new(gw)),
        jwt_config: None,
        pending_approvals: Mutex::new(HashMap::new()),
        cancellation_tokens: Mutex::new(HashMap::new()),
        run_store: None,
        shutdown_tx,
        active_runs: AtomicU32::new(0),
        marketplace: Arc::new(thymos_marketplace::MarketplaceService::in_memory()),
        default_cognition: thymos_server::CognitionConfig::default(),
    })
}

#[tokio::test]
async fn jwt_rejects_unauthenticated_request() {
    let state = jwt_test_state();
    let server = test_server(state);

    // No token → 401
    let resp = server.post("/runs").json(&json!({ "task": "test" })).await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_allows_valid_token() {
    let state = jwt_test_state();
    let server = test_server(state);

    // Create a valid JWT.
    let claims = json!({
        "sub": "user-1",
        "tenant_id": "tenant-jwt",
        "exp": 9999999999u64,
        "iat": 1000000000u64,
    });
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let key = jsonwebtoken::EncodingKey::from_secret(b"test-secret-key");
    let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();

    let resp = server
        .post("/runs")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        )
        .json(&json!({
            "task": "jwt test run",
            "cognition": { "provider": "mock" }
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
}

#[tokio::test]
async fn jwt_rejects_expired_token() {
    let state = jwt_test_state();
    let server = test_server(state);

    let claims = json!({
        "sub": "user-1",
        "exp": 1000000000u64,  // expired
        "iat": 999999999u64,
    });
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let key = jsonwebtoken::EncodingKey::from_secret(b"test-secret-key");
    let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();

    let resp = server
        .post("/runs")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        )
        .json(&json!({ "task": "should fail" }))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_health_bypasses_auth() {
    let state = jwt_test_state();
    let server = test_server(state);

    // Health should work without any token.
    let resp = server.get("/health").await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn gateway_rejects_missing_key() {
    let state = gateway_test_state();
    let server = test_server(state);

    let resp = server.post("/runs").json(&json!({ "task": "test" })).await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_allows_valid_key() {
    let state = gateway_test_state();
    let server = test_server(state);

    let resp = server
        .post("/runs")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer test-key-123"),
        )
        .json(&json!({
            "task": "gateway test",
            "cognition": { "provider": "mock" }
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
}

#[tokio::test]
async fn gateway_rejects_invalid_key() {
    let state = gateway_test_state();
    let server = test_server(state);

    let resp = server
        .post("/runs")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer wrong-key"),
        )
        .json(&json!({ "task": "test" }))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_health_bypasses_auth() {
    let state = gateway_test_state();
    let server = test_server(state);

    let resp = server.get("/health").await;
    resp.assert_status_ok();
}

fn bearer(claims: serde_json::Value) -> String {
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let key = jsonwebtoken::EncodingKey::from_secret(b"test-secret-key");
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

#[tokio::test]
async fn revoke_requires_admin_role() {
    let server = test_server(jwt_test_state());
    let writ = "0".repeat(64);

    // Authenticated but NOT admin → 403.
    let tok = bearer(json!({ "sub": "u", "exp": 9999999999u64, "iat": 1000000000u64 }));
    let resp = server
        .post(&format!("/writs/{writ}/revoke"))
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_str(&format!("Bearer {tok}")).unwrap(),
        )
        .await;
    resp.assert_status(axum::http::StatusCode::FORBIDDEN);

    // Admin role → allowed.
    let admin = bearer(json!({ "sub": "u", "roles": ["admin"], "exp": 9999999999u64, "iat": 1000000000u64 }));
    let resp = server
        .post(&format!("/writs/{writ}/revoke"))
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_str(&format!("Bearer {admin}")).unwrap(),
        )
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn marketplace_requires_auth_when_jwt_configured() {
    // Fix: marketplace routes are now behind the auth layer. Without a token the
    // (previously open) marketplace endpoint is rejected.
    let server = test_server(jwt_test_state());
    let resp = server.get("/marketplace/packages").await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn routing_outcomes_endpoint_returns_safe_records() {
    let server = test_server(test_state());
    // A routed action with evidence.
    let resp = server
        .post("/routed-submit")
        .json(&json!({
            "tool": "kv_set",
            "args": { "key": "k", "value": "secret-value" },
            "routing_evidence": {
                "decision_hash": "deadbeef",
                "selected": "anthropic:claude",
                "alternatives": [],
                "confidence_bps": 9000,
                "reason_codes": ["cost_optimal"],
                "latency_estimate_ms": 100,
                "cost_estimate_millicents": 50
            }
        }))
        .await;
    resp.assert_status_ok();
    let traj = resp.json::<Value>()["trajectory_id"].as_str().unwrap().to_string();

    // Pull the safe feedback for that trajectory.
    let resp = server.get(&format!("/runs/{traj}/routing-outcomes")).await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let outcomes = body["outcomes"].as_array().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0]["decision_hash"], "deadbeef");
    assert_eq!(outcomes[0]["selected"], "anthropic:claude");
    // No workload content leaks through the feedback endpoint.
    let raw = body.to_string();
    assert!(!raw.contains("secret-value") && !raw.contains("\"k\""));
}
