//! Interactive Thymos shell — a programmable terminal that wraps the
//! one-shot CLI verbs into a single persistent REPL session, plus an
//! `auto` autonomous loop that prompts inline for approvals.

use std::collections::HashSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;

use serde_json::Value;

use crate::{
    auth_headers, cmd_approve, cmd_cancel, cmd_health, cmd_replay, cmd_runs_diff, cmd_runs_ls,
    cmd_runs_show, cmd_status, cmd_stream, cmd_usage, cmd_world, json_body_or_error, start_run,
    StartRunOptions,
};

const VIOLET: &str = "\x1b[38;2;199;125;255m";
const GREEN: &str = "\x1b[38;2;52;211;153m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Persistent defaults carried across commands within one shell session.
struct ShellState {
    provider: String,
    model: Option<String>,
    max_steps: u32,
    scopes: Option<String>,
    follow: bool,
    /// Approval policy when stdin is not a TTY: "prompt" (default when interactive),
    /// "approve", or "deny".
    auto_approve: ApprovePolicy,
    last_run_id: Option<String>,
    /// Repo root for workspace-aware `run` / `auto`. When set, a preamble
    /// (git status + top-level entries + project memory) is prepended to the
    /// task, and `remember` appends to `{workspace}/.thymos/memory.md`.
    workspace: Option<PathBuf>,
    /// Label of the last-applied preset (for display only).
    preset: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApprovePolicy {
    Prompt,
    Approve,
    Deny,
}

impl ApprovePolicy {
    fn as_str(self) -> &'static str {
        match self {
            ApprovePolicy::Prompt => "prompt",
            ApprovePolicy::Approve => "approve",
            ApprovePolicy::Deny => "deny",
        }
    }
}

impl ShellState {
    fn new() -> Self {
        Self {
            provider: "mock".into(),
            model: None,
            max_steps: 16,
            scopes: None,
            follow: false,
            auto_approve: ApprovePolicy::Prompt,
            last_run_id: None,
            workspace: None,
            preset: "default",
        }
    }

    /// Expand the literal token `$last` to the most recent run id.
    fn expand(&self, token: &str) -> String {
        if token == "$last" {
            self.last_run_id
                .clone()
                .unwrap_or_else(|| "$last".to_string())
        } else {
            token.to_string()
        }
    }
}

pub(crate) async fn cmd_shell(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
) -> Result<(), String> {
    let mut state = ShellState::new();
    let interactive = io::stdin().is_terminal();

    if interactive {
        run_interactive(client, url, api_key, &mut state).await
    } else {
        run_piped(client, url, api_key, &mut state).await
    }
}

async fn run_interactive(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    state: &mut ShellState,
) -> Result<(), String> {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    let mut rl = DefaultEditor::new().map_err(|e| e.to_string())?;
    let history = history_path();
    if let Some(path) = history.as_ref() {
        let _ = rl.load_history(path);
    }

    print_shell_banner(url);

    loop {
        match rl.readline("thymos> ") {
            Ok(line) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let _ = rl.add_history_entry(trimmed);
                }
                match dispatch(trimmed, client, url, api_key, state).await {
                    Ok(Continue::Exit) => break,
                    Ok(Continue::Keep) => {}
                    Err(e) => eprintln!("error: {e}"),
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("(ctrl-c — type `exit` to quit)");
            }
            Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.to_string()),
        }
    }

    if let Some(path) = history.as_ref() {
        let _ = rl.save_history(path);
    }
    Ok(())
}

fn shell_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|term| term != "dumb")
            .unwrap_or(true)
}

fn c(code: &str) -> &str {
    if shell_color_enabled() {
        code
    } else {
        ""
    }
}

