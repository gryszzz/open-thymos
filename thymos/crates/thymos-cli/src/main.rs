//! Thymos CLI — interact with the Thymos server from the terminal.
//!
//! Usage:
//!     thymos run "Set greeting to hello"
//!     thymos run "Say hello" --model openai --max-steps 8
//!     thymos status <run-id>
//!     thymos stream <run-id>
//!     thymos world <run-id>
//!     thymos replay <run-id>
//!     thymos usage
//!     thymos health

use clap::{Parser, Subcommand};
use serde_json::Value;
use std::process::Command as ProcessCommand;

mod shell;

#[derive(Parser)]
#[command(name = "thymos", about = "Thymos governed-cognition CLI")]
struct Cli {
    /// Server URL (default: http://localhost:3001).
    #[arg(long, env = "THYMOS_URL", default_value = "http://localhost:3001")]
    url: String,

    /// API key for authenticated requests.
    #[arg(long, env = "THYMOS_API_KEY")]
    api_key: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create and start a new agent run.
    Run {
        /// Task description.
        task: String,
        /// Maximum steps (default: 16).
        #[arg(long, default_value = "16")]
        max_steps: u32,
        /// Cognition provider. Default `auto` uses the server's configured
        /// provider (which resolves THYMOS_DEFAULT_PROVIDER, then the first API
        /// key, then mock). Pass anthropic / openai / local / lmstudio /
        /// huggingface / mock to override it for this run.
        #[arg(long, default_value = "auto")]
        provider: String,
        /// Model override.
        #[arg(long)]
        model: Option<String>,
        /// Base URL override for any OpenAI-compatible endpoint (used with
        /// `--provider openai` / a custom host). Presets set this for you.
        #[arg(long = "base-url")]
        base_url: Option<String>,
        /// Tool scopes (comma-separated).
        #[arg(long)]
        scopes: Option<String>,
        /// After starting the run, stream cognition events until it completes.
        #[arg(long, short = 'f')]
        follow: bool,
    },
    /// Get run status and summary.
    Status {
        /// Run ID.
        run_id: String,
    },
    /// Stream cognition events (tokens) in real-time.
    Stream {
        /// Run ID.
        run_id: String,
    },
    /// Get world state for a run.
    World {
        /// Run ID.
        run_id: String,
    },
    /// Verify and fold a run's execution ledger.
    Replay {
        /// Run ID.
        run_id: String,
        /// Accepted for protocol-shaped scripts; replay always verifies integrity.
        #[arg(long)]
        verify: bool,
        /// Accepted for protocol-shaped scripts; replay always folds committed state.
        #[arg(long)]
        fold_world: bool,
        /// Accepted for protocol-shaped scripts; replay reports ledger-visible policy outcomes.
        #[arg(long)]
        policy_trace: bool,
        /// Require every replayed commit to match a compiler version.
        #[arg(long)]
        require_compiler: Option<String>,
        /// Print the raw JSON replay report.
        #[arg(long)]
        json: bool,
    },
    /// Print a trajectory's full governance story: the commit chain,
    /// rejections, suspensions, delegations, the policy decision behind each
    /// committed action, and a replay-verification verdict. This is the audit
    /// trail rendered for a human — the demo of "the boundary worked".
    Audit {
        /// Run ID.
        run_id: String,
        /// Print the raw JSON (ledger entries + replay report) instead.
        #[arg(long)]
        json: bool,
    },
    /// List supported cognition providers / models (presets + how to start).
    Providers,
    /// Show API gateway usage stats.
    Usage,
    /// Health check.
    Health,
    /// Branded local + runtime readiness dashboard.
    Doctor,
    /// Show terminal configuration, env vars, and next commands.
    Config,
    /// Approve or deny a pending proposal.
    Approve {
        /// Run ID.
        run_id: String,
        /// Approval channel name.
        channel: String,
        /// Deny instead of approve.
        #[arg(long)]
        deny: bool,
    },
    /// Run history operations (list / show / diff).
    Runs {
        #[command(subcommand)]
        action: RunsAction,
    },
    /// Cancel a running agent.
    Cancel {
        /// Run ID.
        run_id: String,
    },
    /// Launch an interactive Thymos shell — programmable terminal with
    /// persistent session defaults and an `auto` autonomous loop.
    Shell,
}

#[derive(Subcommand)]
enum RunsAction {
    /// List recent runs.
    Ls {
        /// Filter by status (running, completed, failed).
        #[arg(long)]
        status: Option<String>,
        /// Page size (default: 50, max 200).
        #[arg(long, default_value = "50")]
        limit: u32,
        /// Offset for pagination.
        #[arg(long, default_value = "0")]
        offset: u32,
    },
    /// Show full record + ledger entries for one run.
    Show {
        /// Run ID.
        run_id: String,
    },
    /// Diff the ledger entries of two runs (counts + final-answer compare).
    Diff {
        /// Source run ID.
        a: String,
        /// Target run ID.
        b: String,
    },
}

