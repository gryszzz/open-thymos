//! OpenThymos Desktop — Tauri host process.
//!
//! Design rule (see `docs/rfcs/desktop-app.md`): this process is a **supervisor
//! and a client**, never an executor. It starts/stops a local `thymos-server`
//! child and the webview talks to that server over HTTP/SSE. It does **not**
//! call tools, mutate world state, or spend budget — all of that stays inside
//! the governed `Intent → Proposal → Commit` pipeline in the runtime. The only
//! network egress is to the local runtime and (server-side) the provider the
//! user configured. No analytics, no phone-home.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

/// Address the supervised runtime listens on. The webview's CSP only allows
/// connections here (see `tauri.conf.json`).
const RUNTIME_ADDR: &str = "http://127.0.0.1:3001";

/// Holds the supervised `thymos-server` child, if running.
#[derive(Default)]
struct Supervisor(Mutex<Option<Child>>);

/// Resolve the `thymos-server` binary. Prefer a sidecar shipped next to the app
/// executable (what the bundler installs); fall back to the name on `PATH` for
/// `cargo tauri dev` against a `cargo install`-ed server.
fn server_binary() -> PathBuf {
    let name = if cfg!(windows) {
        "thymos-server.exe"
    } else {
        "thymos-server"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sidecar = dir.join(name);
            if sidecar.exists() {
                return sidecar;
            }
        }
    }
    PathBuf::from(name)
}

#[tauri::command]
fn runtime_addr() -> String {
    RUNTIME_ADDR.to_string()
}

/// User-chosen cognition provider, persisted in the app-data dir as plain JSON
/// (`provider.json`). Same trust model as the CLI's `.env`: the key lives on the
/// user's machine, is injected only into the **local** runtime child at spawn,
/// and is never returned to the webview — `get_provider_config` reports only
/// whether a key is stored.
#[derive(Default, Serialize, Deserialize)]
struct ProviderConfig {
    /// `anthropic`, `openai`, `mock`, or any preset id (`ollama`, `groq`,
    /// `openrouter`, `lmstudio`, `huggingface`, …) — or a custom OpenAI-
    /// compatible adapter via provider `openai` + a `base_url`.
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    /// Base URL for OpenAI-compatible / self-hosted adapters (e.g. Ollama,
    /// LM Studio, vLLM, a corporate gateway). Empty = use the provider default.
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
}

fn provider_config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app data dir: {e}"))?;
    Ok(dir.join("provider.json"))
}

fn load_provider_config(app: &tauri::AppHandle) -> ProviderConfig {
    provider_config_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// File holding the operator-chosen working folder (the agent's sandbox root).
fn workspace_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("workspace.txt"))
}
fn load_workspace(app: &tauri::AppHandle) -> Option<String> {
    workspace_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && std::path::Path::new(s).is_dir())
}

/// Current working folder the agent is allowed to read/edit (empty = none yet).
#[tauri::command]
fn get_workspace(app: tauri::AppHandle) -> String {
    load_workspace(&app).unwrap_or_default()
}

/// Open a native folder picker; persist + return the chosen path (empty if the
/// user cancelled). Changing it takes effect on the next runtime (re)start.
#[tauri::command]
fn pick_workspace(app: tauri::AppHandle) -> Result<String, String> {
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Choose the folder OpenThymos may read and edit")
        .pick_folder()
    else {
        return Ok(String::new()); // cancelled
    };
    let path = dir.display().to_string();
    if let Some(p) = workspace_path(&app) {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        std::fs::write(&p, &path).map_err(|e| format!("save workspace: {e}"))?;
    }
    Ok(path)
}

