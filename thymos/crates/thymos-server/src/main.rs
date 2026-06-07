//! Thymos server binary.
//!
//! Run:
//!     cargo run -p thymos-server
//! or with an LLM:
//!     ANTHROPIC_API_KEY=sk-ant-... cargo run -p thymos-server
//!
//! Then:
//!     curl -X POST http://localhost:3001/runs \
//!       -H 'content-type: application/json' \
//!       -d '{"task": "Set greeting to hello and read it back"}'

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use thymos_server::{
    app, auth, default_runtime_with_capabilities, middleware, persistent_runtime_with_capabilities,
    provider_label, run_store, telemetry, AppState, CognitionProvider, RunStatus, RunSummaryDto,
    RuntimeMode, ServerConfig,
};

#[tokio::main]
async fn main() {
    // Load .env from CWD or any parent dir before reading env-driven config.
    // Never overrides an already-exported variable.
    let _ = dotenvy::dotenv();

    telemetry::init();

    let config = match ServerConfig::from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("configuration error: {err}");
            std::process::exit(2);
        }
    };


    // Be loud about the default cognition provider. The biggest deployment
    // footgun is running with the silent mock and assuming real cognition.
    if matches!(config.default_cognition.provider, CognitionProvider::Mock) {
        eprintln!(
            "WARNING: default cognition provider is MOCK — runs that omit their own `cognition` block return canned, deterministic output, NOT a real model. Set ANTHROPIC_API_KEY / OPENAI_API_KEY or THYMOS_DEFAULT_PROVIDER to use a live model. (/health reports cognition_live=false)"
        );
    } else {
        eprintln!(
            "cognition: default provider = {} (live){}",
            provider_label(&config.default_cognition.provider),
            config
                .default_cognition
                .model
                .as_deref()
                .map(|m| format!(", model = {m}"))
                .unwrap_or_default()
        );
    }

    // Backend selection: Postgres (if configured and this binary was built with
    // the `postgres` feature) → SQLite file → in-memory.
    let runtime = if let Some(url) = config.postgres_url.as_deref() {
        #[cfg(feature = "postgres")]
        {
            eprintln!("ledger: postgres (synchronous blocking facade) at {url}");
            thymos_server::postgres_runtime_with_capabilities(url, &config.tool_manifest_dirs)
        }
        #[cfg(not(feature = "postgres"))]
        {
            eprintln!(
                "note: THYMOS_POSTGRES_URL is set to '{url}', but this binary was built without the `postgres` feature; falling back to SQLite. Rebuild with `--features postgres` to use Postgres."
            );
            if let Some(path) = &config.ledger_path {
                eprintln!("ledger: sqlite file-backed at {path}");
                persistent_runtime_with_capabilities(path, &config.tool_manifest_dirs)
            } else {
                eprintln!("ledger: in-memory reference mode");
                default_runtime_with_capabilities(&config.tool_manifest_dirs)
            }
        }
    } else if let Some(path) = &config.ledger_path {
        eprintln!("ledger: sqlite file-backed at {path}");
        persistent_runtime_with_capabilities(path, &config.tool_manifest_dirs)
    } else {
        eprintln!("ledger: in-memory reference mode");
        default_runtime_with_capabilities(&config.tool_manifest_dirs)
    };

    // Optional: configure API gateway from environment.
    let gateway_store = match middleware::ApiKeyStore::open(&config.gateway_db_path) {
        Ok(store) => {
            eprintln!("api gateway store: {}", config.gateway_db_path);
            Some(Arc::new(store))
        }
        Err(e) => {
            eprintln!(
                "warn: failed to open API gateway store at {}: {}",
                config.gateway_db_path, e
            );
            None
        }
    };
    let gateway = {
        let mut gw = match &gateway_store {
            Some(store) => match middleware::ApiGateway::new().with_store(store.clone()) {
                Ok(gw) => gw,
                Err(e) => {
                    eprintln!("warn: failed to load persisted API keys: {e}");
                    middleware::ApiGateway::new()
                }
            },
            None => middleware::ApiGateway::new(),
        };

        if let Ok(keys_str) = std::env::var("THYMOS_API_KEYS") {
            for entry in keys_str.split(',').filter(|s| !s.trim().is_empty()) {
                let parts: Vec<&str> = entry.split(':').collect();
                if parts.len() >= 4 {
                    if let Err(err) = gw.add_key(middleware::ApiKey {
                        key: parts[0].to_string(),
                        tenant_id: parts[1].to_string(),
                        name: parts[2].to_string(),
                        rate_limit_rpm: parts[3].parse().unwrap_or(60),
                    }) {
                        eprintln!("warn: failed to persist API key '{}': {err}", parts[2]);
                    }
                }
            }
        }

        // Enable the auth middleware only when there are actually keys to
        // check against. A bare `gateway_store.is_some()` check used to enable
        // auth as soon as the sqlite file was created, which locked everyone
        // out of a fresh dev server until they set THYMOS_API_KEYS.
        if gw.usage_stats().is_empty() {
            None
        } else {
            eprintln!("API gateway enabled ({} key(s))", gw.usage_stats().len());
            Some(Arc::new(gw))
        }
    };

    // Persistent run store. Use THYMOS_DB_PATH or default to ./thymos-runs.db.
    let run_store = match run_store::RunStore::open(&config.run_db_path) {
        Ok(store) => {
            eprintln!("run store: {}", config.run_db_path);
            Some(Arc::new(store))
        }
        Err(e) => match config.runtime_mode {
            RuntimeMode::Production => {
                eprintln!(
                    "fatal: failed to open run store at {}: {e}",
                    config.run_db_path
                );
                std::process::exit(2);
            }
            RuntimeMode::Reference => {
                eprintln!(
                    "warn: failed to open run store at {}: {e}",
                    config.run_db_path
                );
                None
            }
        },
    };

    let marketplace =
        match thymos_marketplace::MarketplaceService::open_sqlite(&config.marketplace_db_path) {
            Ok(service) => {
                eprintln!("marketplace store: {}", config.marketplace_db_path);
                Arc::new(service)
            }
            Err(e) => match config.runtime_mode {
                RuntimeMode::Production => {
                    eprintln!(
                        "fatal: failed to open marketplace store at {}: {}",
                        config.marketplace_db_path, e
                    );
                    std::process::exit(2);
                }
                RuntimeMode::Reference => {
                    eprintln!(
                        "warn: failed to open marketplace store at {}: {}",
                        config.marketplace_db_path, e
                    );
                    Arc::new(thymos_marketplace::MarketplaceService::in_memory())
                }
            },
        };

    // Restore previously persisted runs into memory.
    let mut restored_runs = HashMap::new();
    if let Some(store) = &run_store {
        if let Ok(all) = store.load_all() {
            eprintln!("restored {} runs from disk", all.len());
            for (id, rec) in all {
                restored_runs.insert(id, rec);
            }
        }
    }

    // Optional: JWT auth from THYMOS_JWT_SECRET.
    let jwt_config = std::env::var("THYMOS_JWT_SECRET").ok().map(|secret| {
        eprintln!("JWT auth enabled");
        Arc::new(auth::JwtConfig::from_secret(secret.as_bytes()))
    });

    let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);

    let state = Arc::new(AppState {
        runtime_mode: config.runtime_mode,
        cors_allowed_origins: config.cors_allowed_origins.clone(),
        max_concurrent_runs_per_tenant: config.max_concurrent_runs_per_tenant,
        max_concurrent_runs_global: config.max_concurrent_runs_global,
        runtime,
        runs: Mutex::new(restored_runs),
        event_channels: Mutex::new(HashMap::new()),
        cognition_channels: Mutex::new(HashMap::new()),
        execution_sessions: Mutex::new(HashMap::new()),
        execution_channels: Mutex::new(HashMap::new()),
        gateway,
        jwt_config,
        pending_approvals: Mutex::new(HashMap::new()),
        cancellation_tokens: Mutex::new(HashMap::new()),
        run_store,
        shutdown_tx,
        active_runs: AtomicU32::new(0),
        marketplace,
        default_cognition: config.default_cognition.clone(),
        skills: Arc::new(thymos_server::skills::SkillRegistry::new(Some(
            std::path::PathBuf::from(
                std::env::var("THYMOS_SKILLS_DIR").unwrap_or_else(|_| "thymos-skills".into()),
            ),
        ))),
    });

    eprintln!(
        "default cognition provider: {} (runs without a `cognition` block use this)",
        thymos_server::provider_label(&config.default_cognition.provider)
    );

    let app = app(state.clone());
    eprintln!(
        "thymos-server listening on {} ({})",
        config.bind_addr,
        match config.runtime_mode {
            RuntimeMode::Reference => "reference mode",
            RuntimeMode::Production => "production mode",
        }
    );

    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .expect("bind failed");

    // Graceful shutdown: listen for SIGTERM/SIGINT.
    let state_for_shutdown = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("install SIGTERM handler");
            #[cfg(unix)]
            let terminate = sigterm.recv();
            #[cfg(not(unix))]
            let terminate = std::future::pending::<Option<()>>();

            tokio::select! {
                _ = ctrl_c => eprintln!("\nreceived SIGINT, shutting down..."),
                _ = terminate => eprintln!("\nreceived SIGTERM, shutting down..."),
            }

            // Signal all run handlers to stop accepting new work.
            let _ = state_for_shutdown.shutdown_tx.send(true);
            {
                let tokens = state_for_shutdown.cancellation_tokens.lock().unwrap();
                for sender in tokens.values() {
                    let _ = sender.send(true);
                }
            }

            // Wait for active runs to drain (up to 30 seconds).
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                let active = state_for_shutdown.active_runs.load(Ordering::Relaxed);
                if active == 0 {
                    eprintln!("all runs drained, exiting cleanly");
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    eprintln!(
                        "shutdown timeout: {} runs still active, forcing exit",
                        active
                    );
                    // Mark running runs as failed so they can be resumed.
                    let mut runs = state_for_shutdown.runs.lock().unwrap();
                    for (run_id, rec) in runs.iter_mut() {
                        if rec.status == thymos_server::RunStatus::Running {
                            rec.status = RunStatus::Failed;
                            let summary = RunSummaryDto {
                                steps_executed: 0,
                                intents_submitted: 0,
                                commits: 0,
                                rejections: 0,
                                failures: 1,
                                final_answer: Some(
                                    "server shutdown timed out while the run was still active"
                                        .into(),
                                ),
                                terminated_by: "ShutdownTimeout".into(),
                            };
                            rec.summary = Some(summary.clone());
                            if let Some(store) = &state_for_shutdown.run_store {
                                let _ = store.update(
                                    run_id,
                                    &rec.trajectory_id,
                                    "failed",
                                    Some(&summary),
                                );
                            }
                            if let Some(session) = state_for_shutdown
                                .execution_sessions
                                .lock()
                                .unwrap()
                                .get_mut(run_id)
                            {
                                session.mark_failed(
                                    "server shutdown timed out while the run was still active",
                                );
                            }
                        }
                    }
                    break;
                }
                eprintln!("waiting for {} active run(s) to complete...", active);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        })
        .await
        .expect("server failed");
}