#[tokio::main]
async fn main() {
    // Load .env from CWD or any parent dir so THYMOS_URL / THYMOS_API_KEY
    // and provider tokens are picked up without needing to `source` manually.
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();
    let client = reqwest::Client::new();

    // Bare `thymos` (no subcommand) → branded home screen.
    let Some(command) = cli.command else {
        cmd_home(&cli.url);
        return;
    };

    let result = match command {
        Commands::Run {
            task,
            max_steps,
            provider,
            model,
            base_url,
            scopes,
            follow,
        } => {
            cmd_run(
                &client,
                &cli.url,
                cli.api_key.as_deref(),
                CmdRunOptions {
                    start: StartRunOptions {
                        task: &task,
                        max_steps,
                        provider: &provider,
                        model: model.as_deref(),
                        base_url: base_url.as_deref(),
                        scopes: scopes.as_deref(),
                    },
                    follow,
                },
            )
            .await
        }
        Commands::Status { run_id } => {
            cmd_status(&client, &cli.url, cli.api_key.as_deref(), &run_id).await
        }
        Commands::Stream { run_id } => cmd_stream(&cli.url, &run_id).await,
        Commands::World { run_id } => {
            cmd_world(&client, &cli.url, cli.api_key.as_deref(), &run_id).await
        }
        Commands::Replay {
            run_id,
            require_compiler,
            json,
            ..
        } => {
            cmd_replay(
                &client,
                &cli.url,
                cli.api_key.as_deref(),
                &run_id,
                require_compiler.as_deref(),
                json,
            )
            .await
        }
        Commands::Audit { run_id, json } => {
            cmd_audit(&client, &cli.url, cli.api_key.as_deref(), &run_id, json).await
        }
        Commands::Providers => cmd_providers(),
        Commands::Usage => cmd_usage(&client, &cli.url, cli.api_key.as_deref()).await,
        Commands::Health => cmd_health(&client, &cli.url).await,
        Commands::Doctor => cmd_doctor(&client, &cli.url, cli.api_key.as_deref()).await,
        Commands::Config => cmd_config(&cli.url, cli.api_key.as_deref()),
        Commands::Approve {
            run_id,
            channel,
            deny,
        } => {
            cmd_approve(
                &client,
                &cli.url,
                cli.api_key.as_deref(),
                &run_id,
                &channel,
                !deny,
            )
            .await
        }
        Commands::Cancel { run_id } => {
            cmd_cancel(&client, &cli.url, cli.api_key.as_deref(), &run_id).await
        }
        Commands::Shell => shell::cmd_shell(&client, &cli.url, cli.api_key.as_deref()).await,
        Commands::Runs { action } => match action {
            RunsAction::Ls {
                status,
                limit,
                offset,
            } => {
                cmd_runs_ls(
                    &client,
                    &cli.url,
                    cli.api_key.as_deref(),
                    status.as_deref(),
                    limit,
                    offset,
                )
                .await
            }
            RunsAction::Show { run_id } => {
                cmd_runs_show(&client, &cli.url, cli.api_key.as_deref(), &run_id).await
            }
            RunsAction::Diff { a, b } => {
                cmd_runs_diff(&client, &cli.url, cli.api_key.as_deref(), &a, &b).await
            }
        },
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        if is_connection_error(&e) {
            eprintln!();
            eprintln!("Could not reach the Thymos server at {}.", cli.url);
            eprintln!("Is it running?  Start it with:  cargo run -p thymos-server");
            eprintln!("Point at another server with  --url <addr>  or  THYMOS_URL=<addr>.");
        }
        std::process::exit(1);
    }
}

/// Heuristic over the stringified transport error: did we fail because nothing
/// was listening / reachable, rather than because the server returned an error?
fn is_connection_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    // reqwest's top-level Display is just "error sending request for url (…)" —
    // the connect/DNS cause lives in the source chain, which is dropped when the
    // error is stringified. For a CLI a failure to even send the request means
    // the server is unreachable (an HTTP-status error takes a different path),
    // so this phrase is a reliable signal on its own.
    m.contains("error sending request")
        || m.contains("connection refused")
        || m.contains("error trying to connect")
        || m.contains("tcp connect error")
        || m.contains("connection reset")
        || m.contains("dns error")
        || m.contains("failed to lookup")
}

fn color_enabled() -> bool {
    use std::io::IsTerminal;
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|term| term != "dumb")
            .unwrap_or(true)
        // Don't emit SGR codes when output is piped/redirected (scripts, tests,
        // `| grep`); only colorize a real terminal.
        && std::io::stdout().is_terminal()
}

fn paint(code: &str, text: impl AsRef<str>) -> String {
    if color_enabled() {
        format!("\x1b[{code}m{}\x1b[0m", text.as_ref())
    } else {
        text.as_ref().to_string()
    }
}

pub(crate) fn brand_banner() {
    // Wordmark art, painted top→bottom in the brand violet gradient
    // (#c77dff → #7c3aed) so it reads as a glowing logo lockup.
    const ART: [&str; 8] = [
        r"  _______ _                              ",
        r" |__   __| |                             ",
        r"    | |  | |__  _   _ _ __ ___   ___  ___ ",
        r"    | |  | '_ \| | | | '_ ` _ \ / _ \/ __|",
        r"    | |  | | | | |_| | | | | | | (_) \__ \",
        r"    |_|  |_| |_|\__, |_| |_| |_|\___/|___/",
        r"                 __/ |                    ",
        r"                |___/                     ",
    ];
    let (top, bot) = ((199.0_f32, 125.0, 255.0), (124.0_f32, 58.0, 237.0));
    let span = (ART.len() - 1).max(1) as f32;
    println!();
    for (i, line) in ART.iter().enumerate() {
        let t = i as f32 / span;
        let r = (top.0 + (bot.0 - top.0) * t) as u8;
        let g = (top.1 + (bot.1 - top.1) * t) as u8;
        let b = (top.2 + (bot.2 - top.2) * t) as u8;
        println!("{}", paint(&format!("38;2;{r};{g};{b};1"), line));
    }
    println!(
        "    {}   {}",
        paint("38;2;199;125;255;1", "◆ OPEN-THYMOS"),
        paint("38;2;139;233;255", "governed execution runtime"),
    );
    println!(
        "       {}",
        paint(
            "38;2;109;119;137",
            "cognition proposes · the runtime governs · the ledger records",
        ),
    );
    println!();
}

// ----- brand palette (truecolor SGR codes) -----
const C_VIOLET: &str = "38;2;199;125;255";
const C_VIOLET_B: &str = "38;2;199;125;255;1";
const C_STAR: &str = "38;2;139;233;255";
const C_OK: &str = "38;2;52;211;153;1";
const C_WARN: &str = "38;2;251;191;36;1";
const C_RED: &str = "38;2;255;111;111;1";
const C_DIM: &str = "2";