/// Forget the working folder (the agent falls back to no project root).
#[tauri::command]
fn clear_workspace(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(p) = workspace_path(&app) {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

/// Persisted opt-in for token streaming (experimental). Passed to the runtime
/// as THYMOS_STREAM on spawn; takes effect on the next runtime restart.
fn streaming_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("streaming.on"))
}
fn streaming_enabled(app: &tauri::AppHandle) -> bool {
    streaming_path(app).map(|p| p.exists()).unwrap_or(false)
}
#[tauri::command]
fn get_streaming(app: tauri::AppHandle) -> bool {
    streaming_enabled(&app)
}
#[tauri::command]
fn set_streaming(app: tauri::AppHandle, on: bool) -> Result<(), String> {
    if let Some(p) = streaming_path(&app) {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        if on {
            std::fs::write(&p, "1").map_err(|e| format!("save: {e}"))?;
        } else {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(())
}

/// Inject the stored provider as env vars on the runtime child. Anthropic uses
/// its dedicated key/base-URL vars; everything else (native `openai` plus every
/// OpenAI-compatible preset) resolves the generic `OPENAI_API_KEY` /
/// `OPENAI_BASE_URL` fallback in `resolve_default_cognition` + the preset
/// registry — so a single code path covers any adapter.
fn apply_provider_env(cmd: &mut Command, cfg: &ProviderConfig) {
    let anthropic = cfg.provider.eq_ignore_ascii_case("anthropic");
    if !cfg.provider.is_empty() {
        cmd.env("THYMOS_DEFAULT_PROVIDER", &cfg.provider);
    }
    if !cfg.model.is_empty() {
        cmd.env("THYMOS_DEFAULT_MODEL", &cfg.model);
    }
    if !cfg.api_key.is_empty() {
        cmd.env(
            if anthropic {
                "ANTHROPIC_API_KEY"
            } else {
                "OPENAI_API_KEY"
            },
            &cfg.api_key,
        );
    }
    if !cfg.base_url.is_empty() {
        cmd.env(
            if anthropic {
                "ANTHROPIC_BASE_URL"
            } else {
                "OPENAI_BASE_URL"
            },
            &cfg.base_url,
        );
    }
}

/// Current provider config for the Settings UI. Never returns the key itself —
/// only whether one is stored — so the secret never round-trips to the webview.
#[tauri::command]
fn get_provider_config(app: tauri::AppHandle) -> serde_json::Value {
    let cfg = load_provider_config(&app);
    serde_json::json!({
        "provider": cfg.provider,
        "model": cfg.model,
        "base_url": cfg.base_url,
        "key_set": !cfg.api_key.is_empty(),
    })
}

/// Persist the chosen provider/model/base-URL/key. An empty `api_key` leaves any
/// stored key untouched (so editing the model doesn't wipe the secret); pass
/// whitespace to clear it. The caller restarts the runtime to apply.
#[tauri::command]
fn set_provider_config(
    app: tauri::AppHandle,
    provider: String,
    model: String,
    base_url: String,
    api_key: String,
) -> Result<(), String> {
    let mut cfg = load_provider_config(&app);
    cfg.provider = provider.trim().to_string();
    cfg.model = model.trim().to_string();
    cfg.base_url = base_url.trim().to_string();
    if !api_key.is_empty() {
        // Real key stored trimmed; pure whitespace clears it.
        cfg.api_key = api_key.trim().to_string();
    }
    let path = provider_config_path(&app)?;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, &json).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

#[tauri::command]
fn start_runtime(app: tauri::AppHandle, state: State<Supervisor>) -> Result<String, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(child) = guard.as_mut() {
        // Already supervising — unless it has exited, report running.
        match child.try_wait() {
            Ok(Some(_)) => {} // exited; fall through and respawn
            Ok(None) => return Ok("already-running".into()),
            Err(e) => return Err(format!("status check failed: {e}")),
        }
    }

    // Pin a durable, per-user ledger so runs, audit trails, and backups persist
    // across restarts — this is what makes the app a real client of a real,
    // permanent Thymos ledger rather than an ephemeral session. The hash-chained
    // SQLite file lives in the OS app-data dir (the file the Backups tab copies).
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?;
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("create app data dir {}: {e}", data_dir.display()))?;
    let ledger_path = data_dir.join("ledger.db");
    let pid_file = data_dir.join("runtime.pid");

    // If a Thymos server is already listening on 3001 — one you started in the
    // terminal, or a prior session — adopt it instead of spawning a conflicting
    // one, BUT only if it speaks this app's version. After an upgrade, an
    // orphaned older runtime would otherwise keep answering with old behavior
    // (this bit real users twice). A version-mismatched server that WE spawned
    // (its pid is in runtime.pid) is terminated and replaced; one we didn't
    // spawn is adopted with a warning rather than killed.
    if std::net::TcpStream::connect_timeout(
        &([127, 0, 0, 1], 3001).into(),
        std::time::Duration::from_millis(300),
    )
    .is_ok()
    {
        let running = runtime_version();
        if running.as_deref() == Some(env!("CARGO_PKG_VERSION")) {
            return Ok("adopted-existing".into());
        }
        if kill_previous_runtime(&pid_file) {
            // Freed the port — fall through and spawn the bundled version.
            std::thread::sleep(std::time::Duration::from_millis(500));
        } else {
            return Ok(format!(
                "adopted-existing (version {} != app {}; not spawned by this app — restart it manually)",
                running.unwrap_or_else(|| "unknown".into()),
                env!("CARGO_PKG_VERSION"),
            ));
        }
    }

    // User-defined governed tools: the runtime loads JSON manifests from this
    // dir (`ToolManifest`), registering each as a first-class tool bound by its
    // declared effect class. The desktop's Add-tool form writes manifests here.
    let tools_dir = data_dir.join("tools");
    let _ = std::fs::create_dir_all(&tools_dir);

    let bin = server_binary();
    let mut cmd = Command::new(&bin);
    cmd.env("THYMOS_LEDGER_PATH", &ledger_path);
    cmd.env("THYMOS_TOOL_MANIFEST_DIRS", &tools_dir);
    // The agent's working folder: file/shell tools are confined to it. Without
    // one the tools fall back to the runtime's cwd (not useful), so a chosen
    // folder is what makes "read/edit my project" actually work.
    if let Some(ws) = load_workspace(&app) {
        cmd.env("THYMOS_WORKSPACE", ws);
    }
    if streaming_enabled(&app) {
        cmd.env("THYMOS_STREAM", "1");
    }
    apply_provider_env(&mut cmd, &load_provider_config(&app));
    let child = cmd
        .spawn()
        .map_err(|e| format!("failed to start {}: {e}", bin.display()))?;
    // Record the child's pid so a future session (e.g. after an app upgrade)
    // can terminate a stale runtime it finds still holding the port.
    let _ = std::fs::write(&pid_file, child.id().to_string());
    *guard = Some(child);
    Ok("started".into())
}