fn print_shell_banner(url: &str) {
    crate::brand_banner();
    println!(
        "  {}{}interactive shell{}   {}endpoint{} {}",
        c(VIOLET),
        c(BOLD),
        c(RESET),
        c(DIM),
        c(RESET),
        url
    );
    println!(
        "  {}flow{}   intent → proposal → commit → ledger",
        c(DIM),
        c(RESET)
    );
    println!(
        "  {}try{}    set preset code · set workspace . · run \"...\" --follow",
        c(GREEN),
        c(RESET)
    );
    println!(
        "  {}help{}   `help` for commands · `show` for config · `exit` to leave",
        c(DIM),
        c(RESET)
    );
    println!();
}

async fn run_piped(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    state: &mut ShellState,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        match dispatch(line.trim(), client, url, api_key, state).await {
            Ok(Continue::Exit) => break,
            Ok(Continue::Keep) => {}
            Err(e) => eprintln!("error: {e}"),
        }
    }
    Ok(())
}

enum Continue {
    Keep,
    Exit,
}

async fn dispatch(
    line: &str,
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    state: &mut ShellState,
) -> Result<Continue, String> {
    if line.is_empty() || line.starts_with('#') {
        return Ok(Continue::Keep);
    }
    let raw_tokens = shlex::split(line).ok_or("parse error: unbalanced quotes")?;
    if raw_tokens.is_empty() {
        return Ok(Continue::Keep);
    }
    let tokens: Vec<String> = raw_tokens.iter().map(|t| state.expand(t)).collect();
    // Tolerate a redundant leading `thymos` — users arriving from the CLI often
    // type `thymos health` inside the shell, where the command is just `health`.
    let start = if tokens.len() > 1 && tokens[0] == "thymos" { 1 } else { 0 };
    let cmd = tokens[start].as_str();
    let args: Vec<&str> = tokens[start + 1..].iter().map(String::as_str).collect();

    match cmd {
        "exit" | "quit" => return Ok(Continue::Exit),
        "help" | "thymos" => print_help(),
        "show" => print_state(state),
        "set" => set_value(state, &args)?,
        "health" => cmd_health(client, url).await?,
        "usage" => cmd_usage(client, url, api_key).await?,
        "status" => {
            let id = require_arg(&args, 0, "status <run-id>")?;
            state.last_run_id = Some(id.to_string());
            cmd_status(client, url, api_key, id).await?;
        }
        "stream" => {
            let id = require_arg(&args, 0, "stream <run-id>")?;
            state.last_run_id = Some(id.to_string());
            cmd_stream(url, id).await?;
        }
        "world" => {
            let id = require_arg(&args, 0, "world <run-id>")?;
            state.last_run_id = Some(id.to_string());
            cmd_world(client, url, api_key, id).await?;
        }
        "replay" => {
            let id = require_arg(&args, 0, "replay <run-id> [--require-compiler V] [--json]")?;
            let raw_json = args.contains(&"--json");
            let require_compiler = args
                .iter()
                .position(|arg| *arg == "--require-compiler")
                .and_then(|idx| args.get(idx + 1).copied());
            state.last_run_id = Some(id.to_string());
            cmd_replay(client, url, api_key, id, require_compiler, raw_json).await?;
        }
        "cancel" => {
            let id = require_arg(&args, 0, "cancel <run-id>")?;
            state.last_run_id = Some(id.to_string());
            cmd_cancel(client, url, api_key, id).await?;
        }
        "approve" => {
            let id = require_arg(&args, 0, "approve <run-id> <channel> [--deny]")?;
            let channel = require_arg(&args, 1, "approve <run-id> <channel> [--deny]")?;
            let deny = args.contains(&"--deny");
            state.last_run_id = Some(id.to_string());
            cmd_approve(client, url, api_key, id, channel, !deny).await?;
        }
        "deny" => {
            let id = require_arg(&args, 0, "deny <run-id> <channel>")?;
            let channel = require_arg(&args, 1, "deny <run-id> <channel>")?;
            state.last_run_id = Some(id.to_string());
            cmd_approve(client, url, api_key, id, channel, false).await?;
        }
        "run" => {
            let parsed = parse_run_args(&args, state)?;
            let full_task = compose_task(&parsed.task, state.workspace.as_deref());
            let run_id = start_run(
                client,
                url,
                api_key,
                StartRunOptions {
                    task: &full_task,
                    max_steps: parsed.max_steps,
                    provider: &parsed.provider,
                    model: parsed.model.as_deref(),
                    base_url: None,
                    scopes: parsed.scopes.as_deref(),
                    skill: None,
                    skill_params: &[],
                },
            )
            .await?;
            state.last_run_id = Some(run_id.clone());
            println!("Run started: {run_id}");
            println!("  task: {}", parsed.task);
            println!("  provider: {}", parsed.provider);
            if let Some(workspace) = &state.workspace {
                println!("  workspace: {}", workspace.display());
            }
            if parsed.follow {
                println!("--- streaming ---");
                cmd_stream(url, &run_id).await?;
                cmd_status(client, url, api_key, &run_id).await?;
            }
        }
        "auto" => {
            let parsed = parse_run_args(&args, state)?;
            let full_task = compose_task(&parsed.task, state.workspace.as_deref());
            let run_id = start_run(
                client,
                url,
                api_key,
                StartRunOptions {
                    task: &full_task,
                    max_steps: parsed.max_steps,
                    provider: &parsed.provider,
                    model: parsed.model.as_deref(),
                    base_url: None,
                    scopes: parsed.scopes.as_deref(),
                    skill: None,
                    skill_params: &[],
                },
            )
            .await?;
            state.last_run_id = Some(run_id.clone());
            auto_loop(client, url, api_key, &run_id, state).await?;
        }
        "remember" => remember(state, &args)?,
        "runs" => {
            let sub = require_arg(&args, 0, "runs <ls|show|diff> ...")?;
            match sub {
                "ls" => {
                    let (status_flag, limit, offset) = parse_runs_ls(&args[1..])?;
                    cmd_runs_ls(client, url, api_key, status_flag.as_deref(), limit, offset)
                        .await?;
                }
                "show" => {
                    let id = require_arg(&args, 1, "runs show <run-id>")?;
                    state.last_run_id = Some(id.to_string());
                    cmd_runs_show(client, url, api_key, id).await?;
                }
                "diff" => {
                    let a = require_arg(&args, 1, "runs diff <a> <b>")?;
                    let b = require_arg(&args, 2, "runs diff <a> <b>")?;
                    cmd_runs_diff(client, url, api_key, a, b).await?;
                }
                other => return Err(format!("unknown runs subcommand: {other}")),
            }
        }
        other => return Err(format!("unknown command: {other} (try `help`)")),
    }

    Ok(Continue::Keep)
}

