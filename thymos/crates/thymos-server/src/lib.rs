//! Thymos HTTP facade.
//!
//! Thin axum layer over the Thymos runtime with async streaming cognition.
//!
//! Endpoints:
//!   POST   /runs                        — start a new agent run
//!   GET    /runs/:id                    — get run summary
//!   GET    /runs/:id/events             — SSE stream of trajectory entries
//!   GET    /runs/:id/stream             — SSE stream of cognition events (tokens)
//!   GET    /runs/:id/world              — current world projection
//!   GET    /runs/:id/replay             — verify and fold the execution ledger
//!   POST   /runs/:id/approvals/:channel — approve/deny a pending proposal

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex};

pub mod audit;
pub mod auth;
pub mod execution;
pub mod marketplace_api;
pub mod middleware;
pub mod run_store;
pub mod telemetry;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse, Json,
    },
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{broadcast, oneshot, watch};
use tower_http::cors::{Any, CorsLayer};

use execution::ExecutionSession;
use thymos_cognition::{build_cognition, CognitionEvent, NonStreamingAdapter};
pub use thymos_cognition::{CognitionConfig, CognitionProvider};
use thymos_core::{
    content_hash,
    crypto::{generate_signing_key, public_key_of},
    intent::{Intent, IntentBody, IntentKind},
    proposal::RoutingEvidence,
    writ::{Budget, DelegationBounds, EffectCeiling, TimeWindow, ToolPattern, Writ, WritBody},
    TrajectoryId,
};
use thymos_ledger::{EntryPayload, Ledger};
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::agent_async::ApprovalDecision;
use thymos_runtime::{
    routing_outcomes, AgentEventCallback, AgentRunOptions, AgentRunSummary, AgentTraceEvent,
    Runtime, Step, Termination,
};
use thymos_tools::{
    DelegateTool, FsPatchTool, FsReadTool, GrepTool, HttpTool, KvGetTool, KvSetTool, ListFilesTool,
    MemoryRecallTool, MemoryStoreTool, RepoMapTool, ShellTool, TestRunTool, ToolRegistry,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    Reference,
    Production,
}

impl RuntimeMode {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("THYMOS_RUNTIME_MODE")
            .unwrap_or_else(|_| "reference".into())
            .to_lowercase()
            .as_str()
        {
            "reference" => Ok(Self::Reference),
            "production" => Ok(Self::Production),
            other => Err(format!("unsupported THYMOS_RUNTIME_MODE '{other}'")),
        }
    }
}

/// Human-readable name for a cognition provider (for logs and `/health`).
pub fn provider_label(p: &CognitionProvider) -> &'static str {
    match p {
        CognitionProvider::Anthropic => "anthropic",
        CognitionProvider::Openai => "openai",
        CognitionProvider::Local => "local",
        CognitionProvider::Lmstudio => "lmstudio",
        CognitionProvider::Huggingface => "huggingface",
        CognitionProvider::Mock => "mock",
    }
}

/// Resolve the cognition provider used for runs that don't specify their own
/// `cognition` block. Resolution order:
///   1. `THYMOS_DEFAULT_PROVIDER` (anthropic | openai | local | lmstudio |
///      huggingface | mock), optionally with `THYMOS_DEFAULT_MODEL`.
///   2. Auto-detect a configured API key: `ANTHROPIC_API_KEY`, then
///      `OPENAI_API_KEY`.
///   3. Fall back to `mock`.
///
/// This removes the "I exported my key but every run is silently mock" footgun:
/// export a key (or set the provider var) and runs that omit `cognition` use it.
pub fn resolve_default_cognition() -> CognitionConfig {
    let model = std::env::var("THYMOS_DEFAULT_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty());

    let provider = match std::env::var("THYMOS_DEFAULT_PROVIDER")
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
    {
        Some(p) => match p.as_str() {
            "anthropic" => CognitionProvider::Anthropic,
            "openai" => CognitionProvider::Openai,
            "local" => CognitionProvider::Local,
            "lmstudio" => CognitionProvider::Lmstudio,
            "huggingface" => CognitionProvider::Huggingface,
            "mock" => CognitionProvider::Mock,
            other => {
                eprintln!(
                    "warn: unknown THYMOS_DEFAULT_PROVIDER '{other}', defaulting to mock"
                );
                CognitionProvider::Mock
            }
        },
        None => {
            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                CognitionProvider::Anthropic
            } else if std::env::var("OPENAI_API_KEY").is_ok() {
                CognitionProvider::Openai
            } else {
                CognitionProvider::Mock
            }
        }
    };

    CognitionConfig {
        provider,
        model,
        ..CognitionConfig::default()
    }
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub runtime_mode: RuntimeMode,
    pub bind_addr: String,
    pub ledger_path: Option<String>,
    pub postgres_url: Option<String>,
    pub run_db_path: String,
    pub gateway_db_path: String,
    pub marketplace_db_path: String,
    pub cors_allowed_origins: Option<Vec<String>>,
    pub max_concurrent_runs_per_tenant: u32,
    pub max_concurrent_runs_global: u32,
    pub tool_manifest_dirs: Vec<String>,
    /// Provider used for runs that omit their own `cognition` block.
    pub default_cognition: CognitionConfig,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let runtime_mode = RuntimeMode::from_env()?;
        let bind_addr = std::env::var("THYMOS_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3001".into());
        let ledger_path = std::env::var("THYMOS_LEDGER_PATH").ok();
        let postgres_url = std::env::var("THYMOS_POSTGRES_URL").ok();
        let run_db_path =
            std::env::var("THYMOS_DB_PATH").unwrap_or_else(|_| "thymos-runs.db".into());
        let gateway_db_path =
            std::env::var("THYMOS_GATEWAY_DB_PATH").unwrap_or_else(|_| "thymos-gateway.db".into());
        let marketplace_db_path = std::env::var("THYMOS_MARKETPLACE_DB_PATH")
            .unwrap_or_else(|_| "thymos-marketplace.db".into());
        let tool_fabric =
            std::env::var("THYMOS_TOOL_FABRIC").unwrap_or_else(|_| "in_process".into());
        let worker_bin = std::env::var("THYMOS_WORKER_BIN").ok();
        let cors_allowed_origins = parse_list_env("THYMOS_ALLOWED_ORIGINS");
        let tool_manifest_dirs =
            parse_list_envs(&["THYMOS_TOOL_MANIFEST_DIRS", "THYMOS_TOOL_MANIFEST_DIR"]);
        let max_concurrent_runs_per_tenant = parse_u32_env(
            "THYMOS_MAX_CONCURRENT_RUNS_PER_TENANT",
            MAX_CONCURRENT_RUNS_PER_TENANT,
        )?;
        let max_concurrent_runs_global = parse_u32_env(
            "THYMOS_MAX_CONCURRENT_RUNS_GLOBAL",
            MAX_CONCURRENT_RUNS_GLOBAL,
        )?;

        if runtime_mode == RuntimeMode::Production && ledger_path.is_none() {
            if postgres_url.is_some() {
                return Err(
                    "THYMOS_POSTGRES_URL is configured, but the server runtime is still wired to the synchronous SQLite ledger path. For now, production mode requires THYMOS_LEDGER_PATH.".into(),
                );
            }
            return Err(
                "production mode requires THYMOS_LEDGER_PATH so the ledger is not ephemeral".into(),
            );
        }

        if runtime_mode == RuntimeMode::Production && tool_fabric != "worker" {
            return Err(
                "production mode requires THYMOS_TOOL_FABRIC=worker so shell/http execution does not run in-process".into(),
            );
        }

        if runtime_mode == RuntimeMode::Production && worker_bin.is_none() {
            return Err(
                "production mode requires THYMOS_WORKER_BIN to point at the thymos-worker binary"
                    .into(),
            );
        }

        if runtime_mode == RuntimeMode::Production
            && cors_allowed_origins
                .as_ref()
                .is_none_or(|origins| origins.is_empty())
        {
            return Err(
                "production mode requires THYMOS_ALLOWED_ORIGINS to be set to a comma-separated list of allowed browser origins".into(),
            );
        }

        if max_concurrent_runs_per_tenant == 0 || max_concurrent_runs_global == 0 {
            return Err("concurrency limits must be greater than zero".into());
        }

        if max_concurrent_runs_per_tenant > max_concurrent_runs_global {
            return Err(
                "THYMOS_MAX_CONCURRENT_RUNS_PER_TENANT cannot exceed THYMOS_MAX_CONCURRENT_RUNS_GLOBAL".into(),
            );
        }

        Ok(Self {
            runtime_mode,
            bind_addr,
            ledger_path,
            postgres_url,
            run_db_path,
            gateway_db_path,
            marketplace_db_path,
            cors_allowed_origins,
            max_concurrent_runs_per_tenant,
            max_concurrent_runs_global,
            tool_manifest_dirs,
            default_cognition: resolve_default_cognition(),
        })
    }
}

