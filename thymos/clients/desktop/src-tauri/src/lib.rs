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

    let bin = server_binary();
    let child = Command::new(&bin)
        .env("THYMOS_LEDGER_PATH", &ledger_path)
        .spawn()
        .map_err(|e| format!("failed to start {}: {e}", bin.display()))?;
    *guard = Some(child);
    Ok("started".into())
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
            ledger_path
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