fn require_arg<'a>(args: &[&'a str], idx: usize, usage: &str) -> Result<&'a str, String> {
    args.get(idx)
        .copied()
        .ok_or_else(|| format!("usage: {usage}"))
}

struct ParsedRun {
    task: String,
    max_steps: u32,
    provider: String,
    model: Option<String>,
    scopes: Option<String>,
    follow: bool,
}

fn parse_run_args(args: &[&str], state: &ShellState) -> Result<ParsedRun, String> {
    let mut task: Option<String> = None;
    let mut max_steps = state.max_steps;
    let mut provider = state.provider.clone();
    let mut model = state.model.clone();
    let mut scopes = state.scopes.clone();
    let mut follow = state.follow;

    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        match a {
            "--provider" => {
                provider = args
                    .get(i + 1)
                    .ok_or("--provider needs a value")?
                    .to_string();
                i += 2;
            }
            "--model" => {
                model = Some(args.get(i + 1).ok_or("--model needs a value")?.to_string());
                i += 2;
            }
            "--max-steps" => {
                let v = args.get(i + 1).ok_or("--max-steps needs a value")?;
                max_steps = v.parse().map_err(|_| format!("bad --max-steps: {v}"))?;
                i += 2;
            }
            "--scopes" => {
                scopes = Some(args.get(i + 1).ok_or("--scopes needs a value")?.to_string());
                i += 2;
            }
            "--follow" | "-f" => {
                follow = true;
                i += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            _ => {
                if task.is_none() {
                    task = Some(a.to_string());
                } else {
                    return Err("task already set — quote multi-word tasks".into());
                }
                i += 1;
            }
        }
    }

    Ok(ParsedRun {
        task: task.ok_or("usage: run <task> [--provider ...] [--model ...] [--follow]")?,
        max_steps,
        provider,
        model,
        scopes,
        follow,
    })
}

