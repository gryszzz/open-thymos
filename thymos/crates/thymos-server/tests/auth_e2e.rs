//! End-to-end tests for the JWT auth middleware.
//!
//! Spins up the full axum router wired with a `JwtConfig` and asserts the
//! documented contract:
//!
//!   1. `/health` is always accessible (bypasses auth).
//!   2. Protected routes without `Authorization` → 401.
//!   3. Protected routes with `Authorization: Bearer <bad>` → 401.
//!   4. Protected routes with an expired token → 401.
//!   5. Protected routes with a wrong-secret token → 401.
//!   6. Protected routes with a malformed header (no `Bearer ` prefix) → 401.
//!   7. Protected routes with a valid token → 200 and tenant claim is honored.
//!   8. The `x-thymos-user-id` header bypasses JWT verification (the
//!      API-gateway-already-authenticated escape hatch).
//!   9. JWTs with an `iss`/`aud` mismatch are rejected when issuer/audience
//!      validation is configured.

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use axum_test::TestServer;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header, Validation};
use serde_json::{json, Value};

use thymos_server::{app, auth, default_runtime, AppState};

const TEST_SECRET: &[u8] = b"test-secret-key-at-least-32-bytes!";
const WRONG_SECRET: &[u8] = b"wrong-secret-key-at-least-32-byt!";

fn test_state_with_jwt(jwt: Option<Arc<auth::JwtConfig>>) -> Arc<AppState> {
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
        jwt_config: jwt,
        pending_approvals: Mutex::new(HashMap::new()),
        cancellation_tokens: Mutex::new(HashMap::new()),
        run_store: None,
        shutdown_tx,
        active_runs: AtomicU32::new(0),
        marketplace: Arc::new(thymos_marketplace::MarketplaceService::in_memory()),
        default_cognition: thymos_server::CognitionConfig::default(),
    })
}

fn jwt_config_default() -> Arc<auth::JwtConfig> {
    Arc::new(auth::JwtConfig::from_secret(TEST_SECRET))
}

fn make_claims(sub: &str, tenant: Option<&str>, exp: u64) -> auth::JwtClaims {
    auth::JwtClaims {
        sub: sub.into(),
        tenant_id: tenant.map(|s| s.into()),
        name: None,
        email: None,
        roles: vec![],
        iss: None,
        aud: None,
        exp: Some(exp),
        iat: Some(0),
    }
}

fn sign(claims: &auth::JwtClaims, secret: &[u8]) -> String {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret),
    )
    .unwrap()
}

fn sign_with_iss_aud(
    claims: &auth::JwtClaims,
    secret: &[u8],
    iss: Option<&str>,
    aud: Option<&str>,
) -> String {
    let mut c = claims.clone();
    c.iss = iss.map(|s| s.into());
    c.aud = aud.map(|s| s.into());
    encode(&Header::default(), &c, &EncodingKey::from_secret(secret)).unwrap()
}

fn bearer(token: &str) -> axum::http::HeaderValue {
    axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap()
}

const AUTHORIZATION: axum::http::HeaderName = axum::http::header::AUTHORIZATION;

// ─────────────────────────── tests ──────────────────────────────────────────

#[tokio::test]
async fn health_bypasses_jwt() {
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let resp = server.get("/health").await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn protected_route_rejects_missing_authorization() {
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let resp = server.get("/runs").await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    let body: Value = resp.json();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("missing or invalid"),
        "error message: {body}"
    );
}

#[tokio::test]
async fn protected_route_rejects_malformed_authorization_header() {
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let resp = server
        .get("/runs")
        .add_header(
            AUTHORIZATION,
            axum::http::HeaderValue::from_static("NotBearer abc"),
        )
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_rejects_garbage_bearer_token() {
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let resp = server
        .get("/runs")
        .add_header(AUTHORIZATION, bearer("not.a.real.jwt"))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    let body: Value = resp.json();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("JWT verification failed"),
        "expected JWT verification failure, got: {body}"
    );
}