/// Version reported by whatever answers /health on the runtime port, if any.
fn runtime_version() -> Option<String> {
    let resp = ureq::get("http://127.0.0.1:3001/health")
        .timeout(std::time::Duration::from_secs(2))
        .call()
        .ok()?;
    let v: serde_json::Value = resp.into_json().ok()?;
    v.get("version").and_then(|s| s.as_str()).map(String::from)
}

/// Terminate the runtime recorded in `runtime.pid` — a process THIS app (a
/// previous session of it) spawned. Returns true if the pid existed and a
/// kill was issued. Never touches processes we didn't record.
fn kill_previous_runtime(pid_file: &std::path::Path) -> bool {
    let Ok(pid) = std::fs::read_to_string(pid_file).map(|s| s.trim().to_string()) else {
        return false;
    };
    if pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    #[cfg(unix)]
    let ok = Command::new("kill").arg(&pid).status().map(|s| s.success()).unwrap_or(false);
    #[cfg(windows)]
    let ok = Command::new("taskkill")
        .args(["/PID", &pid, "/F"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::fs::remove_file(pid_file);
    ok
}

/// Build the `/models` request for a provider — the host makes this call
/// locally (the webview's CSP can't reach provider URLs), with the stored key
/// applied as the right auth header. OpenAI-compatible by default; Anthropic
/// uses `x-api-key` + a version header.
fn models_request(provider: &str, base_url: &str, api_key: &str) -> ureq::Request {
    let anthropic = provider.eq_ignore_ascii_case("anthropic");
    let base = if !base_url.trim().is_empty() {
        base_url.trim().trim_end_matches('/').to_string()
    } else if anthropic {
        "https://api.anthropic.com/v1".to_string()
    } else {
        "https://api.openai.com/v1".to_string()
    };
    let mut req = ureq::get(&format!("{base}/models"))
        .timeout(std::time::Duration::from_secs(10));
    if anthropic {
        if !api_key.is_empty() {
            req = req.set("x-api-key", api_key);
        }
        req = req.set("anthropic-version", "2023-06-01");
    } else if !api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {api_key}"));
    }
    req
}

/// Test that a provider is reachable with the stored credentials (no secret
/// crosses to the webview — the host reads it and reports only the result).
#[tauri::command]
fn test_provider(app: tauri::AppHandle, provider: String, base_url: String) -> Result<String, String> {
    let cfg = load_provider_config(&app);
    match models_request(&provider, &base_url, &cfg.api_key).call() {
        Ok(resp) => {
            let v: serde_json::Value = resp.into_json().unwrap_or(serde_json::json!({}));
            let n = v.get("data").and_then(|d| d.as_array()).map(|a| a.len()).unwrap_or(0);
            Ok(format!("reachable — {n} models"))
        }
        Err(ureq::Error::Status(code, _)) => Err(format!("HTTP {code} — check the key / base URL")),
        Err(e) => Err(format!("not reachable: {e}")),
    }
}

/// Discover the models a provider offers (OpenAI-compatible `/models`; Ollama,
/// LM Studio, vLLM, etc. all serve this). Returns the model ids.
#[tauri::command]
fn discover_models(app: tauri::AppHandle, provider: String, base_url: String) -> Result<Vec<String>, String> {
    let cfg = load_provider_config(&app);
    let resp = models_request(&provider, &base_url, &cfg.api_key)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(c, _) => format!("HTTP {c} — check the key / base URL"),
            other => format!("not reachable: {other}"),
        })?;
    let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    let mut models: Vec<String> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    models.sort();
    Ok(models)
}