fn parse_runs_ls(args: &[&str]) -> Result<(Option<String>, u32, u32), String> {
    let mut status: Option<String> = None;
    let mut limit = 50u32;
    let mut offset = 0u32;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--status" => {
                status = Some(args.get(i + 1).ok_or("--status needs a value")?.to_string());
                i += 2;
            }
            "--limit" => {
                let v = args.get(i + 1).ok_or("--limit needs a value")?;
                limit = v.parse().map_err(|_| format!("bad --limit: {v}"))?;
                i += 2;
            }
            "--offset" => {
                let v = args.get(i + 1).ok_or("--offset needs a value")?;
                offset = v.parse().map_err(|_| format!("bad --offset: {v}"))?;
                i += 2;
            }
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    Ok((status, limit, offset))
}

fn set_value(state: &mut ShellState, args: &[&str]) -> Result<(), String> {
    let key = require_arg(args, 0, "set <key> <value>")?;
    let value = require_arg(args, 1, "set <key> <value>")?;
    match key {
        "provider" => state.provider = value.to_string(),
        "model" => {
            state.model = if value == "none" {
                None
            } else {
                Some(value.to_string())
            }
        }
        "max_steps" | "max-steps" => {
            state.max_steps = value
                .parse()
                .map_err(|_| format!("bad max_steps: {value}"))?;
        }
        "scopes" => {
            state.scopes = if value == "none" {
                None
            } else {
                Some(value.to_string())
            }
        }
        "follow" => state.follow = parse_bool(value)?,
        "auto_approve" | "auto-approve" => {
            state.auto_approve = match value {
                "prompt" => ApprovePolicy::Prompt,
                "approve" | "yes" | "on" => ApprovePolicy::Approve,
                "deny" | "no" | "off" => ApprovePolicy::Deny,
                _ => {
                    return Err(format!(
                        "auto_approve: expected prompt|approve|deny, got {value}"
                    ))
                }
            }
        }
        "workspace" => {
            if value == "none" {
                state.workspace = None;
            } else {
                let path = PathBuf::from(value);
                let canon = path
                    .canonicalize()
                    .map_err(|e| format!("workspace {value}: {e}"))?;
                if !canon.is_dir() {
                    return Err(format!("workspace {value}: not a directory"));
                }
                state.workspace = Some(canon);
            }
        }
        "preset" => match value {
            "code" | "coding" => {
                state.max_steps = 64;
                state.scopes = Some(
                    "fs_read,fs_patch,list_files,repo_map,grep,test_run,memory_store,memory_recall"
                        .into(),
                );
                state.preset = "coding";
            }
            "default" => {
                state.max_steps = 16;
                state.scopes = None;
                state.preset = "default";
            }
            _ => return Err(format!("unknown preset: {value} (try code|default)")),
        },
        other => return Err(format!("unknown setting: {other}")),
    }
    println!("ok");
    Ok(())
}