fn parse_list_envs(keys: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    for key in keys {
        if let Some(parsed) = parse_list_env(key) {
            for value in parsed {
                if !values.contains(&value) {
                    values.push(value);
                }
            }
        }
    }
    values
}

fn parse_list_env(key: &str) -> Option<Vec<String>> {
    std::env::var(key).ok().and_then(|raw| {
        let values = raw
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if values.is_empty() {
            None
        } else {
            Some(values)
        }
    })
}

fn parse_u32_env(key: &str, default: u32) -> Result<u32, String> {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<u32>()
            .map_err(|_| format!("{key} must be a positive integer")),
        Err(_) => Ok(default),
    }
}

/// Server-side record of a completed or in-progress run.
#[derive(Clone, Debug, Serialize)]
pub struct RunRecord {
    pub trajectory_id: String,
    pub task: String,
    pub status: RunStatus,
    pub summary: Option<RunSummaryDto>,
    /// Tenant that owns this run (empty string = no tenant).
    #[serde(default)]
    pub tenant_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunSummaryDto {
    pub steps_executed: u32,
    pub intents_submitted: u32,
    pub commits: u32,
    pub rejections: u32,
    pub failures: u32,
    pub final_answer: Option<String>,
    pub terminated_by: String,
}

impl From<&AgentRunSummary> for RunSummaryDto {
    fn from(s: &AgentRunSummary) -> Self {
        RunSummaryDto {
            steps_executed: s.steps_executed,
            intents_submitted: s.intents_submitted,
            commits: s.commits,
            rejections: s.rejections,
            failures: s.failures,
            final_answer: s.final_answer.clone(),
            terminated_by: format!("{:?}", s.terminated_by),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EntryDto {
    pub seq: u64,
    pub kind: String,
    pub id: String,
    pub detail: serde_json::Value,
    /// Full (64-hex) commit id for commit entries. None for other kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldDto {
    pub resources: Vec<ResourceDto>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReplayDto {
    pub run_id: String,
    pub trajectory_id: String,
    pub entries_seen: usize,
    pub commits_replayed: usize,
    pub head_commit: Option<String>,
    pub head_seq: u64,
    pub compiler_versions_seen: Vec<String>,
    pub final_world_hash: String,
    pub resources: usize,
    pub rejected_proposals: usize,
    pub pending_approvals: usize,
    pub delegations: usize,
    pub branches: usize,
    pub tool_calls: Vec<ReplayToolCallDto>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReplayToolCallDto {
    pub seq: u64,
    pub commit_id: String,
    pub tool: String,
    pub latency_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResourceDto {
    pub kind: String,
    pub id: String,
    pub version: u64,
    pub value: serde_json::Value,
}

/// Shared application state.
pub struct AppState {
    pub runtime_mode: RuntimeMode,
    pub cors_allowed_origins: Option<Vec<String>>,
    pub max_concurrent_runs_per_tenant: u32,
    pub max_concurrent_runs_global: u32,
    pub runtime: Arc<Runtime>,
    pub runs: Mutex<HashMap<String, RunRecord>>,
    /// Ledger entry events per run.
    pub event_channels: Mutex<HashMap<String, broadcast::Sender<EntryDto>>>,
    /// Cognition streaming events per run (tokens, tool-use deltas).
    pub cognition_channels: Mutex<HashMap<String, broadcast::Sender<CognitionEvent>>>,
    /// Unified execution session snapshots per run.
    pub execution_sessions: Mutex<HashMap<String, ExecutionSession>>,
    /// Broadcast channel of full execution session snapshots per run.
    pub execution_channels: Mutex<HashMap<String, broadcast::Sender<ExecutionSession>>>,
    /// Optional API gateway for auth + rate limiting.
    pub gateway: Option<Arc<middleware::ApiGateway>>,
    /// Optional JWT configuration for token-based auth.
    pub jwt_config: Option<Arc<auth::JwtConfig>>,
    /// Pending approval channels: (run_id, channel_name) → oneshot sender.
    pub pending_approvals: Mutex<HashMap<(String, String), oneshot::Sender<ApprovalDecision>>>,
    /// Cancellation senders: run_id → sender. Send `()` to cancel a run.
    pub cancellation_tokens: Mutex<HashMap<String, watch::Sender<bool>>>,
    /// Persistent run store (SQLite).
    pub run_store: Option<Arc<run_store::RunStore>>,
    /// Shutdown signal: send `true` to initiate graceful shutdown.
    pub shutdown_tx: watch::Sender<bool>,
    /// Number of currently active agent runs.
    pub active_runs: AtomicU32,
    /// Tool marketplace.
    pub marketplace: marketplace_api::MarketplaceState,
    /// Provider used for runs that omit their own `cognition` block. Resolved
    /// from env at startup (see [`resolve_default_cognition`]).
    pub default_cognition: CognitionConfig,
}

#[derive(Deserialize)]
pub struct CreateRunRequest {
    pub task: String,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default)]
    pub tool_scopes: Vec<String>,
    /// Cognition provider config. Defaults to mock if omitted.
    #[serde(default)]
    pub cognition: Option<CognitionConfig>,
}

fn default_max_steps() -> u32 {
    16
}

/// Server-enforced limits.
pub const MAX_TASK_LENGTH: usize = 10_000;
pub const MAX_STEPS_UPPER_BOUND: u32 = 256;
pub const MAX_CONCURRENT_RUNS_PER_TENANT: u32 = 20;
pub const MAX_CONCURRENT_RUNS_GLOBAL: u32 = 100;

#[derive(Deserialize)]
pub struct ApprovalRequest {
    pub approve: bool,
    pub proposal_id: Option<String>,
}

/// Build a runtime with a file-backed ledger.
pub fn persistent_runtime(ledger_path: &str) -> Arc<Runtime> {
    persistent_runtime_with_capabilities(ledger_path, &[])
}

/// Build a runtime with a file-backed ledger and manifest-backed capabilities.
pub fn persistent_runtime_with_capabilities(
    ledger_path: &str,
    tool_manifest_dirs: &[String],
) -> Arc<Runtime> {
    let ledger = Ledger::open(ledger_path).expect("open file-backed ledger");
    build_runtime(ledger, tool_manifest_dirs)
}

/// Build the default runtime with an in-memory ledger (for testing).
pub fn default_runtime() -> Arc<Runtime> {
    default_runtime_with_capabilities(&[])
}

/// Build the default runtime with manifest-backed capabilities.
pub fn default_runtime_with_capabilities(tool_manifest_dirs: &[String]) -> Arc<Runtime> {
    let ledger = Ledger::open_in_memory().expect("open in-memory ledger");
    build_runtime(ledger, tool_manifest_dirs)
}

fn build_runtime(ledger: Ledger, tool_manifest_dirs: &[String]) -> Arc<Runtime> {
    let mut tools = ToolRegistry::new();
    tools.register(KvSetTool::default());
    tools.register(KvGetTool::default());
    tools.register(MemoryStoreTool::default());
    tools.register(MemoryRecallTool::default());
    tools.register(DelegateTool::default());
    tools.register(ShellTool::default());
    tools.register(HttpTool::default());
    // Coding-agent surface — repo-aware, path-confined.
    tools.register(FsReadTool::default());
    tools.register(FsPatchTool::default());
    tools.register(ListFilesTool::default());
    tools.register(RepoMapTool::default());
    tools.register(GrepTool::default());
    tools.register(TestRunTool::default());

    load_programmable_capabilities(&mut tools, tool_manifest_dirs);

    // To register MCP server tools at startup:
    //   tools.register_mcp_server("my-server", &["uvx", "my-mcp-server"])
    //       .expect("spawn MCP server");

    let policy = PolicyEngine::new().with(WritAuthorityPolicy);
    Arc::new(Runtime::new(ledger, tools, policy))
}

fn load_programmable_capabilities(tools: &mut ToolRegistry, tool_manifest_dirs: &[String]) {
    for dir in tool_manifest_dirs {
        let count = tools
            .load_manifest_dir(FsPath::new(dir))
            .unwrap_or_else(|e| panic!("load tool manifests from {dir}: {e}"));
        eprintln!("capabilities: loaded {count} manifest tool(s) from {dir}");
    }
}

/// Build the axum Router.
pub fn app(state: Arc<AppState>) -> Router {
    let marketplace_state = state.marketplace.clone();
    let cors = cors_layer(&state);
    let router = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/runs", get(list_runs).post(create_run))
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/execution", get(get_execution))
        .route("/runs/{id}/execution/stream", get(get_execution_stream))
        .route("/runs/{id}/events", get(get_events))
        .route("/runs/{id}/stream", get(get_stream))
        .route("/runs/{id}/world", get(get_world))
        .route("/runs/{id}/world/at", get(get_world_at))
        .route("/runs/{id}/replay", get(get_replay))
        .route("/runs/{id}/approvals/{channel}", post(post_approval))
        .route("/runs/{id}/resume", post(resume_run))
        .route("/runs/{id}/cancel", post(cancel_run))
        .route("/writs/{writ_id}/revoke", post(revoke_writ_handler))
        .route("/writs/{writ_id}/restore", post(restore_writ_handler))
        .route("/routed-submit", post(routed_submit))
        .route("/runs/{id}/delegations", get(get_delegations))
        .route("/runs/{id}/routing-outcomes", get(get_routing_outcomes))
        .route("/runs/{id}/branch", post(post_branch))
        .route("/usage", get(get_usage))
        .route("/audit/entries", get(audit::get_audit_entries))
        .route("/audit/entries/count", get(audit::count_audit_entries));

    // Combine main + marketplace routes BEFORE the auth layers, so the auth
    // middleware gates the marketplace mutating endpoints (publish/unpublish)
    // too — previously they were merged after the layers and bypassed auth.
    let jwt_config = state.jwt_config.clone();
    let gateway = state.gateway.clone();
    let mut combined = router
        .with_state(state)
        .merge(marketplace_api::marketplace_router(marketplace_state));

    // Wire JWT middleware if configured (wraps main + marketplace).
    if let Some(jwt) = &jwt_config {
        combined = combined.layer(axum::middleware::from_fn_with_state(
            jwt.clone(),
            auth::jwt_middleware,
        ));
    }

    // Wire API gateway middleware if configured.
    if let Some(gw) = &gateway {
        combined = combined.layer(axum::middleware::from_fn_with_state(
            gw.clone(),
            middleware::api_key_middleware,
        ));
    }

    combined.layer(cors)
}

fn cors_layer(state: &Arc<AppState>) -> CorsLayer {
    let base = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    match &state.cors_allowed_origins {
        Some(origins) if !origins.is_empty() => {
            let headers = origins
                .iter()
                .filter_map(|origin| HeaderValue::from_str(origin).ok())
                .collect::<Vec<_>>();
            base.allow_origin(headers)
        }
        _ => base.allow_origin(Any),
    }
}

/// Extract tenant_id from request context.
/// Priority: JWT claims > gateway context > x-thymos-tenant-id header.
fn extract_tenant_id(
    headers: &HeaderMap,
    jwt_claims: &Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: &Option<axum::Extension<middleware::GatewayContext>>,
) -> String {
    if let Some(axum::Extension(claims)) = jwt_claims {
        if let Some(ref tid) = claims.tenant_id {
            if !tid.is_empty() {
                return tid.clone();
            }
        }
    }
    if let Some(axum::Extension(ctx)) = gateway_ctx {
        if !ctx.tenant_id.is_empty() {
            return ctx.tenant_id.clone();
        }
    }
    headers
        .get("x-thymos-tenant-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Authorization gate for privileged control-plane endpoints (writ
/// revoke/restore). When a JWT layer is configured, the caller's claims must
/// include the `admin` role; otherwise (no auth layer — local/dev mode) it is
/// permitted, consistent with the rest of the server being open when unconfigured.
fn require_admin(
    jwt_claims: &Option<axum::Extension<auth::JwtClaims>>,
) -> Result<(), axum::response::Response> {
    match jwt_claims {
        Some(axum::Extension(claims)) => {
            if claims.roles.iter().any(|r| r == "admin") {
                Ok(())
            } else {
                Err((
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({ "error": "admin role required" })),
                )
                    .into_response())
            }
        }
        None => Ok(()),
    }
}

fn run_status_from_summary(summary: &AgentRunSummary) -> RunStatus {
    if matches!(summary.terminated_by, Termination::CognitionDone) {
        RunStatus::Completed
    } else {
        RunStatus::Failed
    }
}

fn status_str(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "running",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
    }
}

fn ensure_execution_channel(
    state: &Arc<AppState>,
    run_id: &str,
) -> broadcast::Sender<ExecutionSession> {
    let mut channels = state.execution_channels.lock().unwrap();
    channels
        .entry(run_id.to_string())
        .or_insert_with(|| {
            let (tx, _) = broadcast::channel(256);
            tx
        })
        .clone()
}

fn publish_execution_session(state: &Arc<AppState>, run_id: &str, session: ExecutionSession) {
    {
        let mut sessions = state.execution_sessions.lock().unwrap();
        sessions.insert(run_id.to_string(), session.clone());
    }
    let tx = ensure_execution_channel(state, run_id);
    let _ = tx.send(session);
}

fn with_execution_session<F>(
    state: &Arc<AppState>,
    run_id: &str,
    task: &str,
    max_steps: u32,
    mut f: F,
) where
    F: FnMut(&mut ExecutionSession),
{
    let session = {
        let mut sessions = state.execution_sessions.lock().unwrap();
        let session = sessions
            .entry(run_id.to_string())
            .or_insert_with(|| ExecutionSession::new(run_id, task, max_steps));
        f(session);
        session.clone()
    };
    let tx = ensure_execution_channel(state, run_id);
    let _ = tx.send(session);
}

/// Check if a caller has access to a run. Empty tenant = no restriction.
fn tenant_can_access(run_tenant: &str, caller_tenant: &str) -> bool {
    // If the run has no tenant, anyone can access it.
    if run_tenant.is_empty() {
        return true;
    }
    // If the caller has no tenant (e.g. no auth), deny access to tenanted runs.
    if caller_tenant.is_empty() {
        return false;
    }
    run_tenant == caller_tenant
}

fn trajectory_from_hex(traj_hex: &str) -> Result<TrajectoryId, &'static str> {
    // Accept both the bare 32-byte hex and the `traj:<hex>` display form.
    let traj_hex = traj_hex.strip_prefix("traj:").unwrap_or(traj_hex);
    let bytes = hex::decode(traj_hex).map_err(|_| "invalid trajectory id")?;
    if bytes.len() != 32 {
        return Err("invalid trajectory id");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(TrajectoryId(thymos_core::ContentHash(arr)))
}

fn writ_id_from_hex(writ_hex: &str) -> Result<thymos_core::WritId, &'static str> {
    let bytes = hex::decode(writ_hex).map_err(|_| "invalid writ id")?;
    if bytes.len() != 32 {
        return Err("invalid writ id");
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(thymos_core::WritId(thymos_core::ContentHash(arr)))
}

/// One governed action submitted with pre-Proposal routing evidence. This is the
/// WisePick integration path: the advisor decides the route, the client posts the
/// action + evidence, THYMOS governs (writ/effect/budget/policy), executes, and
/// ledgers the result — with `routing_evidence` recorded immutably on the commit.
#[derive(Debug, Deserialize)]
pub struct RoutedSubmitRequest {
    /// Tool to invoke.
    pub tool: String,
    /// Tool arguments.
    #[serde(default)]
    pub args: serde_json::Value,
    /// Optional rationale recorded on the intent.
    #[serde(default)]
    pub rationale: String,
    /// Routing evidence to bind into the ledgered proposal (audit/replay only).
    #[serde(default)]
    pub routing_evidence: Option<RoutingEvidence>,
}

/// Submit a single routed action. Creates its own trajectory, mints a writ
/// scoped to the requested tool, attaches `routing_evidence`, and returns the
/// governed outcome.
async fn routed_submit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: Option<axum::Extension<middleware::GatewayContext>>,
    Json(req): Json<RoutedSubmitRequest>,
) -> impl IntoResponse {
    let runtime = state.runtime.clone();
    // Scope the minted writ to the caller's tenant so the routed action is
    // subject to tenant-isolation policy (was previously an unscoped "system"
    // writ that could touch any tenant's resources).
    let tenant_id = extract_tenant_id(&headers, &jwt_claims, &gateway_ctx);

    // Fresh trajectory per routed action.
    let seed = thymos_core::crypto::random_nonce();
    let run = match runtime.create_run(&format!("routed:{}", req.tool), &seed) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    let trajectory_hex = run.trajectory_id().to_string();

    // Mint a writ scoped to just this tool.
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();
    let writ = match Writ::sign(
        WritBody {
            issuer: "server".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "routed".into(),
            subject_pubkey: public_key_of(&agent_key),
            nonce: thymos_core::crypto::random_nonce(),
            parent: None,
            tenant_id: tenant_id.clone(),
            tool_scopes: vec![ToolPattern::exact(&req.tool)],
            budget: Budget {
                tokens: 100_000,
                tool_calls: 8,
                wall_clock_ms: 300_000,
                usd_millicents: 0,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow {
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        },
        &root_key,
    ) {
        Ok(w) => w,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let intent = match Intent::new(IntentBody {
        parent_commit: None,
        author: "routed-submit".into(),
        kind: IntentKind::Act,
        target: req.tool.clone(),
        args: req.args.clone(),
        rationale: req.rationale.clone(),
        nonce: thymos_core::crypto::random_nonce(),
    }) {
        Ok(i) => i,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let result = match req.routing_evidence.clone() {
        Some(ev) => run.submit_with_routing_evidence(intent, &writ, ev),
        None => run.submit(intent, &writ),
    };

    let body = match result {
        Ok(Step::Committed(id)) => serde_json::json!({
            "status": "committed",
            "trajectory_id": trajectory_hex,
            "commit_id": id.to_string(),
            "routing_evidence_recorded": req.routing_evidence.is_some(),
        }),
        Ok(Step::Rejected(reason)) => serde_json::json!({
            "status": "rejected",
            "trajectory_id": trajectory_hex,
            "reason": reason.to_string(),
        }),
        Ok(Step::Suspended { channel, reason }) => serde_json::json!({
            "status": "suspended",
            "trajectory_id": trajectory_hex,
            "channel": channel,
            "reason": reason,
        }),
        Ok(Step::Delegated { child_trajectory_id, .. }) => serde_json::json!({
            "status": "delegated",
            "trajectory_id": trajectory_hex,
            "child_trajectory_id": child_trajectory_id.to_string(),
        }),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// Revoke a writ by id. Subsequent submissions under it (or under any child
/// whose immediate parent is it) are rejected as AuthorityVoid by the compiler.
async fn revoke_writ_handler(
    State(state): State<Arc<AppState>>,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    Path(writ_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&jwt_claims) {
        return resp;
    }
    match writ_id_from_hex(writ_id.trim()) {
        Ok(id) => {
            state.runtime.revoke_writ(id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "revoked": true, "writ_id": writ_id })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Reinstate a previously revoked writ (e.g. an erroneous revocation).
async fn restore_writ_handler(
    State(state): State<Arc<AppState>>,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    Path(writ_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = require_admin(&jwt_claims) {
        return resp;
    }
    match writ_id_from_hex(writ_id.trim()) {
        Ok(id) => {
            state.runtime.revocations.restore(&id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "restored": true, "writ_id": writ_id })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

fn run_record_for_access(
    state: &Arc<AppState>,
    run_id: &str,
    caller_tenant: &str,
) -> Result<RunRecord, StatusCode> {
    {
        let runs = state.runs.lock().unwrap();
        if let Some(rec) = runs.get(run_id) {
            if tenant_can_access(&rec.tenant_id, caller_tenant) {
                return Ok(rec.clone());
            }
            return Err(StatusCode::NOT_FOUND);
        }
    }

    if let Some(store) = &state.run_store {
        if let Ok(Some(rec)) = store.get(run_id) {
            if tenant_can_access(&rec.tenant_id, caller_tenant) {
                return Ok(rec);
            }
            return Err(StatusCode::NOT_FOUND);
        }
    }

    Err(StatusCode::NOT_FOUND)
}

/// GET /health — liveness probe (bypasses API key auth).
async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "mode": match state.runtime_mode {
            RuntimeMode::Reference => "reference",
            RuntimeMode::Production => "production",
        },
        "default_provider": provider_label(&state.default_cognition.provider),
        "shutdown": *state.shutdown_tx.borrow(),
    }))
}

async fn ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let shutting_down = *state.shutdown_tx.borrow();
    let run_store_ready = state.run_store.is_some();
    let marketplace_ready = true;
    let ready = !shutting_down
        && match state.runtime_mode {
            RuntimeMode::Reference => true,
            RuntimeMode::Production => run_store_ready && marketplace_ready,
        };

    let body = serde_json::json!({
        "status": if ready { "ready" } else { "not_ready" },
        "mode": match state.runtime_mode {
            RuntimeMode::Reference => "reference",
            RuntimeMode::Production => "production",
        },
        "default_provider": provider_label(&state.default_cognition.provider),
        "shutdown": shutting_down,
        "checks": {
            "run_store": run_store_ready,
            "marketplace": marketplace_ready,
        }
    });

    if ready {
        (StatusCode::OK, Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}

/// GET /usage — per-key usage stats dashboard.
async fn get_usage(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match &state.gateway {
        Some(gw) => {
            let stats = gw.usage_stats();
            (StatusCode::OK, Json(serde_json::to_value(stats).unwrap())).into_response()
        }
        None => (
            StatusCode::OK,
            Json(serde_json::json!({ "message": "API gateway not configured", "stats": [] })),
        )
            .into_response(),
    }
}

/// POST /runs/:id/resume — resume a previously started (and possibly crashed) run.
///
/// Looks up the run record, verifies it's in a resumable state (running or failed),
/// and re-spawns the agent loop from where the ledger left off.
async fn resume_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateRunRequest>,
) -> impl IntoResponse {
    // Look up the run.
    let run_record = {
        let runs = state.runs.lock().unwrap();
        runs.get(&id).cloned()
    };

    let record = match run_record {
        Some(rec) => rec,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "run not found" })),
            )
                .into_response()
        }
    };

    if record.status == RunStatus::Completed {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "run already completed" })),
        )
            .into_response();
    }

    if record.trajectory_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "run has no trajectory to resume" })),
        )
            .into_response();
    }

    // Parse trajectory ID.
    let traj_bytes = match hex::decode(&record.trajectory_id) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid trajectory id" })),
            )
                .into_response()
        }
    };

    let trajectory_id = TrajectoryId(thymos_core::ContentHash(traj_bytes));

    // Verify trajectory exists in ledger.
    if !state.runtime.ledger.has_trajectory(trajectory_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "trajectory not found in ledger" })),
        )
            .into_response();
    }

    // Mark as running again.
    {
        let mut runs = state.runs.lock().unwrap();
        if let Some(rec) = runs.get_mut(&id) {
            rec.status = RunStatus::Running;
            rec.summary = None;
        }
    }

    // Create fresh channels.
    let (entry_tx, _) = broadcast::channel(256);
    let (cognition_tx, _) = broadcast::channel::<CognitionEvent>(256);
    let (execution_tx, _) = broadcast::channel::<ExecutionSession>(256);
    {
        let mut channels = state.event_channels.lock().unwrap();
        channels.insert(id.clone(), entry_tx.clone());
    }
    {
        let mut channels = state.cognition_channels.lock().unwrap();
        channels.insert(id.clone(), cognition_tx.clone());
    }
    {
        let mut channels = state.execution_channels.lock().unwrap();
        channels.insert(id.clone(), execution_tx);
    }
    publish_execution_session(
        &state,
        &id,
        ExecutionSession::new(&id, &req.task, req.max_steps),
    );

    let runtime = state.runtime.clone();
    let state2 = state.clone();
    let run_id = id.clone();
    let task = req.task.clone();

    tokio::spawn(async move {
        let config = req
            .cognition
            .clone()
            .unwrap_or_else(|| state2.default_cognition.clone());
        // `build_cognition` may call `reqwest::blocking::ClientBuilder::build()`,
        // which internally creates and drops a current-thread tokio runtime.
        // Dropping a runtime from inside an async context panics, so construct
        // the client on a blocking thread.
        let cognition = match tokio::task::spawn_blocking(move || build_cognition(&config)).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("cognition construction panicked: {e}");
                return;
            }
        };
        let mut streaming = NonStreamingAdapter(cognition);

        // Resume: the runtime will pick up the existing trajectory.
        let run = match runtime.resume_run(trajectory_id) {
            Ok(r) => r,
            Err(e) => {
                let mut runs = state2.runs.lock().unwrap();
                if let Some(rec) = runs.get_mut(&run_id) {
                    rec.status = RunStatus::Failed;
                    rec.summary = Some(RunSummaryDto {
                        steps_executed: 0,
                        intents_submitted: 0,
                        commits: 0,
                        rejections: 0,
                        failures: 1,
                        final_answer: Some(format!("resume failed: {e}")),
                        terminated_by: "Error".into(),
                    });
                }
                with_execution_session(&state2, &run_id, &task, req.max_steps, |session| {
                    session.mark_failed(format!("resume failed: {e}"));
                });
                return;
            }
        };

        // Get existing world state (checkpoint).
        let _world = run.project_world().ok();
        let summary = run.summary().ok();
        let existing_commits = summary.as_ref().map(|s| s.commits).unwrap_or(0);

        // Re-run the agent from the checkpoint. The cognition will start
        // fresh but the world state carries forward from the ledger.
        // We need to use run_agent_streaming with the existing runtime.
        let state_for_approvals = state2.clone();
        let run_id_for_approvals = run_id.clone();
        let approval_requester: thymos_runtime::agent_async::ApprovalRequester =
            Box::new(move |_traj_id, _proposal_id, channel, _reason| {
                let (tx, rx) = oneshot::channel();
                let mut pending = state_for_approvals.pending_approvals.lock().unwrap();
                pending.insert((run_id_for_approvals.clone(), channel), tx);
                rx
            });
        let trace_state = state2.clone();
        let trace_run_id = run_id.clone();
        let trace_task = task.clone();
        let trace_max_steps = req.max_steps;
        let trace_tx: AgentEventCallback = Arc::new(move |event: AgentTraceEvent| {
            with_execution_session(
                &trace_state,
                &trace_run_id,
                &trace_task,
                trace_max_steps,
                |session| {
                    session.apply_trace(event.clone());
                },
            );
        });

        let result = thymos_runtime::run_agent_streaming(
            &runtime,
            &mut streaming,
            &task,
            // Mint a fresh writ for the resumed run.
            &{
                let root_key = generate_signing_key();
                let agent_key = generate_signing_key();
                Writ::sign(
                    WritBody {
                        issuer: "server".into(),
                        issuer_pubkey: public_key_of(&root_key),
                        subject: format!("resumed-{}", run_id),
                        subject_pubkey: public_key_of(&agent_key),
                        nonce: thymos_core::crypto::random_nonce(),
                        parent: None,
                        tenant_id: String::new(),
                        tool_scopes: vec![ToolPattern::exact("*")],
                        budget: Budget {
                            tokens: 100_000,
                            tool_calls: 64,
                            wall_clock_ms: 300_000,
                            usd_millicents: 0,
                        },
                        effect_ceiling: EffectCeiling::read_write_local(),
                        time_window: TimeWindow {
                            not_before: 0,
                            expires_at: u64::MAX,
                        },
                        delegation: DelegationBounds {
                            max_depth: 3,
                            may_subdivide: true,
                        },
                    },
                    &root_key,
                )
                .expect("sign writ")
            },
            AgentRunOptions {
                max_steps: req.max_steps,
            },
            cognition_tx,
            Some(approval_requester),
            Some(trace_tx),
        )
        .await;

        let mut runs = state2.runs.lock().unwrap();
        match result {
            Ok(summary) => {
                let traj_id = summary.trajectory_id.to_string();
                let dto = RunSummaryDto::from(&summary);
                // Add existing commits to the count.
                let mut full_dto = dto.clone();
                full_dto.commits += existing_commits as u32;
                let status = run_status_from_summary(&summary);
                if let Some(rec) = runs.get_mut(&run_id) {
                    rec.trajectory_id = traj_id;
                    rec.status = status.clone();
                    rec.summary = Some(full_dto.clone());
                }
                with_execution_session(&state2, &run_id, &task, req.max_steps, |session| {
                    session.apply_summary(&summary);
                });
            }
            Err(e) => {
                if let Some(rec) = runs.get_mut(&run_id) {
                    rec.status = RunStatus::Failed;
                    rec.summary = Some(RunSummaryDto {
                        steps_executed: 0,
                        intents_submitted: 0,
                        commits: 0,
                        rejections: 0,
                        failures: 1,
                        final_answer: Some(format!("error: {e}")),
                        terminated_by: "Error".into(),
                    });
                }
                with_execution_session(&state2, &run_id, &task, req.max_steps, |session| {
                    session.mark_failed(format!("error: {e}"));
                });
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "run_id": id,
            "status": "resuming",
        })),
    )
        .into_response()
}