/// Absolute path to the durable ledger the supervised runtime writes to. The
/// Backups tab uses this to copy/verify the real on-disk chain.
#[tauri::command]
fn ledger_path(app: tauri::AppHandle) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?;
    Ok(dir.join("ledger.db").to_string_lossy().into_owned())
}

/// Directory holding the user's custom tool manifests (loaded by the runtime via
/// `THYMOS_TOOL_MANIFEST_DIRS`).
fn tools_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app data dir: {e}"))?
        .join("tools");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create tools dir: {e}"))?;
    Ok(dir)
}

/// Persist a custom tool manifest as `<name>.json`. The runtime validates and
/// registers it on next start; an invalid manifest is skipped by the loader, so
/// this only does light shape checks (a non-empty, file-safe `name`). The tool
/// then runs under the same governance as any native tool — its declared
/// `effect_class` is enforced by the compiler before it can execute.
#[tauri::command]
fn save_tool(app: tauri::AppHandle, manifest: serde_json::Value) -> Result<String, String> {
    let name = manifest
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("tool name must be non-empty and use only letters, digits, or _".into());
    }
    let path = tools_dir(&app)?.join(format!("{name}.json"));
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// List the names of the user's saved tool manifests.
#[tauri::command]
fn list_tool_manifests(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let dir = tools_dir(&app)?;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Delete a saved tool manifest by name. Takes effect on the next runtime start.
#[tauri::command]
fn delete_tool_manifest(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let safe = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !safe {
        return Err("invalid tool name".into());
    }
    let path = tools_dir(&app)?.join(format!("{name}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("delete {}: {e}", path.display()))?;
    }
    Ok(())
}

#[tauri::command]
fn stop_runtime(state: State<Supervisor>) -> Result<String, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
        return Ok("stopped".into());
    }
    Ok("not-running".into())
}

#[tauri::command]
fn runtime_running(state: State<Supervisor>) -> Result<bool, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    let running = match guard.as_mut() {
        Some(child) => matches!(child.try_wait(), Ok(None)),
        None => false,
    };
    Ok(running)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Supervisor::default())
        .invoke_handler(tauri::generate_handler![
            runtime_addr,
            start_runtime,
            stop_runtime,
            runtime_running,
            ledger_path,
            get_provider_config,
            set_provider_config,
            test_provider,
            discover_models,
            save_tool,
            list_tool_manifests,
            delete_tool_manifest,
            get_workspace,
            pick_workspace,
            clear_workspace,
            get_streaming,
            set_streaming
        ])
        .on_window_event(|window, event| {
            // Don't orphan the runtime when the app window closes.
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.try_state::<Supervisor>() {
                    if let Ok(mut guard) = state.0.lock() {
                        if let Some(mut child) = guard.take() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running OpenThymos Desktop");
}