fn remember(state: &ShellState, args: &[&str]) -> Result<(), String> {
    let ws = state
        .workspace
        .as_ref()
        .ok_or("remember needs a workspace — `set workspace <path>` first")?;
    if args.is_empty() {
        return Err("usage: remember <text>".into());
    }
    let text = args.join(" ");
    let dir = ws.join(".thymos");
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join("memory.md");
    let line = format!("- {}\n", text);
    use std::fs::OpenOptions;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open {}: {e}", path.display()))?
        .write_all(line.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    println!("remembered → {}", path.display());
    Ok(())
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        other => Err(format!("expected on/off, got {other}")),
    }
}

fn print_state(state: &ShellState) {
    println!("provider      {}", state.provider);
    println!(
        "model         {}",
        state.model.as_deref().unwrap_or("(default)")
    );
    println!("max_steps     {}", state.max_steps);
    println!(
        "scopes        {}",
        state.scopes.as_deref().unwrap_or("(default)")
    );
    println!("follow        {}", state.follow);
    println!("auto_approve  {}", state.auto_approve.as_str());
    println!("preset        {}", state.preset);
    println!(
        "workspace     {}",
        state
            .workspace
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(none)".into())
    );
    println!(
        "last_run_id   {}",
        state.last_run_id.as_deref().unwrap_or("(none)")
    );
}

fn print_help() {
    println!(
        "\
Thymos shell commands:
  run <task> [--provider P] [--model M] [--max-steps N] [--scopes a,b] [--follow]
  auto <task> [flags...]        run, then poll + prompt for approvals until done
  status <run-id>               fetch status + summary
  stream <run-id>               tail SSE events
  world <run-id>                dump world state
  replay <run-id> [flags]       verify and fold the execution ledger
  approve <run-id> <chan> [--deny]
  deny <run-id> <chan>
  cancel <run-id>
  runs ls [--status S] [--limit N] [--offset N]
  runs show <run-id>
  runs diff <a> <b>
  health                        GET /health
  usage                         GET /usage
  set <key> <value>             provider|model|max_steps|scopes|follow|auto_approve|
                                 workspace|preset
  preset code                   max_steps=64, coding toolkit scopes
  workspace <path>              repo root; enables preamble + .thymos/memory.md
  remember <text>               append note to {{workspace}}/.thymos/memory.md
  show                          print session defaults + last run id
  help                          this message
  exit | quit                   leave the shell

Tokens: `$last` expands to the most recent run id.
Lines starting with `#` are comments.
The shell attaches to the same Thymos runtime the web console and VS Code sidebar use."
    );
}

/// Build the final task string sent to the server. When a workspace is set,
/// prepend a preamble with git status, top-level listing, and any
/// `.thymos/memory.md` content so the agent starts grounded.
fn compose_task(task: &str, workspace: Option<&Path>) -> String {
    match workspace {
        None => task.to_string(),
        Some(ws) => format!("{}\n\n## Task\n{task}", workspace_preamble(ws)),
    }
}

fn workspace_preamble(ws: &Path) -> String {
    let mut out = String::new();
    out.push_str("## Workspace\n\n");
    out.push_str(&format!("Repo root: `{}`\n", ws.display()));

    if let Ok(output) = ProcessCommand::new("git")
        .arg("-C")
        .arg(ws)
        .args(["status", "--short", "--branch"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push_str("\n### git status\n```\n");
                out.push_str(trimmed);
                out.push_str("\n```\n");
            }
        }
    }

    if let Ok(entries) = fs::read_dir(ws) {
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        names.truncate(40);
        if !names.is_empty() {
            out.push_str("\n### top-level\n");
            out.push_str(&names.join(", "));
            out.push('\n');
        }
    }

    let mem = ws.join(".thymos").join("memory.md");
    if let Ok(content) = fs::read_to_string(&mem) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            out.push_str("\n### project memory\n");
            out.push_str(trimmed);
            out.push('\n');
        }
    }

    out
}

fn history_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".thymos_history"))
}