/// A glyph + violet/cyan badge for an execution phase, e.g. Intent → Proposal →
/// Commit. Unknown phases fall back to a neutral dot.
fn phase_badge(phase: &str) -> String {
    let (glyph, code) = match phase.to_ascii_lowercase().as_str() {
        "intent" => ('◆', C_STAR),
        "proposal" | "propose" | "compile" => ('▸', C_VIOLET_B),
        "commit" | "committed" => ('✓', C_OK),
        "rejection" | "rejected" | "deny" => ('✕', C_RED),
        "approval" | "pending_approval" | "suspended" => ('⏸', C_WARN),
        "cognition" | "think" | "model" => ('✷', C_VIOLET),
        "tool" | "execute" => ('⚙', C_VIOLET),
        _ => ('·', C_DIM),
    };
    paint(code, format!("{glyph} {phase}"))
}

/// Color an execution-log level word.
fn level_code(level: &str) -> &'static str {
    match level.to_ascii_lowercase().as_str() {
        "ok" | "success" | "commit" => C_OK,
        "warn" | "warning" => C_WARN,
        "error" | "fail" | "failed" => C_RED,
        _ => C_DIM,
    }
}

/// Status word colored by lifecycle state.
fn status_code(status: &str) -> &'static str {
    match status.to_ascii_lowercase().as_str() {
        "completed" | "complete" | "ok" => C_OK,
        "failed" | "cancelled" | "error" => C_RED,
        _ => C_VIOLET_B,
    }
}

fn status_line(label: &str, ok: bool, detail: impl AsRef<str>) {
    let marker = if ok {
        paint(C_OK, "OK ")
    } else {
        paint(C_WARN, "CHK")
    };
    println!("{marker}  {label:<24} {}", detail.as_ref());
}

/// One aligned `command  description` row for the home screen.
fn home_row(cmd: &str, desc: &str) {
    println!(
        "    {}{}",
        paint(C_VIOLET, format!("{cmd:<30}")),
        paint(C_DIM, desc)
    );
}

fn home_section(title: &str) {
    println!();
    println!("  {}", paint(C_VIOLET_B, title));
}

/// `thymos` with no subcommand: the branded home — banner, what it is, and the
/// command map. Pure print; no network.
fn cmd_home(url: &str) {
    brand_banner();
    println!(
        "  {}",
        paint(C_DIM, "A governed execution runtime — cognition proposes, the runtime")
    );
    println!(
        "  {}",
        paint(C_DIM, "governs every effect, the ledger records and replays.")
    );

    home_section("RUN");
    home_row("run \"<task>\" --follow", "start a governed run; stream Intent → Proposal → Commit");
    home_row("status <run-id>", "run status + summary");
    home_row("stream <run-id>", "live execution feed");

    home_section("GOVERNANCE");
    home_row("audit <run-id>", "the full governance trail + replay verdict");
    home_row("replay <run-id>", "verify the ledger folds to its world");
    home_row("world <run-id>", "current projected world state");
    home_row("approve <run-id> <channel>", "clear a human-in-the-loop gate");

    home_section("SETUP");
    home_row("doctor", "branded readiness dashboard");
    home_row("providers", "list cognition providers / presets");
    home_row("health", "server liveness · live-vs-mock · ledger backend");
    home_row("shell", "interactive Thymos terminal");

    println!();
    println!("  {} {}", paint(C_DIM, "server"), paint(C_STAR, url));
    println!(
        "  {} {}",
        paint(C_DIM, "repo  "),
        paint(C_VIOLET, "https://github.com/gryszzz/open-thymos")
    );
    println!();
    println!(
        "  {}",
        paint(C_DIM, "`thymos <command> --help` for details · `thymos shell` for a session")
    );
}

fn mask_secret(value: Option<&str>) -> String {
    match value {
        Some(v) if v.len() > 8 => format!("{}...{}", &v[..4], &v[v.len() - 4..]),
        Some(_) => "(set)".into(),
        None => "(not set)".into(),
    }
}

fn command_version(command: &str) -> Option<String> {
    ProcessCommand::new(command)
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()?
                        .to_string(),
                )
            } else {
                None
            }
        })
}

pub(crate) fn auth_headers(api_key: Option<&str>) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(key) = api_key {
        headers.push(("Authorization".into(), format!("Bearer {key}")));
    }
    headers
}

pub(crate) async fn json_body_or_error(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status,
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
        ));
    }
    Ok(body)
}

/// POST /runs and return the new run id. Used by both the one-shot `run`
/// command and the interactive shell's `run` / `auto` commands.
#[derive(Clone, Copy)]
pub(crate) struct StartRunOptions<'a> {
    pub(crate) task: &'a str,
    pub(crate) max_steps: u32,
    pub(crate) provider: &'a str,
    pub(crate) model: Option<&'a str>,
    pub(crate) base_url: Option<&'a str>,
    pub(crate) scopes: Option<&'a str>,
}

pub(crate) async fn start_run(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    options: StartRunOptions<'_>,
) -> Result<String, String> {
    let mut body = serde_json::json!({
        "task": options.task,
        "max_steps": options.max_steps,
    });
    // `auto` means "don't override the server's configured provider" — so we omit
    // the cognition block entirely and let the server's own resolution order win
    // (THYMOS_DEFAULT_PROVIDER -> first API key -> mock). An explicit provider is
    // sent as a per-run override; `--model` pairs with an explicit provider.
    if !options.provider.eq_ignore_ascii_case("auto") {
        body["cognition"] = serde_json::json!({ "provider": options.provider });
        if let Some(m) = options.model {
            body["cognition"]["model"] = serde_json::json!(m);
        }
        if let Some(u) = options.base_url {
            body["cognition"]["base_url"] = serde_json::json!(u);
        }
    }
    if let Some(s) = options.scopes {
        let scope_list: Vec<&str> = s.split(',').collect();
        body["tool_scopes"] = serde_json::json!(scope_list);
    }

    let mut req = client.post(format!("{url}/runs")).json(&body);
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    let parsed = json_body_or_error(resp).await?;
    parsed["run_id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("server response missing run_id: {parsed}"))
}

#[derive(Clone, Copy)]
pub(crate) struct CmdRunOptions<'a> {
    pub(crate) start: StartRunOptions<'a>,
    pub(crate) follow: bool,
}

