use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use thymos_server::{app, default_runtime, AppState, RuntimeMode};

fn test_state() -> Arc<AppState> {
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    Arc::new(AppState {
        runtime_mode: RuntimeMode::Reference,
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
    })
}

fn run_cli(url: &str, args: &[&str]) -> std::process::Output {
    Command::new(thymos_bin())
        .args(["--url", url])
        .args(args)
        .output()
        .expect("run thymos cli")
}

fn thymos_bin() -> PathBuf {
    let cargo_bin = PathBuf::from(env!("CARGO_BIN_EXE_thymos"));
    if cargo_bin.exists() {
        return cargo_bin;
    }

    let deps_dir = std::env::current_exe()
        .expect("current test exe")
        .parent()
        .expect("test exe parent")
        .to_path_buf();

    let mut candidates = std::fs::read_dir(&deps_dir)
        .expect("read target deps dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().is_none()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("thymos-") && !name.ends_with(".d"))
        })
        .filter_map(|path| {
            let modified = path.metadata().and_then(|meta| meta.modified()).ok()?;
            Some((modified, path))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|(left, _), (right, _)| right.cmp(left));

    candidates
        .into_iter()
        .map(|(_, path)| path)
        .find(|path| {
            Command::new(path)
                .arg("--help")
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        })
        .expect("locate compiled thymos binary")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn parse_run_id(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Run started: "))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .expect("cli output should include run id")
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_health_and_run_flow_against_live_server() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let url = format!("http://{addr}");

    let server = tokio::spawn(async move {
        axum::serve(listener, app(test_state()))
            .await
            .expect("serve thymos app");
    });

    let health = run_cli(&url, &["health"]);
    assert!(
        health.status.success(),
        "health failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&health),
        stderr(&health)
    );
    assert!(stdout(&health).contains("\"status\": \"ok\""));

    let run = run_cli(&url, &["run", "cli integration", "--provider", "mock"]);
    assert!(
        run.status.success(),
        "run failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&run),
        stderr(&run)
    );
    let run_stdout = stdout(&run);
    assert!(run_stdout.contains("Run started: "));
    assert!(run_stdout.contains("Poll status:"));
    assert!(run_stdout.contains("Stream live:"));

    let run_id = parse_run_id(&run_stdout);

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let status = run_cli(&url, &["status", &run_id]);
    assert!(
        status.status.success(),
        "status failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&status),
        stderr(&status)
    );
    let status_stdout = stdout(&status);
    assert!(status_stdout.contains("\"status\": \"completed\""));
    assert!(status_stdout.contains("\"final_answer\": \"mock cognition\""));

    let world = run_cli(&url, &["world", &run_id]);
    assert!(
        world.status.success(),
        "world failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&world),
        stderr(&world)
    );
    let world_stdout = stdout(&world);
    let world_json: serde_json::Value =
        serde_json::from_str(&world_stdout).expect("world output should be valid json");
    assert!(
        world_json.get("resources").is_some(),
        "unexpected world body: {world_stdout}"
    );

    let replay = run_cli(&url, &["replay", &run_id, "--verify", "--fold-world"]);
    assert!(
        replay.status.success(),
        "replay failed:\nstdout:\n{}\nstderr:\n{}",
        stdout(&replay),
        stderr(&replay)
    );
    let replay_stdout = stdout(&replay);
    assert!(replay_stdout.contains("OpenThymos replay"));
    assert!(replay_stdout.contains("result: replay verified"));

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_dispatches_piped_commands() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let url = format!("http://{addr}");

    let server = tokio::spawn(async move {
        axum::serve(listener, app(test_state()))
            .await
            .expect("serve thymos app");
    });

    // Drive the shell via stdin: health → run (mock) → status $last → exit.
    let script = "\
health
run \"shell integration\" --provider mock
# give the mock run a beat to finish
status $last
show
exit
";

    let mut child = Command::new(thymos_bin())
        .args(["--url", &url, "shell"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn thymos shell");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(script.as_bytes()).expect("write script");
        // Drop closes stdin so the shell loop sees EOF after the scripted lines.
    }

    // Give the mock run a moment to finish between the `run` and `status $last`
    // lines above — the shell processes lines back-to-back so the status call
    // can race the server otherwise.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let output = child.wait_with_output().expect("wait shell");
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "shell exited non-zero:\nstdout:\n{out}\nstderr:\n{err}"
    );
    assert!(out.contains("\"status\": \"ok\""), "health missing: {out}");
    assert!(out.contains("Run started: "), "run start missing: {out}");
    // Status call may race the mock run's completion — accept either state.
    assert!(
        out.contains("\"status\": \"completed\"") || out.contains("\"status\": \"running\""),
        "status output missing: {out}"
    );
    assert!(out.contains("provider      "), "show output missing: {out}");
    assert!(out.contains("last_run_id   "), "show output missing: {out}");

    server.abort();
    let _ = server.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_preset_and_workspace_wire_up() {
    let workspace = tempdir_with_git();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let url = format!("http://{addr}");

    let server = tokio::spawn(async move {
        axum::serve(listener, app(test_state()))
            .await
            .expect("serve thymos app");
    });

    let ws = workspace.to_string_lossy();
    let script = format!(
        "set preset code
set workspace {ws}
remember prefers rust, idiomatic code only
show
exit
"
    );

    let mut child = Command::new(thymos_bin())
        .args(["--url", &url, "shell"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn thymos shell");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(script.as_bytes()).expect("write script");
    }

    let output = child.wait_with_output().expect("wait shell");
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "shell exited non-zero:\nstdout:\n{out}\nstderr:\n{err}"
    );
    assert!(
        out.contains("preset        coding"),
        "preset missing: {out}"
    );
    assert!(
        out.contains("max_steps     64"),
        "preset did not bump max_steps: {out}"
    );
    assert!(out.contains("fs_patch"), "coding scopes missing: {out}");
    assert!(
        out.contains(workspace.to_str().unwrap()),
        "workspace path missing: {out}"
    );

    let mem = std::fs::read_to_string(workspace.join(".thymos").join("memory.md"))
        .expect("memory.md should be written by remember");
    assert!(mem.contains("prefers rust"), "memory contents: {mem}");

    server.abort();
    let _ = server.await;
}

fn tempdir_with_git() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("thymos-shell-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir tempdir");
    std::fs::create_dir_all(dir.join("src")).expect("mkdir src");
    std::fs::write(dir.join("README.md"), "test repo").expect("write readme");
    // `git status` runs but the dir doesn't need to be a real repo; any stderr
    // is swallowed by the preamble builder.
    dir
}