#[tokio::test]
async fn protected_route_rejects_expired_token() {
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let claims = make_claims("u1", Some("t1"), 0); // epoch => expired
    let token = sign(&claims, TEST_SECRET);
    let resp = server
        .get("/runs")
        .add_header(AUTHORIZATION, bearer(&token))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_rejects_wrong_secret_token() {
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let claims = make_claims("u1", Some("t1"), u64::MAX);
    let token = sign(&claims, WRONG_SECRET);
    let resp = server
        .get("/runs")
        .add_header(AUTHORIZATION, bearer(&token))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn protected_route_accepts_valid_token() {
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let claims = make_claims("user-abc", Some("tenant-abc"), u64::MAX);
    let token = sign(&claims, TEST_SECRET);
    let resp = server
        .get("/runs")
        .add_header(AUTHORIZATION, bearer(&token))
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn jwt_tenant_claim_isolates_runs() {
    // With JWT auth on, a run created by tenant A must be invisible to
    // tenant B and reachable from tenant A only.
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));

    let token_a = sign(
        &make_claims("user-a", Some("tenant-a"), u64::MAX),
        TEST_SECRET,
    );
    let token_b = sign(
        &make_claims("user-b", Some("tenant-b"), u64::MAX),
        TEST_SECRET,
    );

    // Tenant A creates a run.
    let resp = server
        .post("/runs")
        .add_header(AUTHORIZATION, bearer(&token_a))
        .json(&json!({
            "task": "tenant-a task",
            "cognition": { "provider": "mock" }
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
    let body: Value = resp.json();
    let run_id = body["run_id"].as_str().unwrap().to_string();

    // Let mock cognition complete.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Tenant A can read it.
    let resp = server
        .get(&format!("/runs/{run_id}"))
        .add_header(AUTHORIZATION, bearer(&token_a))
        .await;
    resp.assert_status_ok();

    // Tenant B must NOT be able to read it. The auth middleware lets the
    // request through (token is valid) but the tenant-access check rejects.
    let resp = server
        .get(&format!("/runs/{run_id}"))
        .add_header(AUTHORIZATION, bearer(&token_b))
        .await;
    // Either 403 or 404 is acceptable depending on leak policy.
    let status = resp.status_code();
    assert!(
        status == axum::http::StatusCode::FORBIDDEN || status == axum::http::StatusCode::NOT_FOUND,
        "expected 403 or 404 for cross-tenant read, got {status}"
    );
}

#[tokio::test]
async fn x_thymos_user_id_header_bypasses_jwt() {
    // Documented escape hatch: when an upstream gateway has already
    // authenticated, it sets `x-thymos-user-id` and JWT is skipped.
    let state = test_state_with_jwt(Some(jwt_config_default()));
    let server = TestServer::new(app(state));
    let resp = server
        .get("/runs")
        .add_header(
            axum::http::HeaderName::from_static("x-thymos-user-id"),
            axum::http::HeaderValue::from_static("gateway-authenticated-user"),
        )
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn issuer_mismatch_is_rejected_when_configured() {
    // Build a JwtConfig that requires a specific issuer.
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.set_issuer(&["thymos.test"]);
    let jwt = Arc::new(auth::JwtConfig {
        decoding_key: jsonwebtoken::DecodingKey::from_secret(TEST_SECRET),
        validation,
    });
    let state = test_state_with_jwt(Some(jwt));
    let server = TestServer::new(app(state));

    let claims = make_claims("user-abc", Some("tenant-abc"), u64::MAX);
    // Token with WRONG issuer.
    let bad = sign_with_iss_aud(&claims, TEST_SECRET, Some("evil.test"), None);
    let resp = server
        .get("/runs")
        .add_header(AUTHORIZATION, bearer(&bad))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);

    // Token with the right issuer is accepted.
    let good = sign_with_iss_aud(&claims, TEST_SECRET, Some("thymos.test"), None);
    let resp = server
        .get("/runs")
        .add_header(AUTHORIZATION, bearer(&good))
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn audience_mismatch_is_rejected_when_configured() {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.set_audience(&["thymos-api"]);
    let jwt = Arc::new(auth::JwtConfig {
        decoding_key: jsonwebtoken::DecodingKey::from_secret(TEST_SECRET),
        validation,
    });
    let state = test_state_with_jwt(Some(jwt));
    let server = TestServer::new(app(state));

    let claims = make_claims("user-abc", Some("tenant-abc"), u64::MAX);
    let bad = sign_with_iss_aud(&claims, TEST_SECRET, None, Some("some-other-api"));
    let resp = server
        .get("/runs")
        .add_header(AUTHORIZATION, bearer(&bad))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn jwt_off_means_unauthenticated_access_is_allowed() {
    // Regression guard: when JWT is not configured, the middleware is not
    // mounted and requests succeed without any Authorization header.
    let state = test_state_with_jwt(None);
    let server = TestServer::new(app(state));
    let resp = server.get("/runs").await;
    resp.assert_status_ok();
}