/// POST /runs ��� start a new agent run with async streaming cognition.
async fn create_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: Option<axum::Extension<middleware::GatewayContext>>,
    Json(req): Json<CreateRunRequest>,
) -> impl IntoResponse {
    // Reject new runs during shutdown.
    if *state.shutdown_tx.borrow() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "server is shutting down" })),
        )
            .into_response();
    }

    // Input validation.
    if req.task.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "task is required" })),
        )
            .into_response();
    }
    if req.task.len() > MAX_TASK_LENGTH {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("task exceeds max length of {MAX_TASK_LENGTH} characters") }))).into_response();
    }
    if req.max_steps > MAX_STEPS_UPPER_BOUND {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("max_steps cannot exceed {MAX_STEPS_UPPER_BOUND}") }))).into_response();
    }

    // Global concurrency check.
    let active = state.active_runs.load(Ordering::Relaxed);
    if active >= state.max_concurrent_runs_global {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": format!("server at capacity ({})", state.max_concurrent_runs_global) })),
        )
            .into_response();
    }

    let tenant_id = extract_tenant_id(&headers, &jwt_claims, &gateway_ctx);

    // Per-tenant concurrency check.
    if !tenant_id.is_empty() {
        let runs = state.runs.lock().unwrap();
        let tenant_running = runs
            .values()
            .filter(|r| r.tenant_id == tenant_id && r.status == RunStatus::Running)
            .count() as u32;
        if tenant_running >= state.max_concurrent_runs_per_tenant {
            return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({
                "error": format!("tenant has reached max concurrent runs ({})", state.max_concurrent_runs_per_tenant)
            }))).into_response();
        }
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let task = req.task.clone();

    let user_id = if let Some(axum::Extension(claims)) = &jwt_claims {
        claims.sub.clone()
    } else {
        headers
            .get("x-thymos-user-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };

    let subject = if user_id.is_empty() {
        format!("run-{}", run_id)
    } else {
        format!("user-{}-run-{}", user_id, run_id)
    };

    // Mint a server-side writ.
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();
    let scopes: Vec<ToolPattern> = if req.tool_scopes.is_empty() {
        vec![ToolPattern::exact("*")]
    } else {
        req.tool_scopes
            .iter()
            .map(|s| ToolPattern::exact(s.as_str()))
            .collect()
    };

    let writ = match Writ::sign(
        WritBody {
            issuer: "server".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject,
            subject_pubkey: public_key_of(&agent_key),
            nonce: thymos_core::crypto::random_nonce(),
            parent: None,
            tenant_id: tenant_id.clone(),
            tool_scopes: scopes,
            budget: Budget {
                tokens: 100_000,
                tool_calls: 64,
                wall_clock_ms: 300_000,
                usd_millicents: 0,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow {
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 3,
                may_subdivide: true,
            },
        },
        &root_key,
    ) {
        Ok(w) => w,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    // Create event channels.
    let (entry_tx, _) = broadcast::channel(256);
    let (cognition_tx, _) = broadcast::channel::<CognitionEvent>(256);
    let (execution_tx, _) = broadcast::channel::<ExecutionSession>(256);
    {
        let mut channels = state.event_channels.lock().unwrap();
        channels.insert(run_id.clone(), entry_tx.clone());
    }
    {
        let mut channels = state.cognition_channels.lock().unwrap();
        channels.insert(run_id.clone(), cognition_tx.clone());
    }
    {
        let mut channels = state.execution_channels.lock().unwrap();
        channels.insert(run_id.clone(), execution_tx);
    }

    // Record the run as running.
    {
        let mut runs = state.runs.lock().unwrap();
        runs.insert(
            run_id.clone(),
            RunRecord {
                trajectory_id: String::new(),
                task: task.clone(),
                status: RunStatus::Running,
                summary: None,
                tenant_id: tenant_id.clone(),
            },
        );
    }
    // Persist to SQLite.
    if let Some(store) = &state.run_store {
        let _ = store.insert(&run_id, &task, &tenant_id);
    }
    publish_execution_session(
        &state,
        &run_id,
        ExecutionSession::new(&run_id, &task, req.max_steps),
    );

    // Spawn the agent as an async task with streaming cognition.
    // Create cancellation token for this run.
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    {
        let mut tokens = state.cancellation_tokens.lock().unwrap();
        tokens.insert(run_id.clone(), cancel_tx);
    }

    state.active_runs.fetch_add(1, Ordering::Relaxed);
    let runtime = state.runtime.clone();
    let state2 = state.clone();
    let run_id2 = run_id.clone();
    let task2 = task.clone();

    tokio::spawn(async move {
        // Build cognition from per-run config, or the server's configured
        // default provider (env-resolved) instead of silently using mock.
        let config = req
            .cognition
            .clone()
            .unwrap_or_else(|| state2.default_cognition.clone());
        // `build_cognition` may call `reqwest::blocking::ClientBuilder::build()`,
        // which internally creates and drops a current-thread tokio runtime.
        // Dropping a runtime from inside an async context panics, so construct
        // the client on a blocking thread.
        let cognition = match tokio::task::spawn_blocking(move || build_cognition(&config)).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("cognition construction panicked: {e}");
                return;
            }
        };
        let mut streaming = NonStreamingAdapter(cognition);

        // Build approval requester that parks a oneshot in AppState.
        let state_for_approvals = state2.clone();
        let run_id_for_approvals = run_id2.clone();
        let approval_requester: thymos_runtime::agent_async::ApprovalRequester =
            Box::new(move |_traj_id, _proposal_id, channel, _reason| {
                let (tx, rx) = oneshot::channel();
                let mut pending = state_for_approvals.pending_approvals.lock().unwrap();
                pending.insert((run_id_for_approvals.clone(), channel), tx);
                rx
            });
        let trace_state = state2.clone();
        let trace_run_id = run_id2.clone();
        let trace_task = task2.clone();
        let trace_max_steps = req.max_steps;
        let trace_tx: AgentEventCallback = Arc::new(move |event: AgentTraceEvent| {
            with_execution_session(
                &trace_state,
                &trace_run_id,
                &trace_task,
                trace_max_steps,
                |session| {
                    session.apply_trace(event.clone());
                },
            );
        });

        let agent_fut = thymos_runtime::run_agent_streaming(
            &runtime,
            &mut streaming,
            &task2,
            &writ,
            AgentRunOptions {
                max_steps: req.max_steps,
            },
            cognition_tx,
            Some(approval_requester),
            Some(trace_tx),
        );

        // Race the agent against a cancellation signal.
        let result = tokio::select! {
            biased;
            res = agent_fut => res,
            Ok(_) = cancel_rx.changed() => {
                Err(thymos_core::Error::Other("run cancelled by user".into()))
            }
        };

        // Clean up cancellation token.
        {
            let mut tokens = state2.cancellation_tokens.lock().unwrap();
            tokens.remove(&run_id2);
        }

        // Post-run: emit ledger entries and update the run record.
        let mut runs = state2.runs.lock().unwrap();
        match result {
            Ok(summary) => {
                let traj_id = summary.trajectory_id.0.to_string();

                if let Ok(entries) = runtime.ledger.entries(summary.trajectory_id) {
                    for e in &entries {
                        let dto = entry_to_dto(e);
                        let _ = entry_tx.send(dto);
                    }
                }

                let dto = RunSummaryDto::from(&summary);
                let status = run_status_from_summary(&summary);
                if let Some(rec) = runs.get_mut(&run_id2) {
                    rec.trajectory_id = traj_id.clone();
                    rec.status = status.clone();
                    rec.summary = Some(dto.clone());
                }
                with_execution_session(&state2, &run_id2, &task2, req.max_steps, |session| {
                    session.apply_summary(&summary);
                });
                // Persist to SQLite.
                if let Some(store) = &state2.run_store {
                    let _ = store.update(&run_id2, &traj_id, status_str(&status), Some(&dto));
                }
            }
            Err(e) => {
                let err_dto = RunSummaryDto {
                    steps_executed: 0,
                    intents_submitted: 0,
                    commits: 0,
                    rejections: 0,
                    failures: 1,
                    final_answer: Some(format!("error: {e}")),
                    terminated_by: "Error".into(),
                };
                if let Some(rec) = runs.get_mut(&run_id2) {
                    rec.status = RunStatus::Failed;
                    rec.summary = Some(err_dto.clone());
                }
                with_execution_session(&state2, &run_id2, &task2, req.max_steps, |session| {
                    if e.to_string().contains("run cancelled by user") {
                        session.mark_cancelled();
                    } else {
                        session.mark_failed(format!("error: {e}"));
                    }
                });
                // Persist to SQLite.
                if let Some(store) = &state2.run_store {
                    let _ = store.update(&run_id2, "", "failed", Some(&err_dto));
                }
            }
        }

        state2.active_runs.fetch_sub(1, Ordering::Relaxed);
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "run_id": run_id,
            "task": task,
            "status": "running",
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct ListRunsQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub status: Option<String>,
}

/// GET /runs — list runs with pagination and tenant scoping.
async fn list_runs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: Option<axum::Extension<middleware::GatewayContext>>,
    axum::extract::Query(q): axum::extract::Query<ListRunsQuery>,
) -> impl IntoResponse {
    let caller_tenant = extract_tenant_id(&headers, &jwt_claims, &gateway_ctx);
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    // Filter from in-memory runs.
    let runs = state.runs.lock().unwrap();
    let mut results: Vec<serde_json::Value> = runs
        .iter()
        .filter(|(_, rec)| {
            if !caller_tenant.is_empty() && !tenant_can_access(&rec.tenant_id, &caller_tenant) {
                return false;
            }
            if let Some(ref status_filter) = q.status {
                let rec_status = match rec.status {
                    RunStatus::Running => "running",
                    RunStatus::Completed => "completed",
                    RunStatus::Failed => "failed",
                };
                if rec_status != status_filter.as_str() {
                    return false;
                }
            }
            true
        })
        .map(|(id, rec)| {
            serde_json::json!({
                "run_id": id,
                "task": rec.task,
                "status": rec.status,
                "trajectory_id": rec.trajectory_id,
                "tenant_id": rec.tenant_id,
            })
        })
        .collect();

    // Simple pagination on the collected results.
    let total = results.len();
    results = results
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "runs": results,
            "total": total,
            "limit": limit,
            "offset": offset,
        })),
    )
        .into_response()
}