pub(crate) async fn cmd_run(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    options: CmdRunOptions<'_>,
) -> Result<(), String> {
    let run_id = start_run(client, url, api_key, options.start).await?;

    // `Run started: <id>` stays a stable, line-start-parseable contract.
    println!("Run started: {run_id}");
    println!("  {} {}", paint(C_DIM, "task:"), options.start.task);
    if options.start.provider.eq_ignore_ascii_case("auto") {
        println!("  {} auto (server default)", paint(C_DIM, "provider:"));
    } else {
        println!(
            "  {} {}",
            paint(C_DIM, "provider:"),
            paint(C_VIOLET, options.start.provider)
        );
    }
    println!();
    if options.follow {
        println!("{}", paint(C_VIOLET, "── live ── Intent → Proposal → Commit"));
        cmd_stream(url, &run_id).await?;
        // Print final status once the stream closes.
        return cmd_status(client, url, api_key, &run_id).await;
    }
    println!("Poll status:  thymos status {run_id}");
    println!("Stream live:  thymos stream {run_id}");
    Ok(())
}

pub(crate) async fn cmd_cancel(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
) -> Result<(), String> {
    let mut req = client.post(format!("{url}/runs/{run_id}/cancel"));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    println!("{}", serde_json::to_string_pretty(&body).unwrap());
    Ok(())
}

pub(crate) async fn cmd_status(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
) -> Result<(), String> {
    let mut req = client.get(format!("{url}/runs/{run_id}"));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    println!("{}", serde_json::to_string_pretty(&body).unwrap());
    Ok(())
}

pub(crate) async fn cmd_stream(url: &str, run_id: &str) -> Result<(), String> {
    // Unified execution-session stream.
    let resp = reqwest::get(format!("{url}/runs/{run_id}/execution/stream"))
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        let body: Value = resp.json().await.map_err(|e| e.to_string())?;
        println!("Error: {}", serde_json::to_string_pretty(&body).unwrap());
        return Ok(());
    }

    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut last_log_idx = 0u64;
    let mut last_status = String::new();
    let mut last_operator_state = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Parse SSE lines.
        while let Some(pos) = buffer.find("\n\n") {
            let event_block = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            for line in event_block.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if let Ok(snapshot) = serde_json::from_str::<Value>(data) {
                        let status = snapshot["status"].as_str().unwrap_or("?");
                        let phase = snapshot["phase"].as_str().unwrap_or("?");
                        let operator_state = snapshot["operator_state"].as_str().unwrap_or("");

                        if status != last_status || operator_state != last_operator_state {
                            println!(
                                "\n{}  {}  {}",
                                paint(status_code(status), format!("● {}", status.to_uppercase())),
                                phase_badge(phase),
                                paint(C_DIM, operator_state)
                            );
                            last_status = status.to_string();
                            last_operator_state = operator_state.to_string();
                        }

                        if let Some(entries) = snapshot["log"].as_array() {
                            for entry in entries {
                                let idx = entry["idx"].as_u64().unwrap_or(0);
                                if idx <= last_log_idx {
                                    continue;
                                }
                                print_execution_entry(entry);
                                last_log_idx = idx;
                            }
                        }

                        if matches!(status, "completed" | "failed" | "cancelled") {
                            if let Some(answer) = snapshot["final_answer"].as_str() {
                                println!("\n{}", paint(C_VIOLET_B, "── final answer ──"));
                                println!("{answer}");
                            }
                        }
                    }
                }
            }
        }
    }
    println!();
    Ok(())
}

fn print_execution_entry(entry: &Value) {
    let phase = entry["phase"].as_str().unwrap_or("?");
    let level = entry["level"].as_str().unwrap_or("info");
    let title = entry["title"].as_str().unwrap_or("");
    let detail = entry["detail"].as_str().unwrap_or("");
    let step = entry["step_index"]
        .as_u64()
        .map(|n| format!("step {} ", n + 1))
        .unwrap_or_default();
    let tool = entry["tool"].as_str().unwrap_or("");

    let mut head = format!("  {}  ", phase_badge(phase));
    if !step.is_empty() {
        head.push_str(&paint(C_DIM, &step));
    }
    if !tool.is_empty() {
        head.push_str(&paint(C_VIOLET, format!("{tool} ")));
    }
    if !level.eq_ignore_ascii_case("info") {
        head.push_str(&paint(level_code(level), format!("{} ", level.to_uppercase())));
    }
    head.push_str(title);
    println!("{head}");
    if !detail.is_empty() {
        println!("     {}", paint(C_DIM, detail));
    }
}

pub(crate) async fn cmd_world(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
) -> Result<(), String> {
    let mut req = client.get(format!("{url}/runs/{run_id}/world"));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    println!("{}", serde_json::to_string_pretty(&body).unwrap());
    Ok(())
}

pub(crate) async fn cmd_replay(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
    require_compiler: Option<&str>,
    raw_json: bool,
) -> Result<(), String> {
    let mut req = client.get(format!("{url}/runs/{run_id}/replay"));
    if let Some(version) = require_compiler {
        req = req.query(&[("require_compiler", version)]);
    }
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    if raw_json {
        println!("{}", serde_json::to_string_pretty(&body).unwrap());
        return Ok(());
    }

    let trajectory = body["trajectory_id"].as_str().unwrap_or("?");
    let entries_seen = body["entries_seen"].as_u64().unwrap_or(0);
    let commits = body["commits_replayed"].as_u64().unwrap_or(0);
    let head_seq = body["head_seq"].as_u64().unwrap_or(0);
    let head_commit = body["head_commit"].as_str().unwrap_or("(none)");
    let final_world_hash = body["final_world_hash"].as_str().unwrap_or("?");
    let resources = body["resources"].as_u64().unwrap_or(0);
    let rejections = body["rejected_proposals"].as_u64().unwrap_or(0);
    let approvals = body["pending_approvals"].as_u64().unwrap_or(0);

    println!("{}", paint(C_VIOLET_B, "◆ OpenThymos replay"));
    println!("  {}  {run_id}", paint(C_DIM, "run       "));
    println!("  {}  {trajectory}", paint(C_DIM, "trajectory"));
    println!();
    println!("{}", paint(C_OK, "✓ [integrity] verified"));
    println!("  entries seen:        {entries_seen}");
    println!("  commits replayed:   {commits}");
    println!("  rejected proposals: {rejections}");
    println!("  pending approvals:  {approvals}");
    println!("  head sequence:      {head_seq}");
    println!("  head commit:        {head_commit}");
    println!();
    println!("{}", paint(C_VIOLET_B, "[fold]"));
    println!("  resources:          {resources}");
    println!("  final world hash:   {}", paint(C_STAR, final_world_hash));

    if let Some(versions) = body["compiler_versions_seen"].as_array() {
        let versions = versions
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        if !versions.is_empty() {
            println!("  compiler versions:  {}", versions.join(", "));
        }
    }

    if let Some(tool_calls) = body["tool_calls"].as_array() {
        if !tool_calls.is_empty() {
            println!();
            println!("{}", paint(C_VIOLET_B, "[tool calls]"));
            for call in tool_calls {
                let seq = call["seq"].as_u64().unwrap_or(0);
                let tool = call["tool"].as_str().unwrap_or("?");
                let latency = call["latency_ms"].as_u64().unwrap_or(0);
                let commit = call["commit_id"].as_str().unwrap_or("");
                let commit_short: String = commit.chars().take(12).collect();
                println!(
                    "  seq={seq:<4} tool={tool:<16} latency={latency}ms commit={commit_short}"
                );
            }
        }
    }

    println!();
    println!("{}", paint(C_OK, "result: replay verified ✓"));
    Ok(())
}