/// Poll the run to completion, surfacing each unseen pending-approval ledger
/// entry as an inline prompt (or applying the configured policy when no TTY).
async fn auto_loop(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
    state: &ShellState,
) -> Result<(), String> {
    println!(
        "[auto] run {run_id} — polling; {} approvals",
        state.auto_approve.as_str()
    );
    let mut handled_seqs: HashSet<u64> = HashSet::new();
    let mut last_printed_status: Option<String> = None;
    let mut last_log_idx = 0u64;

    loop {
        let execution = fetch_execution(client, url, api_key, run_id).await?;
        let st = execution["status"].as_str().unwrap_or("?").to_string();
        let phase = execution["phase"].as_str().unwrap_or("?");
        let operator = execution["operator_state"].as_str().unwrap_or("");
        if last_printed_status.as_deref() != Some(st.as_str()) {
            println!("[auto] status: {st} ({phase})");
            last_printed_status = Some(st.clone());
        }
        if !operator.is_empty() {
            println!("[auto] {operator}");
        }
        if let Some(entries) = execution["log"].as_array() {
            for entry in entries {
                let idx = entry["idx"].as_u64().unwrap_or(0);
                if idx <= last_log_idx {
                    continue;
                }
                print_execution_entry(entry);
                last_log_idx = idx;
            }
        }
        if matches!(st.as_str(), "completed" | "failed" | "cancelled") {
            print_final(&execution);
            return Ok(());
        }

        let entries = fetch_pending_entries(client, url, api_key, run_id).await?;
        for entry in entries {
            let seq = entry["seq"].as_u64().unwrap_or_default();
            if handled_seqs.contains(&seq) {
                continue;
            }
            let detail = &entry["detail"];
            let channel = detail["channel"].as_str().unwrap_or("?");
            let reason = detail["reason"].as_str().unwrap_or("(no reason)");
            let review = render_review(
                channel,
                reason,
                detail.get("proposal"),
                state.workspace.as_deref(),
            );
            let decision = resolve_approval(state, &review);
            match decision {
                ApprovalDecision::Skip => {
                    return Err(format!(
                        "pending approval on channel `{channel}` (reason: {reason}) and no interactive TTY — set auto_approve approve|deny or re-run in a TTY"
                    ));
                }
                ApprovalDecision::Approve | ApprovalDecision::Deny => {
                    let approve = matches!(decision, ApprovalDecision::Approve);
                    println!(
                        "[auto] {}: channel={channel}",
                        if approve { "approving" } else { "denying" }
                    );
                    cmd_approve(client, url, api_key, run_id, channel, approve).await?;
                    handled_seqs.insert(seq);
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

enum ApprovalDecision {
    Approve,
    Deny,
    Skip,
}

fn resolve_approval(state: &ShellState, review: &str) -> ApprovalDecision {
    match state.auto_approve {
        ApprovePolicy::Approve => ApprovalDecision::Approve,
        ApprovePolicy::Deny => ApprovalDecision::Deny,
        ApprovePolicy::Prompt => {
            if !io::stdin().is_terminal() {
                return ApprovalDecision::Skip;
            }
            println!("{review}");
            print!("  approve? [y/N] ");
            let _ = io::stdout().flush();
            let mut line = String::new();
            match io::stdin().read_line(&mut line) {
                Ok(_) => {
                    let t = line.trim().to_lowercase();
                    if matches!(t.as_str(), "y" | "yes") {
                        ApprovalDecision::Approve
                    } else {
                        ApprovalDecision::Deny
                    }
                }
                Err(_) => ApprovalDecision::Deny,
            }
        }
    }
}

/// Render a human-readable review block for a pending proposal. Uses the
/// proposal detail surfaced by /audit/entries (tool name + args) to show
/// *what* the agent is about to do, not just the policy reason. When a
/// workspace is set and the proposal is an `fs_patch`, render a unified
/// line-level diff against the on-disk file.
fn render_review(
    channel: &str,
    reason: &str,
    proposal: Option<&Value>,
    workspace: Option<&Path>,
) -> String {
    let mut out = String::new();
    out.push_str("\n──────── Approval Required ────────\n");
    out.push_str(&format!("channel  {channel}\n"));
    out.push_str(&format!("reason   {reason}\n"));

    if let Some(p) = proposal.and_then(|v| v.get("body")) {
        let tool = p["plan"]["tool"].as_str().unwrap_or("?");
        let args = &p["plan"]["args"];
        out.push_str(&format!("tool     {tool}\n"));

        // Summarise common coding-tool args succinctly.
        match tool {
            "fs_patch" => {
                let path = args["path"].as_str().unwrap_or("?");
                let mode = args["mode"].as_str().unwrap_or("?");
                out.push_str(&format!("path     {path}\n"));
                out.push_str(&format!("mode     {mode}\n"));
                if let Some(diff) = unified_diff_for_patch(workspace, path, mode, args) {
                    out.push_str("diff\n");
                    out.push_str(&diff);
                    if !diff.ends_with('\n') {
                        out.push('\n');
                    }
                } else if mode == "replace" {
                    let anchor = args["anchor"].as_str().unwrap_or("").lines().count();
                    let replace = args["replacement"].as_str().unwrap_or("").lines().count();
                    out.push_str(&format!("diff     -{anchor} +{replace} lines\n"));
                } else if let Some(content) = args["content"].as_str() {
                    out.push_str(&format!("write    {} bytes\n", content.len()));
                }
            }
            "test_run" | "shell" => {
                if let Some(cmd) = args["command"].as_str() {
                    out.push_str(&format!("command  {cmd}\n"));
                }
                if let Some(runner) = args["runner"].as_str() {
                    out.push_str(&format!("runner   {runner}\n"));
                }
            }
            _ => {
                let pretty =
                    serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
                out.push_str("args\n");
                for line in pretty.lines() {
                    out.push_str(&format!("  {line}\n"));
                }
            }
        }
    }
    out.push_str("───────────────────────────────────");
    out
}

async fn fetch_execution(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
) -> Result<Value, String> {
    let mut req = client.get(format!("{url}/runs/{run_id}/execution"));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    json_body_or_error(resp).await
}

async fn fetch_pending_entries(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    run_id: &str,
) -> Result<Vec<Value>, String> {
    let mut req = client.get(format!(
        "{url}/audit/entries?run_id={run_id}&kind=pending_approval&limit=200"
    ));
    for (k, v) in auth_headers(api_key) {
        req = req.header(&k, &v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let body = json_body_or_error(resp).await?;
    Ok(body["entries"].as_array().cloned().unwrap_or_default())
}

fn print_final(status: &Value) {
    if let Some(answer) = status.get("final_answer").and_then(|v| v.as_str()) {
        println!("--- final answer ---");
        println!("{answer}");
    }
    if let Some(commits) = status
        .get("counters")
        .and_then(|v| v.get("commits"))
        .and_then(|v| v.as_u64())
    {
        println!("[auto] commits: {commits}");
    }
    if let Some(summary) = status.get("summary") {
        if let Some(answer) = summary.get("final_answer").and_then(|v| v.as_str()) {
            println!("--- final answer ---");
            println!("{answer}");
        }
        if let Some(commits) = summary.get("commits").and_then(|v| v.as_u64()) {
            println!("[auto] commits: {commits}");
        }
    }
}

fn print_execution_entry(entry: &Value) {
    let phase = entry["phase"].as_str().unwrap_or("?");
    let level = entry["level"].as_str().unwrap_or("info");
    let title = entry["title"].as_str().unwrap_or("");
    let detail = entry["detail"].as_str().unwrap_or("");
    let step = entry["step_index"]
        .as_u64()
        .map(|n| format!(" step {}", n + 1))
        .unwrap_or_default();
    let tool = entry["tool"]
        .as_str()
        .map(|tool| format!(" {tool}"))
        .unwrap_or_default();

    println!(
        "[{}:{}{}{}] {}",
        level.to_uppercase(),
        phase,
        step,
        tool,
        title
    );
    if !detail.is_empty() {
        println!("  {detail}");
    }
}

/// Read the file on disk and produce a unified line-diff against the
/// proposed content. Returns None if we can't resolve the path (no
/// workspace) or the `fs_patch` args are malformed. Long diffs are
/// truncated to keep the review prompt readable.
fn unified_diff_for_patch(
    workspace: Option<&Path>,
    path: &str,
    mode: &str,
    args: &Value,
) -> Option<String> {
    let ws = workspace?;
    let full = {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            ws.join(p)
        }
    };
    let current = fs::read_to_string(&full).unwrap_or_default();

    let proposed = match mode {
        "write" => args["content"].as_str()?.to_string(),
        "replace" => {
            let anchor = args["anchor"].as_str()?;
            let replacement = args["replacement"].as_str()?;
            if current.is_empty() {
                // File missing — we can still show the proposed block as a pure add.
                replacement.to_string()
            } else {
                // Preview the would-be file; fall back to plain splice view if
                // the anchor is ambiguous or absent.
                match current.match_indices(anchor).count() {
                    1 => current.replace(anchor, replacement),
                    _ => return Some(format_replace_fallback(anchor, replacement)),
                }
            }
        }
        _ => return None,
    };

    Some(format_unified_diff(&current, &proposed))
}

fn format_unified_diff(old: &str, new: &str) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();
    let mut printed = 0usize;
    const MAX_LINES: usize = 80;

    for change in diff.iter_all_changes() {
        if printed >= MAX_LINES {
            out.push_str("  … (diff truncated)\n");
            break;
        }
        let marker = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        // Only show context around changes: skip Equal lines beyond a small window.
        if matches!(change.tag(), ChangeTag::Equal) {
            continue;
        }
        out.push_str(&format!("  {marker} {}", change.value()));
        if !change.value().ends_with('\n') {
            out.push('\n');
        }
        printed += 1;
    }

    if printed == 0 {
        out.push_str("  (no textual change)\n");
    }
    out
}

fn format_replace_fallback(anchor: &str, replacement: &str) -> String {
    let mut out = String::new();
    for line in anchor.lines().take(40) {
        out.push_str(&format!("  - {line}\n"));
    }
    for line in replacement.lines().take(40) {
        out.push_str(&format!("  + {line}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_write_mode_shows_additions_and_deletions() {
        let tmp = std::env::temp_dir().join(format!("thymos-diff-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "one\ntwo\nthree\n").unwrap();

        let args = serde_json::json!({
            "content": "one\nTWO\nthree\nfour\n",
        });
        let diff = unified_diff_for_patch(Some(&tmp), "a.txt", "write", &args).unwrap();
        assert!(diff.contains("- two"), "missing deletion: {diff}");
        assert!(diff.contains("+ TWO"), "missing insertion: {diff}");
        assert!(diff.contains("+ four"), "missing added line: {diff}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unified_diff_replace_mode_uses_anchor_splice() {
        let tmp = std::env::temp_dir().join(format!("thymos-diff2-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("b.txt"), "alpha\nbeta\ngamma\n").unwrap();

        let args = serde_json::json!({
            "anchor": "beta",
            "replacement": "BETA",
        });
        let diff = unified_diff_for_patch(Some(&tmp), "b.txt", "replace", &args).unwrap();
        assert!(diff.contains("- beta"), "missing old anchor: {diff}");
        assert!(diff.contains("+ BETA"), "missing new text: {diff}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unified_diff_returns_none_without_workspace() {
        let args = serde_json::json!({ "content": "anything" });
        assert!(unified_diff_for_patch(None, "x.txt", "write", &args).is_none());
    }
}