/// GET /runs/:id — get run status and summary.
async fn get_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: Option<axum::Extension<middleware::GatewayContext>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let caller_tenant = extract_tenant_id(&headers, &jwt_claims, &gateway_ctx);

    // Check in-memory cache first.
    {
        let runs = state.runs.lock().unwrap();
        if let Some(rec) = runs.get(&id) {
            if !tenant_can_access(&rec.tenant_id, &caller_tenant) {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "run not found" })),
                )
                    .into_response();
            }
            return (StatusCode::OK, Json(serde_json::to_value(rec).unwrap())).into_response();
        }
    }
    // Fall back to persistent store.
    if let Some(store) = &state.run_store {
        if let Ok(Some(rec)) = store.get(&id) {
            return (StatusCode::OK, Json(serde_json::to_value(rec).unwrap())).into_response();
        }
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "run not found" })),
    )
        .into_response()
}

/// GET /runs/:id/execution — get the unified execution session snapshot.
async fn get_execution(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: Option<axum::Extension<middleware::GatewayContext>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let caller_tenant = extract_tenant_id(&headers, &jwt_claims, &gateway_ctx);

    if let Some(session) = state.execution_sessions.lock().unwrap().get(&id).cloned() {
        return (StatusCode::OK, Json(serde_json::to_value(session).unwrap())).into_response();
    }

    {
        let runs = state.runs.lock().unwrap();
        if let Some(rec) = runs.get(&id) {
            if !tenant_can_access(&rec.tenant_id, &caller_tenant) {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "run not found" })),
                )
                    .into_response();
            }
            let session = execution::ExecutionSession::from_run_record(&id, rec);
            return (StatusCode::OK, Json(serde_json::to_value(session).unwrap())).into_response();
        }
    }

    if let Some(store) = &state.run_store {
        if let Ok(Some(rec)) = store.get(&id) {
            let session = execution::ExecutionSession::from_run_record(&id, &rec);
            return (StatusCode::OK, Json(serde_json::to_value(session).unwrap())).into_response();
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "run not found" })),
    )
        .into_response()
}