/// Render one short, hex-prefix label for a content-addressed id.
fn short_id(v: &Value) -> String {
    v.as_str().map(|s| s.chars().take(12).collect()).unwrap_or_default()
}

/// Human-readable rendering of a `PolicyDecision` / `RejectionReason`
/// (both serialize as `{ "kind": ..., "detail": ... }`).
fn tagged_reason(v: &Value) -> String {
    let kind = v["kind"].as_str().unwrap_or("?");
    match &v["detail"] {
        Value::Null => kind.to_string(),
        Value::String(s) => format!("{kind}: {s}"),
        Value::Object(o) => {
            let parts: Vec<String> = o
                .iter()
                .map(|(k, val)| format!("{k}={}", val.as_str().unwrap_or(&val.to_string())))
                .collect();
            format!("{kind} ({})", parts.join(", "))
        }
        other => format!("{kind}: {other}"),
    }
}

/// `thymos audit <run-id>` — compose the ledger entries and the replay report
/// into one human-readable governance trail. Pure read: it only GETs
/// `/audit/entries` and `/runs/:id/replay`.
pub(crate) async fn cmd_audit(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
    raw_json: bool,
) -> Result<(), String> {
    // 1. Ledger entries for this run's trajectory.
    let mut req = client
        .get(format!("{url}/audit/entries"))
        .query(&[("run_id", run_id)]);
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }
    let entries_body = json_body_or_error(req.send().await.map_err(|e| e.to_string())?).await?;

    // 2. Replay verdict (best-effort: a trajectory with no commits still audits).
    let mut rreq = client.get(format!("{url}/runs/{run_id}/replay"));
    for (k, v) in auth_headers(api_key) {
        rreq = rreq.header(&k, &v);
    }
    let replay = match rreq.send().await {
        Ok(resp) => json_body_or_error(resp).await.ok(),
        Err(_) => None,
    };

    if raw_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "entries": entries_body["entries"],
                "replay": replay,
            }))
            .unwrap()
        );
        return Ok(());
    }

    let empty = vec![];
    let entries = entries_body["entries"].as_array().unwrap_or(&empty);
    print!("{}", render_audit(run_id, entries, replay.as_ref(), color_enabled()));
    Ok(())
}