/// GET /runs/:id/execution/stream — SSE stream of unified execution session snapshots.
async fn get_execution_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: Option<axum::Extension<middleware::GatewayContext>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let caller_tenant = extract_tenant_id(&headers, &jwt_claims, &gateway_ctx);
    {
        let runs = state.runs.lock().unwrap();
        if let Some(rec) = runs.get(&id) {
            if !tenant_can_access(&rec.tenant_id, &caller_tenant) {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "run not found" })),
                )
                    .into_response();
            }
        }
    }

    let initial = state
        .execution_sessions
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .or_else(|| {
            let runs = state.runs.lock().unwrap();
            runs.get(&id)
                .map(|rec| execution::ExecutionSession::from_run_record(&id, rec))
        });

    let rx = {
        let channels = state.execution_channels.lock().unwrap();
        channels.get(&id).map(|tx| tx.subscribe())
    };

    match (initial, rx) {
        (Some(initial_session), Some(mut rx)) => {
            let stream = async_stream::stream! {
                let initial_data = serde_json::to_string(&initial_session).unwrap_or_default();
                yield Ok::<_, std::convert::Infallible>(Event::default().event("snapshot").data(initial_data));
                while let Ok(session) = rx.recv().await {
                    let data = serde_json::to_string(&session).unwrap_or_default();
                    yield Ok::<_, std::convert::Infallible>(Event::default().event("snapshot").data(data));
                }
            };
            Sse::new(stream).into_response()
        }
        (Some(initial_session), None) => {
            let stream = async_stream::stream! {
                let initial_data = serde_json::to_string(&initial_session).unwrap_or_default();
                yield Ok::<_, std::convert::Infallible>(Event::default().event("snapshot").data(initial_data));
            };
            Sse::new(stream).into_response()
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found" })),
        )
            .into_response(),
    }
}

/// GET /runs/:id/events — SSE stream of trajectory entries (ledger events).
async fn get_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let rx = {
        let channels = state.event_channels.lock().unwrap();
        channels.get(&id).map(|tx| tx.subscribe())
    };

    match rx {
        Some(mut rx) => {
            let stream = async_stream::stream! {
                while let Ok(entry) = rx.recv().await {
                    let data = serde_json::to_string(&entry).unwrap_or_default();
                    yield Ok::<_, std::convert::Infallible>(Event::default().data(data));
                }
            };
            Sse::new(stream).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found" })),
        )
            .into_response(),
    }
}

/// GET /runs/:id/stream — SSE stream of cognition events (tokens, tool-use deltas).
async fn get_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let rx = {
        let channels = state.cognition_channels.lock().unwrap();
        channels.get(&id).map(|tx| tx.subscribe())
    };

    match rx {
        Some(mut rx) => {
            let stream = async_stream::stream! {
                while let Ok(evt) = rx.recv().await {
                    let data = serde_json::to_string(&evt).unwrap_or_default();
                    yield Ok::<_, std::convert::Infallible>(
                        Event::default()
                            .event(event_type(&evt))
                            .data(data)
                    );
                }
            };
            Sse::new(stream).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found" })),
        )
            .into_response(),
    }
}