/// Pure renderer for `thymos audit`: turn the ledger entries (+ optional replay
/// report) into the human-readable governance trail. Kept side-effect-free so it
/// is unit-testable against synthetic entries covering every entry kind.
pub(crate) fn render_audit(
    run_id: &str,
    entries: &[Value],
    replay: Option<&Value>,
    color: bool,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    // Paint only when color is enabled; uncolored output stays byte-identical
    // (the unit tests assert plain strings).
    let c = |code: &str, s: &str| if color { paint(code, s) } else { s.to_string() };

    let trajectory = entries
        .first()
        .and_then(|e| e["trajectory_id"].as_str())
        .or_else(|| replay.and_then(|r| r["trajectory_id"].as_str()))
        .unwrap_or("?");

    let _ = writeln!(out, "{}", c(C_VIOLET_B, "OpenThymos audit"));
    let _ = writeln!(out, "run:              {run_id}");
    let _ = writeln!(out, "trajectory:       {trajectory}");
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", c(C_VIOLET_B, &format!("ledger ({} entries)", entries.len())));

    let mut commits = 0u64;
    let mut rejections = 0u64;
    for e in entries {
        let seq = e["seq"].as_u64().unwrap_or(0);
        let p = &e["payload"];
        let prefix = format!("  #{seq:<3}");
        match p["type"].as_str().unwrap_or("?") {
            "root" => {
                let note = p["note"].as_str().unwrap_or("");
                let _ = writeln!(out, "{prefix} {}        trajectory bound  {note:?}", c(C_VIOLET, "ROOT"));
            }
            "commit" => {
                commits += 1;
                let body = &p["body"];
                let tool = body["observations"]
                    .as_array()
                    .and_then(|o| o.first())
                    .and_then(|o| o["tool"].as_str())
                    .unwrap_or("?");
                let decision = tagged_reason(&body["policy_trace"]["decision"]);
                let rules = body["policy_trace"]["rules_evaluated"]
                    .as_array()
                    .map(|r| {
                        r.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                let writ = short_id(&body["writ_id"]);
                let signed = if body["signature"].is_string() {
                    " signed✓"
                } else {
                    ""
                };
                let _ = write!(out, "{prefix} {}      {tool:<16} policy={decision}", c(C_OK, "COMMIT"));
                if !rules.is_empty() {
                    let _ = write!(out, " [{rules}]");
                }
                let _ = write!(out, " writ={writ}{signed}");
                if body["compensates"].is_string() {
                    let _ = write!(out, " compensates={}", short_id(&body["compensates"]));
                }
                if body["routing_evidence"].is_object() {
                    let _ = write!(
                        out,
                        " route={}",
                        body["routing_evidence"]["selected"].as_str().unwrap_or("?")
                    );
                }
                let _ = writeln!(out);
            }
            "rejection" => {
                rejections += 1;
                let _ = writeln!(out, "{prefix} {}    {}", c(C_RED, "REJECTED"), tagged_reason(&p["reason"]));
            }
            "pending_approval" => {
                let channel = p["channel"].as_str().unwrap_or("?");
                let reason = p["reason"].as_str().unwrap_or("");
                let _ = writeln!(out, "{prefix} {}   channel={channel} reason={reason:?}", c(C_WARN, "SUSPENDED"));
            }
            "delegation" => {
                let child = short_id(&p["child_trajectory_id"]);
                let task = p["task"].as_str().unwrap_or("");
                let _ = writeln!(out, "{prefix} {}  child={child} task={task:?}", c(C_VIOLET, "DELEGATION"));
            }
            "branch" => {
                let src = short_id(&p["source_trajectory_id"]);
                let commit = short_id(&p["source_commit_id"]);
                let _ = writeln!(out, "{prefix} {}      from {src}@{commit}", c(C_VIOLET, "BRANCH"));
            }
            other => {
                let _ = writeln!(out, "{prefix} {other}");
            }
        }
    }

    let _ = writeln!(out);
    if let Some(r) = replay {
        let verified = r["verified"].as_bool();
        let final_hash = r["final_world_hash"].as_str().unwrap_or("?");
        let head_seq = r["head_seq"].as_u64().unwrap_or(0);
        let ok = verified != Some(false);
        let _ = writeln!(out, "{}", c(C_VIOLET_B, "replay"));
        let _ = writeln!(
            out,
            "  {}",
            c(
                if ok { C_OK } else { C_RED },
                &format!("[integrity] {}", if ok { "verified" } else { "FAILED" })
            )
        );
        let _ = writeln!(
            out,
            "  commits replayed:   {}",
            r["commits_replayed"].as_u64().unwrap_or(commits)
        );
        let _ = writeln!(
            out,
            "  rejected proposals: {}",
            r["rejected_proposals"].as_u64().unwrap_or(rejections)
        );
        let _ = writeln!(out, "  head sequence:      {head_seq}");
        let _ = writeln!(out, "  final world hash:   {final_hash}");
        let _ = writeln!(out);
        let verdict = if verified == Some(false) {
            "replay verification FAILED"
        } else {
            "replay verified"
        };
        let _ = writeln!(
            out,
            "{}",
            c(
                if ok { C_OK } else { C_RED },
                &format!("result: {commits} commits, {rejections} rejections — {verdict}")
            )
        );
    } else {
        let _ = writeln!(
            out,
            "replay: unavailable (run has no trajectory or replay endpoint errored)"
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "result: {commits} commits, {rejections} rejections");
    }
    out
}

#[cfg(test)]
mod audit_tests {
    use super::render_audit;
    use serde_json::json;

    #[test]
    fn renders_every_entry_kind_and_verdict() {
        let entries = vec![
            json!({"seq":0,"trajectory_id":"deadbeefcafe0000","payload":{"type":"root","note":"demo"}}),
            json!({"seq":1,"payload":{"type":"commit","body":{
                "writ_id":"ab12ab12ab12ab12",
                "observations":[{"tool":"kv_set","latency_ms":42}],
                "policy_trace":{"rules_evaluated":["WritAuthority"],"decision":{"kind":"permit"}},
                "signature":"ff",
                "routing_evidence":{"selected":"anthropic:claude"}
            }}}),
            json!({"seq":2,"payload":{"type":"rejection","reason":{"kind":"policy_denied","detail":"writ does not authorize tool 'delete_all'"}}}),
            json!({"seq":3,"payload":{"type":"pending_approval","channel":"ops","reason":"irreversible"}}),
            json!({"seq":4,"payload":{"type":"delegation","child_trajectory_id":"cccc1111dddd2222","task":"sub-task"}}),
            json!({"seq":5,"payload":{"type":"commit","body":{
                "writ_id":"ab12ab12ab12ab12",
                "observations":[{"tool":"kv_del","latency_ms":3}],
                "policy_trace":{"rules_evaluated":[],"decision":{"kind":"permit"}},
                "compensates":"99aa99aa99aa99aa","signature":null
            }}}),
        ];
        let replay = json!({"verified":true,"commits_replayed":2,"rejected_proposals":1,"head_seq":5,"final_world_hash":"f4cfe88219"});
        let out = render_audit("run-x", &entries, Some(&replay), false);

        assert!(out.contains("trajectory:       deadbeefcafe0000"));
        assert!(out.contains("ROOT        trajectory bound  \"demo\""));
        // commit: tool, human-readable policy decision, rules, short writ, signed mark, route
        assert!(out.contains("COMMIT      kv_set"));
        assert!(out.contains("policy=permit [WritAuthority]"));
        assert!(out.contains("writ=ab12ab12ab12"));
        assert!(out.contains("signed✓"));
        assert!(out.contains("route=anthropic:claude"));
        // rejection renders the tagged reason
        assert!(out.contains("REJECTED    policy_denied: writ does not authorize tool 'delete_all'"));
        assert!(out.contains("SUSPENDED   channel=ops"));
        assert!(out.contains("DELEGATION  child=cccc1111dddd"));
        // compensation commit
        assert!(out.contains("compensates=99aa99aa99aa"));
        // verdict line
        assert!(out.contains("[integrity] verified"));
        assert!(out.contains("result: 2 commits, 1 rejections — replay verified"));
    }

    #[test]
    fn failed_integrity_and_missing_replay() {
        let entries = vec![json!({"seq":0,"trajectory_id":"aa","payload":{"type":"root","note":"x"}})];
        let failed = json!({"verified":false,"final_world_hash":"z","head_seq":0});
        assert!(render_audit("r", &entries, Some(&failed), false).contains("replay verification FAILED"));
        assert!(render_audit("r", &entries, None, false).contains("replay: unavailable"));
    }
}

pub(crate) async fn cmd_usage(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
) -> Result<(), String> {
    let mut req = client.get(format!("{url}/usage"));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    println!("{}", serde_json::to_string_pretty(&body).unwrap());
    Ok(())
}

pub(crate) async fn cmd_health(client: &reqwest::Client, url: &str) -> Result<(), String> {
    let resp = client
        .get(format!("{url}/health"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    println!("{}", serde_json::to_string_pretty(&body).unwrap());
    Ok(())
}

/// List the cognition providers you can drive — the native adapters plus every
/// OpenAI-compatible preset — and how to start each. Runs fully offline; it
/// reads the built-in registry, not the server.
// Literal args are deliberate: this is a width-aligned table, so inlining them
// into the format string would break the column layout.
#[allow(clippy::print_literal)]
pub(crate) fn cmd_providers() -> Result<(), String> {
    use thymos_cognition::presets;

    brand_banner();
    println!("{}", paint("1", "Cognition providers — drive (almost) any model"));
    println!("Cognition only ever proposes intents; the runtime still governs every");
    println!("effect. API keys are read server-side — a provider name never carries one.");
    println!();

    println!("{}", paint("1", "Native adapters"));
    println!("  {:<13} {:<34} {}", "anthropic", "ANTHROPIC_API_KEY", "Claude (native Messages API)");
    println!("  {:<13} {:<34} {}", "openai", "OPENAI_API_KEY", "OpenAI (OPENAI_BASE_URL for any host)");
    println!("  {:<13} {:<34} {}", "mock", "(no key)", "deterministic, offline — the default");
    println!();

    println!("{}", paint("1", "OpenAI-compatible presets — cloud (set the key, go)"));
    for p in presets::all().iter().filter(|p| !p.local) {
        println!(
            "  {:<13} {:<34} e.g. {}",
            p.id,
            p.api_key_envs.join(" / "),
            p.default_model
        );
    }
    println!();

    println!("{}", paint("1", "Local runtimes — no key, just a running server"));
    for p in presets::all().iter().filter(|p| p.local) {
        println!("  {:<13} {:<34} e.g. {}", p.id, p.base_url, p.default_model);
    }
    println!();

    println!("{}", paint("1", "Start with any of them"));
    println!("  Make it the server default (every run uses it):");
    println!("    THYMOS_DEFAULT_PROVIDER=groq GROQ_API_KEY=… cargo run -p thymos-server");
    println!("  Pick per run:");
    println!("    thymos run \"…\" --provider openrouter --model openai/gpt-4o-mini");
    println!("  Point at ANY OpenAI-compatible endpoint:");
    println!("    thymos run \"…\" --provider openai --base-url https://host/v1 --model my-model");
    Ok(())
}

pub(crate) async fn cmd_doctor(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
) -> Result<(), String> {
    brand_banner();
    println!("{}", paint("1", "Thymos Terminal Doctor"));
    println!("endpoint: {url}");
    println!();

    status_line("thymos cli", true, env!("CARGO_PKG_VERSION"));
    status_line("api key", api_key.is_some(), mask_secret(api_key));

    for command in ["cargo", "node", "npm", "git"] {
        match command_version(command) {
            Some(version) => status_line(command, true, version),
            None => status_line(command, false, "not found on PATH"),
        }
    }

    println!();
    match client.get(format!("{url}/health")).send().await {
        Ok(resp) => {
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            status_line(
                "runtime health",
                status.is_success(),
                format!(
                    "{} {}",
                    status.as_u16(),
                    body["mode"].as_str().unwrap_or("unknown mode")
                ),
            );
            // The single most common first-run footgun: not knowing whether the
            // server is answering with a real model or the deterministic mock.
            let provider = body["default_provider"].as_str().unwrap_or("unknown");
            let live = body["cognition_live"].as_bool().unwrap_or(false);
            status_line(
                "cognition",
                live,
                if live {
                    format!("{provider} (live model)")
                } else {
                    format!(
                        "{provider} — MOCK, not a real model (set ANTHROPIC_API_KEY / OPENAI_API_KEY)"
                    )
                },
            );
        }
        Err(err) => status_line("runtime health", false, format!("offline: {err}")),
    }

    match client.get(format!("{url}/ready")).send().await {
        Ok(resp) => {
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            let checks = body["checks"]
                .as_object()
                .map(|obj| {
                    obj.iter()
                        .map(|(key, value)| {
                            format!(
                                "{key}={}",
                                if value.as_bool().unwrap_or(false) {
                                    "ok"
                                } else {
                                    "wait"
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| "no checks".into());
            status_line("runtime ready", status.is_success(), checks);
        }
        Err(err) => status_line("runtime ready", false, format!("offline: {err}")),
    }

    println!();
    status_line(
        "OPENAI_API_KEY",
        std::env::var_os("OPENAI_API_KEY").is_some(),
        mask_secret(std::env::var("OPENAI_API_KEY").ok().as_deref()),
    );
    status_line(
        "ANTHROPIC_API_KEY",
        std::env::var_os("ANTHROPIC_API_KEY").is_some(),
        mask_secret(std::env::var("ANTHROPIC_API_KEY").ok().as_deref()),
    );
    status_line(
        "HF_TOKEN",
        std::env::var_os("HF_TOKEN").is_some(),
        mask_secret(std::env::var("HF_TOKEN").ok().as_deref()),
    );
    status_line(
        "OPENAI_BASE_URL",
        std::env::var_os("OPENAI_BASE_URL").is_some(),
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "(not set)".into()),
    );

    println!();
    println!("{}", paint("38;2;199;125;255;1", "Next moves"));
    println!("  thymos config");
    println!("  thymos shell");
    println!("  thymos run \"Inspect this repo and explain the runtime\" --provider mock --follow");
    Ok(())
}

pub(crate) fn cmd_config(url: &str, api_key: Option<&str>) -> Result<(), String> {
    brand_banner();
    println!("{}", paint("1", "Terminal Configuration"));
    println!();
    let env_url = std::env::var("THYMOS_URL").ok();
    let installed_thymos = command_version("thymos");
    status_line("endpoint", true, url);
    status_line(
        "THYMOS_URL",
        env_url.is_some(),
        env_url.unwrap_or_else(|| format!("defaulting to {url}")),
    );
    status_line("THYMOS_API_KEY", api_key.is_some(), mask_secret(api_key));
    status_line(
        "config file",
        thymos_config_path().is_some(),
        thymos_config_path().unwrap_or_else(|| "~/.config/thymos/thymos.env (not found)".into()),
    );
    status_line(
        "installed thymos",
        installed_thymos.is_some(),
        installed_thymos.unwrap_or_else(|| "run scripts/install.sh".into()),
    );
    println!();
    println!("{}", paint("38;2;199;125;255;1", "Clean terminal workflow"));
    println!("  1. thymos doctor");
    println!("  2. thymos shell");
    println!("  3. set preset code");
    println!("  4. set workspace .");
    println!("  5. auto \"Inspect the repo, improve one small thing, and verify it\"");
    Ok(())
}

fn thymos_config_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = format!("{home}/.config/thymos/thymos.env");
    if std::path::Path::new(&path).exists() {
        Some(path)
    } else {
        None
    }
}

pub(crate) async fn cmd_approve(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
    channel: &str,
    approve: bool,
) -> Result<(), String> {
    let mut req = client
        .post(format!("{url}/runs/{run_id}/approvals/{channel}"))
        .json(&serde_json::json!({ "approve": approve }));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    println!("{}", serde_json::to_string_pretty(&body).unwrap());
    Ok(())
}

pub(crate) async fn cmd_runs_ls(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    status: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<(), String> {
    let mut endpoint = format!("{url}/runs?limit={limit}&offset={offset}");
    if let Some(s) = status {
        endpoint.push_str(&format!("&status={s}"));
    }
    let mut req = client.get(&endpoint);
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;

    let runs = body["runs"].as_array().cloned().unwrap_or_default();
    let total = body["total"].as_u64().unwrap_or(0);
    if runs.is_empty() {
        println!("(no runs)");
        return Ok(());
    }
    println!("{:<14}  {:<10}  TASK", "RUN ID", "STATUS");
    for r in &runs {
        let id = r["run_id"].as_str().unwrap_or("?");
        let st = r["status"].as_str().unwrap_or("?");
        let task = r["task"].as_str().unwrap_or("");
        let id_short: String = id.chars().take(12).collect();
        let task_short: String = task.chars().take(56).collect();
        println!("{id_short:<14}  {st:<10}  {task_short}");
    }
    println!();
    println!("({} of {total} shown, offset {offset})", runs.len());
    Ok(())
}

pub(crate) async fn cmd_runs_show(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
) -> Result<(), String> {
    // Status block.
    let mut req = client.get(format!("{url}/runs/{run_id}"));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let rec = json_body_or_error(resp).await?;
    println!("=== Run {run_id} ===");
    println!("{}", serde_json::to_string_pretty(&rec).unwrap_or_default());

    // Audit entries.
    let mut req = client.get(format!("{url}/audit/entries?run_id={run_id}&limit=200"));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    let entries = body["entries"].as_array().cloned().unwrap_or_default();
    println!("\n=== Ledger ({} entries) ===", entries.len());
    for e in &entries {
        let seq = e["seq"].as_u64().unwrap_or(0);
        let kind = e["kind"].as_str().unwrap_or("?");
        let id = e["id"].as_str().unwrap_or("");
        let id_short: String = id.chars().take(12).collect();
        println!("  #{seq:<4} {kind:<18} {id_short}");
    }
    Ok(())
}

pub(crate) async fn cmd_runs_diff(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    a: &str,
    b: &str,
) -> Result<(), String> {
    async fn summary(
        client: &reqwest::Client,
        url: &str,
        api_key: Option<&str>,
        run_id: &str,
    ) -> Result<(Value, Value), String> {
        let mut req = client.get(format!("{url}/runs/{run_id}"));
        for (k, v) in auth_headers(api_key) {
            req = req.header(&k, &v);
        }
        let rec = json_body_or_error(req.send().await.map_err(|e| e.to_string())?).await?;
        let mut req = client.get(format!("{url}/audit/entries?run_id={run_id}&limit=2000"));
        for (k, v) in auth_headers(api_key) {
            req = req.header(&k, &v);
        }
        let entries = json_body_or_error(req.send().await.map_err(|e| e.to_string())?).await?;
        Ok((rec, entries))
    }

    let (rec_a, ent_a) = summary(client, url, api_key, a).await?;
    let (rec_b, ent_b) = summary(client, url, api_key, b).await?;

    fn count_kind(entries: &Value, kind: &str) -> usize {
        entries["entries"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|e| e["kind"].as_str() == Some(kind))
                    .count()
            })
            .unwrap_or(0)
    }

    let commits_a = count_kind(&ent_a, "commit");
    let commits_b = count_kind(&ent_b, "commit");
    let rej_a = count_kind(&ent_a, "rejection");
    let rej_b = count_kind(&ent_b, "rejection");
    let final_a = rec_a["summary"]["final_answer"].as_str().unwrap_or("");
    let final_b = rec_b["summary"]["final_answer"].as_str().unwrap_or("");

    println!("           {:<22} {:<22} delta", a, b);
    println!(
        "status     {:<22} {:<22}",
        rec_a["status"].as_str().unwrap_or("?"),
        rec_b["status"].as_str().unwrap_or("?")
    );
    println!(
        "commits    {commits_a:<22} {commits_b:<22} {:+}",
        commits_b as i64 - commits_a as i64
    );
    println!(
        "rejections {rej_a:<22} {rej_b:<22} {:+}",
        rej_b as i64 - rej_a as i64
    );
    println!();
    if final_a == final_b {
        println!("final_answer: identical");
    } else {
        println!("final_answer DIFFERS:");
        println!("  a: {final_a}");
        println!("  b: {final_b}");
    }
    Ok(())
}

// Needed for the streaming SSE client.
mod futures_util {
    pub use tokio_stream::StreamExt;
}