/// Map a CognitionEvent to an SSE event type name for client-side filtering.
fn event_type(evt: &CognitionEvent) -> &'static str {
    match evt {
        CognitionEvent::Token { .. } => "token",
        CognitionEvent::ToolUseStart { .. } => "tool_use_start",
        CognitionEvent::ToolUseArgDelta { .. } => "tool_use_arg_delta",
        CognitionEvent::ToolUseDone { .. } => "tool_use_done",
        CognitionEvent::TurnComplete { .. } => "turn_complete",
        CognitionEvent::Error { .. } => "error",
    }
}

/// GET /runs/:id/world — current world projection.
async fn get_world(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let traj_id = {
        let runs = state.runs.lock().unwrap();
        runs.get(&id).map(|r| r.trajectory_id.clone())
    };

    let traj_id = match traj_id {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "run not found or not started" })),
            )
                .into_response()
        }
    };

    let traj_bytes = match hex::decode(&traj_id) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid trajectory id" })),
            )
                .into_response()
        }
    };

    let trajectory_id = TrajectoryId(thymos_core::ContentHash(traj_bytes));
    let runtime = state.runtime.clone();

    let result = tokio::task::spawn_blocking(move || {
        let run = runtime.resume_run(trajectory_id)?;
        let world = run.project_world()?;
        let resources: Vec<ResourceDto> = world
            .resources
            .iter()
            .map(|(k, v)| ResourceDto {
                kind: k.kind.clone(),
                id: k.id.clone(),
                version: v.version,
                value: v.value.clone(),
            })
            .collect();
        Ok::<_, thymos_core::Error>(WorldDto { resources })
    })
    .await;

    match result {
        Ok(Ok(dto)) => (StatusCode::OK, Json(serde_json::to_value(dto).unwrap())).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /runs/:id/world/at?seq=N — replay the trajectory up to `seq` and
/// return the world projection at that commit. Used by the web debugger
/// scrubber.
#[derive(Deserialize)]
struct WorldAtQuery {
    seq: u64,
}

async fn get_world_at(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<WorldAtQuery>,
) -> impl IntoResponse {
    let traj_hex = {
        let runs = state.runs.lock().unwrap();
        runs.get(&id).map(|r| r.trajectory_id.clone())
    };
    let traj_hex = match traj_hex {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "run not found or not started" })),
            )
                .into_response()
        }
    };

    let traj_bytes = match hex::decode(&traj_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid trajectory id" })),
            )
                .into_response()
        }
    };
    let trajectory_id = TrajectoryId(thymos_core::ContentHash(traj_bytes));

    let runtime = state.runtime.clone();
    let until_seq = q.seq;
    let result = tokio::task::spawn_blocking(move || {
        let mut entries = runtime.ledger.entries(trajectory_id)?;
        entries.retain(|e| e.seq <= until_seq);
        let (world, report) =
            thymos_ledger::replay(&entries, &thymos_ledger::ReplayConfig::default())?;
        let resources: Vec<ResourceDto> = world
            .resources
            .iter()
            .map(|(k, v)| ResourceDto {
                kind: k.kind.clone(),
                id: k.id.clone(),
                version: v.version,
                value: v.value.clone(),
            })
            .collect();
        Ok::<_, thymos_core::Error>(serde_json::json!({
            "seq": report.head_seq,
            "commits_replayed": report.commits_replayed,
            "head_commit": report.head_commit,
            "resources": resources,
        }))
    })
    .await;

    match result {
        Ok(Ok(v)) => (StatusCode::OK, Json(v)).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ReplayQuery {
    require_compiler: Option<String>,
}

/// GET /runs/:id/replay — verify the run ledger and fold committed deltas.
async fn get_replay(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    jwt_claims: Option<axum::Extension<auth::JwtClaims>>,
    gateway_ctx: Option<axum::Extension<middleware::GatewayContext>>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<ReplayQuery>,
) -> impl IntoResponse {
    let caller_tenant = extract_tenant_id(&headers, &jwt_claims, &gateway_ctx);
    let record = match run_record_for_access(&state, &id, &caller_tenant) {
        Ok(record) => record,
        Err(status) => {
            return (
                status,
                Json(serde_json::json!({ "error": "run not found" })),
            )
                .into_response()
        }
    };

    if record.trajectory_id.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found or not started" })),
        )
            .into_response();
    }

    let trajectory_id = match trajectory_from_hex(&record.trajectory_id) {
        Ok(trajectory_id) => trajectory_id,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response()
        }
    };

    let runtime = state.runtime.clone();
    let run_id = id.clone();
    let require_compiler = q.require_compiler.clone();
    let result = tokio::task::spawn_blocking(move || {
        let entries = runtime.ledger.entries(trajectory_id)?;
        let cfg = thymos_ledger::ReplayConfig {
            require_compiler_version: require_compiler,
            ..Default::default()
        };
        let (world, report) = thymos_ledger::replay(&entries, &cfg)?;
        let final_world_hash = content_hash(&world)?.to_string();

        let mut rejected_proposals = 0usize;
        let mut pending_approvals = 0usize;
        let mut delegations = 0usize;
        let mut branches = 0usize;
        let mut tool_calls = Vec::new();

        for entry in &entries {
            match &entry.payload {
                EntryPayload::Commit(commit) => {
                    for observation in &commit.body.observations {
                        tool_calls.push(ReplayToolCallDto {
                            seq: entry.seq,
                            commit_id: commit.id.to_string(),
                            tool: observation.tool.clone(),
                            latency_ms: observation.latency_ms,
                        });
                    }
                }
                EntryPayload::Rejection { .. } => rejected_proposals += 1,
                EntryPayload::PendingApproval { .. } => pending_approvals += 1,
                EntryPayload::Delegation { .. } => delegations += 1,
                EntryPayload::Branch { .. } => branches += 1,
                EntryPayload::Root { .. } => {}
            }
        }

        Ok::<_, thymos_core::Error>(ReplayDto {
            run_id,
            trajectory_id: report.trajectory_id,
            entries_seen: report.entries_seen,
            commits_replayed: report.commits_replayed,
            head_commit: report.head_commit,
            head_seq: report.head_seq,
            compiler_versions_seen: report.compiler_versions_seen,
            final_world_hash,
            resources: world.resources.len(),
            rejected_proposals,
            pending_approvals,
            delegations,
            branches,
            tool_calls,
        })
    })
    .await;

    match result {
        Ok(Ok(report)) => {
            (StatusCode::OK, Json(serde_json::to_value(report).unwrap())).into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /runs/:id/approvals/:channel — approve or deny a pending proposal.
async fn post_approval(
    State(state): State<Arc<AppState>>,
    Path((id, channel)): Path<(String, String)>,
    Json(req): Json<ApprovalRequest>,
) -> impl IntoResponse {
    let key = (id.clone(), channel.clone());
    let sender = {
        let mut pending = state.pending_approvals.lock().unwrap();
        pending.remove(&key)
    };

    match sender {
        Some(tx) => {
            let decision = ApprovalDecision {
                approve: req.approve,
            };
            match tx.send(decision) {
                Ok(()) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "run_id": id,
                        "channel": channel,
                        "approved": req.approve,
                    })),
                )
                    .into_response(),
                Err(_) => (
                    StatusCode::GONE,
                    Json(serde_json::json!({ "error": "agent loop already terminated" })),
                )
                    .into_response(),
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "no pending approval for this run/channel",
                "run_id": id,
                "channel": channel,
            })),
        )
            .into_response(),
    }
}

/// POST /runs/:id/cancel — cancel a running agent.
async fn cancel_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Check the run exists and is running.
    {
        let runs = state.runs.lock().unwrap();
        match runs.get(&id) {
            Some(rec) if rec.status == RunStatus::Running => {}
            Some(_) => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": "run is not in running state" })),
                )
                    .into_response()
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "run not found" })),
                )
                    .into_response()
            }
        }
    }

    // Send cancellation signal.
    let token = {
        let tokens = state.cancellation_tokens.lock().unwrap();
        tokens.get(&id).cloned()
    };

    match token {
        Some(tx) => {
            let _ = tx.send(true);
            let task = {
                let runs = state.runs.lock().unwrap();
                runs.get(&id)
                    .map(|rec| rec.task.clone())
                    .unwrap_or_else(|| "cancelled run".into())
            };
            with_execution_session(&state, &id, &task, 1, |session| {
                session.mark_cancelled();
            });
            (
                StatusCode::OK,
                Json(serde_json::json!({ "run_id": id, "status": "cancelling" })),
            )
                .into_response()
        }
        None => (
            StatusCode::GONE,
            Json(serde_json::json!({ "error": "run has already finished" })),
        )
            .into_response(),
    }
}

/// POST /runs/:id/branch — create a shadow (counterfactual) branch from a commit.
///
/// Body: `{ "commit_id": "<64 hex>", "note": "why branching" }`
/// Returns the new branch trajectory id; the branch starts with world state
/// as of `commit_id` and has its own independent history going forward.
#[derive(Deserialize)]
struct BranchRequest {
    commit_id: String,
    #[serde(default)]
    note: Option<String>,
}

async fn post_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<BranchRequest>,
) -> impl IntoResponse {
    let traj_hex = {
        let runs = state.runs.lock().unwrap();
        runs.get(&id).map(|r| r.trajectory_id.clone())
    };
    let traj_hex = match traj_hex {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "run not found or not started" })),
            )
                .into_response()
        }
    };

    let traj_bytes = match hex::decode(&traj_hex) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid trajectory id" })),
            )
                .into_response()
        }
    };
    let trajectory_id = TrajectoryId(thymos_core::ContentHash(traj_bytes));

    let commit_bytes = match hex::decode(body.commit_id.trim()) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "commit_id must be 64 hex chars" })),
            )
                .into_response()
        }
    };
    let commit_id = thymos_core::CommitId(thymos_core::ContentHash(commit_bytes));
    let note = body.note.unwrap_or_else(|| "shadow branch".to_string());

    let run = match state.runtime.resume_run(trajectory_id) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    match run.branch_from(commit_id, &note) {
        Ok(branch) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "branch_trajectory_id": branch.trajectory_id().to_string(),
                "source_trajectory_id": trajectory_id.to_string(),
                "source_commit_id": commit_id.to_string(),
                "note": note,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /runs/:id/delegations — list child trajectories created via delegation.
/// Read-only: the safe routing-feedback records for a run (decision_hash,
/// selected route, status, latency only — never workload content). A pull, not
/// a push: an advisor fetches these and decides what to do with them; THYMOS
/// initiates no egress.
async fn get_routing_outcomes(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Resolve to a trajectory: a known run id maps to its trajectory; otherwise
    // the path is treated as a trajectory hex directly (e.g. the trajectory_id
    // returned by /routed-submit, which is not tracked in the run store).
    let traj_hex = {
        let runs = state.runs.lock().unwrap();
        runs.get(&id)
            .map(|r| r.trajectory_id.clone())
            .filter(|t| !t.is_empty())
    }
    .unwrap_or_else(|| id.clone());
    let trajectory_id = match trajectory_from_hex(&traj_hex) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response()
        }
    };
    match state.runtime.ledger.entries(trajectory_id) {
        Ok(entries) => (
            StatusCode::OK,
            Json(serde_json::json!({ "outcomes": routing_outcomes(&entries) })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn get_delegations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let traj_id = {
        let runs = state.runs.lock().unwrap();
        runs.get(&id).map(|r| r.trajectory_id.clone())
    };

    let traj_id = match traj_id {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "run not found or not started" })),
            )
                .into_response()
        }
    };

    let traj_bytes = match hex::decode(&traj_id) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid trajectory id" })),
            )
                .into_response()
        }
    };

    let trajectory_id = TrajectoryId(thymos_core::ContentHash(traj_bytes));

    match state.runtime.ledger.entries(trajectory_id) {
        Ok(entries) => {
            let delegations: Vec<serde_json::Value> = entries
                .iter()
                .filter_map(|e| {
                    if let EntryPayload::Delegation {
                        child_trajectory_id,
                        task,
                        final_answer,
                    } = &e.payload
                    {
                        Some(serde_json::json!({
                            "child_trajectory_id": child_trajectory_id.to_string(),
                            "task": task,
                            "final_answer": final_answer,
                            "seq": e.seq,
                        }))
                    } else {
                        None
                    }
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "delegations": delegations })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

fn entry_to_dto(e: &thymos_ledger::Entry) -> EntryDto {
    let (kind, detail) = match &e.payload {
        EntryPayload::Root { note, .. } => ("root".into(), serde_json::json!({ "note": note })),
        EntryPayload::Commit(c) => (
            "commit".into(),
            serde_json::json!({
                "proposal_id": c.body.proposal_id.to_string(),
                "delta_ops": c.body.delta.0.len(),
                "observations": c.body.observations.len(),
            }),
        ),
        EntryPayload::Rejection { intent_id, reason } => (
            "rejection".into(),
            serde_json::json!({
                "intent_id": intent_id.to_string(),
                "reason": format!("{:?}", reason),
            }),
        ),
        EntryPayload::PendingApproval {
            channel,
            reason,
            proposal,
        } => (
            "pending_approval".into(),
            serde_json::json!({
                "channel": channel,
                "reason": reason,
                "proposal": serde_json::to_value(proposal).ok(),
            }),
        ),
        EntryPayload::Delegation {
            child_trajectory_id,
            task,
            final_answer,
        } => (
            "delegation".into(),
            serde_json::json!({
                "child_trajectory_id": child_trajectory_id.to_string(),
                "task": task,
                "final_answer": final_answer,
            }),
        ),
        EntryPayload::Branch {
            source_trajectory_id,
            source_commit_id,
            note,
        } => (
            "branch".into(),
            serde_json::json!({
                "source_trajectory_id": source_trajectory_id.to_string(),
                "source_commit_id": source_commit_id.to_string(),
                "note": note,
            }),
        ),
    };

    let commit_id = match &e.payload {
        EntryPayload::Commit(c) => Some(c.id.to_string()),
        _ => None,
    };

    EntryDto {
        seq: e.seq,
        kind,
        id: e.id.short().to_string(),
        detail,
        commit_id,
    }
}

#[cfg(test)]
mod onboarding_tests {
    use super::*;

    #[test]
    fn provider_labels_are_stable() {
        assert_eq!(provider_label(&CognitionProvider::Anthropic), "anthropic");
        assert_eq!(provider_label(&CognitionProvider::Openai), "openai");
        assert_eq!(provider_label(&CognitionProvider::Local), "local");
        assert_eq!(provider_label(&CognitionProvider::Lmstudio), "lmstudio");
        assert_eq!(provider_label(&CognitionProvider::Huggingface), "huggingface");
        assert_eq!(provider_label(&CognitionProvider::Mock), "mock");
    }

    #[test]
    fn explicit_default_provider_env_wins_and_is_case_insensitive() {
        // An explicit THYMOS_DEFAULT_PROVIDER short-circuits key auto-detection,
        // so this is deterministic regardless of any API keys in the env.
        let prev = std::env::var("THYMOS_DEFAULT_PROVIDER").ok();
        let prev_model = std::env::var("THYMOS_DEFAULT_MODEL").ok();

        std::env::set_var("THYMOS_DEFAULT_PROVIDER", "  OpenAI ");
        std::env::set_var("THYMOS_DEFAULT_MODEL", "gpt-4o-mini");
        let cfg = resolve_default_cognition();
        assert_eq!(cfg.provider, CognitionProvider::Openai);
        assert_eq!(cfg.model.as_deref(), Some("gpt-4o-mini"));

        std::env::set_var("THYMOS_DEFAULT_PROVIDER", "definitely-not-a-provider");
        assert_eq!(
            resolve_default_cognition().provider,
            CognitionProvider::Mock,
            "unknown provider falls back to mock"
        );

        // Restore prior env so we don't perturb other tests.
        match prev {
            Some(v) => std::env::set_var("THYMOS_DEFAULT_PROVIDER", v),
            None => std::env::remove_var("THYMOS_DEFAULT_PROVIDER"),
        }
        match prev_model {
            Some(v) => std::env::set_var("THYMOS_DEFAULT_MODEL", v),
            None => std::env::remove_var("THYMOS_DEFAULT_MODEL"),
        }
    }
}
