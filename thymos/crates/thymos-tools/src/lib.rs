//! Tool Contract runtime.
//!
//! A ToolContract is not a function — it is a bounded operational contract
//! that declares:
//!   - input / output schemas (Phase 1: validation is delegated to the impl),
//!   - effect class and risk class,
//!   - preconditions and postconditions evaluated against the `World`,
//!   - the state delta produced by a successful execution.
//!
//! Phase 1 executes synchronously, single-phase. Phase 2+ will introduce
//! two-phase staging with compensation registration.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use thymos_core::{
    commit::Observation,
    delta::StructuredDelta,
    error::{Error, Result},
    world::World,
    writ::BudgetCost,
};

pub mod coding;
pub use coding::{
    CodingSandbox, FsPatchTool, FsReadTool, GrepTool, ListFilesTool, RepoMapTool, TestRunTool,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    Pure,
    Read,
    Write,
    External,
    Irreversible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolContractMeta {
    pub name: String,
    pub version: String,
    pub effect_class: EffectClass,
    pub risk_class: RiskClass,
}

pub struct ToolInvocation<'a> {
    pub args: &'a Value,
    pub world: &'a World,
}

pub struct ToolOutcome {
    pub delta: StructuredDelta,
    pub observation: Observation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    InProcess,
    Worker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellCapabilityProfile {
    Inspect,
    Build,
    Mutate,
    Networked,
}

impl ShellCapabilityProfile {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "inspect" => Ok(Self::Inspect),
            "build" => Ok(Self::Build),
            "mutate" => Ok(Self::Mutate),
            "networked" => Ok(Self::Networked),
            other => Err(Error::ToolExecution(format!(
                "unsupported shell capability profile '{other}'"
            ))),
        }
    }

    fn allowed_commands(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Inspect => Some(&[
                "ls", "pwd", "cat", "head", "tail", "sed", "awk", "cut", "sort", "uniq", "wc",
                "rg", "find", "stat", "git", "env", "printenv", "which", "echo", "printf",
            ]),
            Self::Build => Some(&[
                "ls", "pwd", "cat", "head", "tail", "sed", "awk", "cut", "sort", "uniq", "wc",
                "rg", "find", "stat", "git", "env", "printenv", "which", "echo", "printf", "cargo",
                "rustc", "rustfmt", "make", "npm", "pnpm", "yarn", "bun", "go", "pytest",
            ]),
            Self::Mutate => Some(&[
                "ls", "pwd", "cat", "head", "tail", "sed", "awk", "cut", "sort", "uniq", "wc",
                "rg", "find", "stat", "git", "env", "printenv", "which", "echo", "printf", "cargo",
                "rustc", "rustfmt", "make", "npm", "pnpm", "yarn", "bun", "go", "pytest", "cp",
                "mv", "mkdir", "touch", "chmod", "rm",
            ]),
            Self::Networked => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolWorkerRequest {
    Shell {
        command: String,
        cwd: Option<String>,
        timeout_secs: u64,
        purpose: Option<String>,
        capability_profile: String,
        restricted_env: bool,
        env: BTreeMap<String, String>,
        max_output_bytes: usize,
        blocked_patterns: Vec<String>,
        wrapper: Vec<String>,
        allowed_roots: Vec<String>,
        isolate_home: bool,
    },
    Http {
        url: String,
        method: String,
        body: Option<String>,
        headers: BTreeMap<String, String>,
        allowlist: Vec<String>,
        timeout_secs: u64,
        block_private_hosts: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolWorkerResponse {
    pub kind: String,
    pub output: Value,
    pub latency_ms: u64,
}

fn worker_receipt(kind: &str, payload: &Value) -> Value {
    let canonical = serde_json::to_vec(payload).unwrap_or_default();
    let receipt = blake3::hash(&canonical).to_hex().to_string();
    serde_json::json!({
        "kind": kind,
        "receipt_id": receipt,
    })
}

fn in_process_worker_execute(req: ToolWorkerRequest) -> Result<ToolWorkerResponse> {
    match req {
        ToolWorkerRequest::Shell {
            command,
            cwd,
            timeout_secs,
            purpose,
            capability_profile,
            restricted_env,
            env,
            max_output_bytes,
            blocked_patterns,
            wrapper,
            allowed_roots,
            isolate_home,
        } => execute_shell_request(ShellExecutionRequest {
            command: &command,
            cwd: cwd.as_deref(),
            timeout_secs,
            purpose: purpose.as_deref(),
            capability_profile: &capability_profile,
            restricted_env,
            env: &env,
            max_output_bytes,
            blocked_patterns: &blocked_patterns,
            wrapper: &wrapper,
            allowed_roots: &allowed_roots,
            isolate_home,
        }),
        ToolWorkerRequest::Http {
            url,
            method,
            body,
            headers,
            allowlist,
            timeout_secs,
            block_private_hosts,
        } => execute_http_request(
            &url,
            &method,
            body.as_deref(),
            &headers,
            &allowlist,
            timeout_secs,
            block_private_hosts,
        ),
    }
}

fn subprocess_worker_execute(
    worker_bin: &str,
    req: &ToolWorkerRequest,
) -> Result<ToolWorkerResponse> {
    use std::process::{Command, Stdio};

    let mut child = Command::new(worker_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::ToolExecution(format!("spawn worker failed: {e}")))?;

    let request_bytes =
        serde_json::to_vec(req).map_err(|e| Error::ToolExecution(format!("worker encode: {e}")))?;

    child
        .stdin
        .as_mut()
        .ok_or_else(|| Error::ToolExecution("worker stdin unavailable".into()))?
        .write_all(&request_bytes)
        .map_err(|e| Error::ToolExecution(format!("worker write: {e}")))?;

    let output = child
        .wait_with_output()
        .map_err(|e| Error::ToolExecution(format!("worker wait: {e}")))?;

    if !output.status.success() {
        return Err(Error::ToolExecution(format!(
            "worker failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    serde_json::from_slice::<ToolWorkerResponse>(&output.stdout)
        .map_err(|e| Error::ToolExecution(format!("worker decode: {e}")))
}

pub fn worker_entrypoint() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| Error::ToolExecution(format!("worker stdin read: {e}")))?;

    let request = serde_json::from_slice::<ToolWorkerRequest>(&input)
        .map_err(|e| Error::ToolExecution(format!("worker request decode: {e}")))?;
    let response = in_process_worker_execute(request)?;
    let output = serde_json::to_vec(&response)
        .map_err(|e| Error::ToolExecution(format!("worker response encode: {e}")))?;
    std::io::stdout()
        .write_all(&output)
        .map_err(|e| Error::ToolExecution(format!("worker stdout write: {e}")))?;
    Ok(())
}

struct ShellExecutionRequest<'a> {
    command: &'a str,
    cwd: Option<&'a str>,
    timeout_secs: u64,
    purpose: Option<&'a str>,
    capability_profile: &'a str,
    restricted_env: bool,
    env: &'a BTreeMap<String, String>,
    max_output_bytes: usize,
    blocked_patterns: &'a [String],
    wrapper: &'a [String],
    allowed_roots: &'a [String],
    isolate_home: bool,
}

fn execute_shell_request(req: ShellExecutionRequest<'_>) -> Result<ToolWorkerResponse> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    validate_shell_command(req.command, req.capability_profile, req.blocked_patterns)?;
    let profile = ShellCapabilityProfile::parse(req.capability_profile)?;
    let exec_cwd = resolve_working_dir(req.cwd, req.allowed_roots)?;
    let isolated_home = if req.restricted_env && req.isolate_home {
        Some(create_isolated_home_dir()?)
    } else {
        None
    };

    let result = (|| -> Result<ToolWorkerResponse> {
        let start = Instant::now();
        let mut cmd = if req.wrapper.is_empty() {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(req.command);
            c
        } else {
            let mut c = Command::new(&req.wrapper[0]);
            for arg in &req.wrapper[1..] {
                c.arg(arg);
            }
            c.arg("/bin/sh").arg("-c").arg(req.command);
            c
        };
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(dir) = &exec_cwd {
            cmd.current_dir(dir);
        }

        if req.restricted_env {
            cmd.env_clear();
            if let Ok(path) = std::env::var("PATH") {
                cmd.env("PATH", path);
            }
            if let Some(home) = &isolated_home {
                cmd.env("HOME", home);
            } else if let Ok(home) = std::env::var("HOME") {
                cmd.env("HOME", home);
            }
        }

        for (k, v) in req.env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::ToolExecution(format!("spawn failed: {e}")))?;
        let timeout = Duration::from_secs(req.timeout_secs);

        loop {
            if let Some(_status) = child
                .try_wait()
                .map_err(|e| Error::ToolExecution(format!("wait failed: {e}")))?
            {
                break;
            }

            if start.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::ToolExecution(format!(
                    "command timed out after {}s",
                    req.timeout_secs
                )));
            }

            std::thread::sleep(Duration::from_millis(25));
        }

        let output = child
            .wait_with_output()
            .map_err(|e| Error::ToolExecution(format!("collect output failed: {e}")))?;
        let elapsed = start.elapsed();

        let stdout_bytes = &output.stdout[..output.stdout.len().min(req.max_output_bytes)];
        let stderr_bytes = &output.stderr[..output
            .stderr
            .len()
            .min(req.max_output_bytes.saturating_sub(stdout_bytes.len()))];
        let stdout = String::from_utf8_lossy(stdout_bytes).to_string();
        let stderr = String::from_utf8_lossy(stderr_bytes).to_string();
        let exit_code = output.status.code().unwrap_or(-1);
        let truncated = output.stdout.len() + output.stderr.len() > req.max_output_bytes;

        let receipt_payload = serde_json::json!({
            "worker_boundary": "subprocess_contract",
            "runtime": "thymos_secure_shell",
            "purpose": req.purpose,
            "capability_profile": req.capability_profile,
            "cwd": exec_cwd.as_ref().map(|p| p.display().to_string()),
            "restricted_env": req.restricted_env,
            "isolated_home": isolated_home.as_ref().map(|p| p.display().to_string()),
            "allowed_roots": req.allowed_roots,
            "command_digest": blake3::hash(req.command.as_bytes()).to_hex().to_string(),
            "profile": format!("{profile:?}").to_lowercase(),
        });
        let mut receipt = worker_receipt("shell", &receipt_payload);
        if let Some(obj) = receipt.as_object_mut() {
            obj.extend(receipt_payload.as_object().cloned().unwrap_or_default());
        }

        Ok(ToolWorkerResponse {
            kind: "shell".into(),
            output: serde_json::json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": exit_code,
                "truncated": truncated,
                "receipt": receipt,
            }),
            latency_ms: elapsed.as_millis() as u64,
        })
    })();

    if let Some(home) = isolated_home {
        let _ = std::fs::remove_dir_all(home);
    }

    result
}

fn execute_http_request(
    url: &str,
    method: &str,
    body: Option<&str>,
    headers: &BTreeMap<String, String>,
    allowlist: &[String],
    timeout_secs: u64,
    block_private_hosts: bool,
) -> Result<ToolWorkerResponse> {
    use std::time::Instant;

    if block_private_hosts {
        reject_private_host(url)?;
    }

    if !allowlist.is_empty() {
        let host = url
            .split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .and_then(|s| s.split(':').next())
            .unwrap_or("");
        let allowed = allowlist
            .iter()
            .any(|d| host == d.as_str() || host.ends_with(&format!(".{d}")));
        if !allowed {
            return Err(Error::PreconditionFailed(format!(
                "domain '{}' not in allowlist: {:?}",
                host, allowlist
            )));
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| Error::ToolExecution(format!("http client: {e}")))?;

    let start = Instant::now();
    let mut req = match method {
        "POST" => client.post(url),
        _ => client.get(url),
    };

    if let Some(body) = body {
        req = req.body(body.to_string());
    }
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = req
        .send()
        .map_err(|e| Error::ToolExecution(format!("http request failed: {e}")))?;
    let elapsed = start.elapsed();
    let status = resp.status().as_u16();
    let body = resp.text().unwrap_or_default();
    let body_truncated = if body.len() > 10_000 {
        format!(
            "{}... (truncated, {} bytes total)",
            &body[..10_000],
            body.len()
        )
    } else {
        body
    };

    let receipt_payload = serde_json::json!({
        "worker_boundary": "subprocess_contract",
        "runtime": "thymos_secure_http",
        "method": method,
        "url_digest": blake3::hash(url.as_bytes()).to_hex().to_string(),
    });
    let mut receipt = worker_receipt("http", &receipt_payload);
    if let Some(obj) = receipt.as_object_mut() {
        obj.extend(receipt_payload.as_object().cloned().unwrap_or_default());
    }

    Ok(ToolWorkerResponse {
        kind: "http".into(),
        output: serde_json::json!({
            "status": status,
            "body": body_truncated,
            "receipt": receipt,
        }),
        latency_ms: elapsed.as_millis() as u64,
    })
}

fn validate_shell_command(
    command: &str,
    capability_profile: &str,
    blocked_patterns: &[String],
) -> Result<()> {
    const FORBIDDEN_SEQUENCES: [&str; 6] = ["&&", "||", ";", "\n", "\r", "$("];

    if command.trim().is_empty() {
        return Err(Error::ToolExecution("empty shell command".into()));
    }

    if command.len() > 2048 {
        return Err(Error::ToolExecution(
            "shell command exceeds 2048-byte hardened limit".into(),
        ));
    }

    for pattern in blocked_patterns {
        if command.contains(pattern.as_str()) {
            return Err(Error::ToolExecution(format!(
                "command blocked by sandbox policy: contains '{pattern}'"
            )));
        }
    }

    for pattern in FORBIDDEN_SEQUENCES {
        if command.contains(pattern) {
            return Err(Error::ToolExecution(format!(
                "shell command uses forbidden sequence '{pattern}'"
            )));
        }
    }

    if command.contains('`') {
        return Err(Error::ToolExecution(
            "shell command uses forbidden backtick substitution".into(),
        ));
    }

    let profile = ShellCapabilityProfile::parse(capability_profile)?;
    if let Some(allowlist) = profile.allowed_commands() {
        for stage in command.split('|') {
            let stage = stage.trim();
            if stage.is_empty() {
                return Err(Error::ToolExecution(
                    "shell pipeline contains an empty stage".into(),
                ));
            }
            let cmd_name = stage
                .split_whitespace()
                .next()
                .ok_or_else(|| Error::ToolExecution("invalid shell stage".into()))?;
            if !allowlist.contains(&cmd_name) {
                return Err(Error::ToolExecution(format!(
                    "command '{cmd_name}' is not allowed in {capability_profile} profile"
                )));
            }
        }
    }

    if profile != ShellCapabilityProfile::Networked {
        for stage in command.split('|') {
            let cmd_name = stage.split_whitespace().next().unwrap_or_default();
            if matches!(
                cmd_name,
                "curl"
                    | "wget"
                    | "ssh"
                    | "scp"
                    | "telnet"
                    | "nc"
                    | "ncat"
                    | "ping"
                    | "dig"
                    | "nslookup"
            ) {
                return Err(Error::ToolExecution(format!(
                    "network command '{cmd_name}' requires the networked profile"
                )));
            }
        }
    }

    Ok(())
}

fn resolve_working_dir(cwd: Option<&str>, allowed_roots: &[String]) -> Result<Option<PathBuf>> {
    let requested = if let Some(cwd) = cwd {
        Some(PathBuf::from(cwd))
    } else {
        allowed_roots.first().map(PathBuf::from)
    };

    let Some(requested) = requested else {
        return Ok(None);
    };

    let canonical_requested = requested.canonicalize().map_err(|e| {
        Error::ToolExecution(format!(
            "working directory '{}' is invalid: {e}",
            requested.display()
        ))
    })?;

    if allowed_roots.is_empty() {
        return Ok(Some(canonical_requested));
    }

    let mut allowed = false;
    for root in allowed_roots {
        let canonical_root = PathBuf::from(root).canonicalize().map_err(|e| {
            Error::ToolExecution(format!("allowed root '{}' is invalid: {e}", root))
        })?;
        if canonical_requested.starts_with(&canonical_root) {
            allowed = true;
            break;
        }
    }

    if !allowed {
        return Err(Error::ToolExecution(format!(
            "working directory '{}' escapes allowed roots",
            canonical_requested.display()
        )));
    }

    Ok(Some(canonical_requested))
}

fn create_isolated_home_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "thymos-home-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::ToolExecution(format!("create isolated HOME failed: {e}")))?;
    Ok(dir)
}

fn reject_private_host(url: &str) -> Result<()> {
    use std::net::IpAddr;

    let parsed = reqwest::Url::parse(url)
        .map_err(|e| Error::ToolExecution(format!("invalid url '{url}': {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::ToolExecution(format!("url '{url}' is missing a host")))?;

    if matches!(host, "localhost" | "127.0.0.1" | "::1")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return Err(Error::PreconditionFailed(format!(
            "private or loopback host '{host}' is blocked"
        )));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        let blocked = match ip {
            IpAddr::V4(v4) => {
                v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_broadcast()
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
            }
        };
        if blocked {
            return Err(Error::PreconditionFailed(format!(
                "private or loopback host '{host}' is blocked"
            )));
        }
    }

    Ok(())
}

pub trait ToolContract: Send + Sync {
    fn meta(&self) -> &ToolContractMeta;

    /// Human-readable description exposed to cognition (goes into tool prompts).
    fn description(&self) -> &str;

    /// JSON Schema describing the expected `args` shape. Used by cognition
    /// adapters (e.g. Anthropic tool_use) to constrain model output.
    fn input_schema(&self) -> Value;

    /// Schema-validate arguments. Default: permissive.
    fn validate_args(&self, _args: &Value) -> Result<()> {
        Ok(())
    }

    /// Preconditions evaluated against the world projection at parent commit.
    fn check_preconditions(&self, _inv: &ToolInvocation<'_>) -> Result<()> {
        Ok(())
    }

    /// Execute and produce a structured delta + observation.
    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome>;

    /// Estimated budget cost for this invocation. Default: 1 tool call.
    fn estimate_cost(&self, _args: &Value) -> BudgetCost {
        BudgetCost {
            tokens: 0,
            tool_calls: 1,
            wall_clock_ms: 0,
            usd_millicents: 0,
        }
    }

    /// Postconditions evaluated against the would-be next world state.
    fn check_postconditions(
        &self,
        _inv: &ToolInvocation<'_>,
        _delta: &StructuredDelta,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn ToolContract>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        ToolRegistry {
            tools: BTreeMap::new(),
        }
    }

    pub fn register<T: ToolContract + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.meta().name.clone(), Box::new(tool));
    }

    pub fn get(&self, name: &str) -> Result<&dyn ToolContract> {
        self.tools
            .get(name)
            .map(|b| b.as_ref())
            .ok_or_else(|| Error::UnknownTool(name.to_string()))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(|k| k.as_str())
    }
}

// ---- Async tool contract (behind "async" feature) -------------------------

#[cfg(feature = "async")]
mod async_tools {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;

    /// Async variant of `ToolContract`. Tools that perform I/O (HTTP, shell,
    /// MCP calls) should implement this for non-blocking execution inside a
    /// tokio runtime.
    pub trait AsyncToolContract: Send + Sync {
        fn meta(&self) -> &ToolContractMeta;
        fn description(&self) -> &str;
        fn input_schema(&self) -> Value;

        fn validate_args(&self, _args: &Value) -> Result<()> {
            Ok(())
        }

        fn check_preconditions(&self, _inv: &ToolInvocation<'_>) -> Result<()> {
            Ok(())
        }

        fn execute<'a>(
            &'a self,
            inv: ToolInvocation<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutcome>> + Send + 'a>>;

        fn estimate_cost(&self, _args: &Value) -> BudgetCost {
            BudgetCost {
                tokens: 0,
                tool_calls: 1,
                wall_clock_ms: 0,
                usd_millicents: 0,
            }
        }

        fn check_postconditions(
            &self,
            _inv: &ToolInvocation<'_>,
            _delta: &StructuredDelta,
        ) -> Result<()> {
            Ok(())
        }
    }

    /// Adapter: wrap any sync `ToolContract` as an `AsyncToolContract` by
    /// running its `execute` inside `spawn_blocking`.
    pub struct SyncAdapter<T: ToolContract + Clone + 'static>(pub T);

    impl<T: ToolContract + Clone + 'static> AsyncToolContract for SyncAdapter<T> {
        fn meta(&self) -> &ToolContractMeta {
            self.0.meta()
        }
        fn description(&self) -> &str {
            self.0.description()
        }
        fn input_schema(&self) -> Value {
            self.0.input_schema()
        }
        fn validate_args(&self, args: &Value) -> Result<()> {
            self.0.validate_args(args)
        }
        fn check_preconditions(&self, inv: &ToolInvocation<'_>) -> Result<()> {
            self.0.check_preconditions(inv)
        }
        fn estimate_cost(&self, args: &Value) -> BudgetCost {
            self.0.estimate_cost(args)
        }
        fn check_postconditions(
            &self,
            inv: &ToolInvocation<'_>,
            delta: &StructuredDelta,
        ) -> Result<()> {
            self.0.check_postconditions(inv, delta)
        }
        fn execute<'a>(
            &'a self,
            inv: ToolInvocation<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutcome>> + Send + 'a>> {
            // Clone what we need to move into the blocking task.
            let tool = self.0.clone();
            let args = inv.args.clone();
            let world = inv.world.clone();
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    let inv = ToolInvocation {
                        args: &args,
                        world: &world,
                    };
                    tool.execute(&inv)
                })
                .await
                .map_err(|e| Error::ToolExecution(format!("spawn_blocking: {e}")))?
            })
        }
    }

    /// Registry for async tools.
    #[derive(Default)]
    pub struct AsyncToolRegistry {
        tools: BTreeMap<String, Box<dyn AsyncToolContract>>,
    }

    impl AsyncToolRegistry {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn register<T: AsyncToolContract + 'static>(&mut self, tool: T) {
            self.tools.insert(tool.meta().name.clone(), Box::new(tool));
        }

        /// Wrap a sync ToolContract and register it as async via spawn_blocking.
        pub fn register_sync<T: ToolContract + Clone + 'static>(&mut self, tool: T) {
            self.register(SyncAdapter(tool));
        }

        pub fn get(&self, name: &str) -> Result<&dyn AsyncToolContract> {
            self.tools
                .get(name)
                .map(|b| b.as_ref())
                .ok_or_else(|| Error::UnknownTool(name.to_string()))
        }

        pub fn names(&self) -> impl Iterator<Item = &str> {
            self.tools.keys().map(|k| k.as_str())
        }

        /// Convert the sync ToolRegistry into an AsyncToolRegistry by wrapping
        /// each tool with SyncAdapter. Consumes self.
        pub fn from_sync(sync_reg: ToolRegistry) -> Self {
            let mut async_reg = Self::new();
            for (name, tool) in sync_reg.tools {
                // We can't clone boxed trait objects, so we store them directly
                // and use a different adapter that boxes the sync call.
                async_reg
                    .tools
                    .insert(name, Box::new(BoxedSyncAdapter(tool)));
            }
            async_reg
        }
    }

    /// Adapter for a boxed `dyn ToolContract` that can't be cloned.
    struct BoxedSyncAdapter(Box<dyn ToolContract>);

    // SAFETY: ToolContract requires Send + Sync, and we only call execute
    // from within spawn_blocking which moves it to a blocking thread.
    unsafe impl Send for BoxedSyncAdapter {}
    unsafe impl Sync for BoxedSyncAdapter {}

    impl AsyncToolContract for BoxedSyncAdapter {
        fn meta(&self) -> &ToolContractMeta {
            self.0.meta()
        }
        fn description(&self) -> &str {
            self.0.description()
        }
        fn input_schema(&self) -> Value {
            self.0.input_schema()
        }
        fn validate_args(&self, args: &Value) -> Result<()> {
            self.0.validate_args(args)
        }
        fn check_preconditions(&self, inv: &ToolInvocation<'_>) -> Result<()> {
            self.0.check_preconditions(inv)
        }
        fn estimate_cost(&self, args: &Value) -> BudgetCost {
            self.0.estimate_cost(args)
        }
        fn check_postconditions(
            &self,
            inv: &ToolInvocation<'_>,
            delta: &StructuredDelta,
        ) -> Result<()> {
            self.0.check_postconditions(inv, delta)
        }
        fn execute<'a>(
            &'a self,
            inv: ToolInvocation<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutcome>> + Send + 'a>> {
            let args = inv.args.clone();
            let world = inv.world.clone();
            Box::pin(async move {
                // We need to call self.0.execute from a blocking context.
                // Since we can't move self into the closure, we use a scoped approach.
                // The tool is Send+Sync so we can reference it from the blocking thread.
                let tool_ref = &self.0;
                let inv = ToolInvocation {
                    args: &args,
                    world: &world,
                };
                // Execute synchronously — the caller (async runtime) should wrap
                // in spawn_blocking if needed. For now, run inline since the tool
                // is already Send+Sync.
                tool_ref.execute(&inv)
            })
        }
    }
}

#[cfg(feature = "async")]
pub use async_tools::{AsyncToolContract, AsyncToolRegistry, SyncAdapter};

// ---- Stock tool: kv_set ------------------------------------------------

/// A trivial local-state tool: `kv_set(key: string, value: json)` ->
/// creates or replaces a `kv:{key}` resource. Write/low-risk.
pub struct KvSetTool {
    meta: ToolContractMeta,
}

impl Default for KvSetTool {
    fn default() -> Self {
        KvSetTool {
            meta: ToolContractMeta {
                name: "kv_set".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Write,
                risk_class: RiskClass::Low,
            },
        }
    }
}

impl ToolContract for KvSetTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Set a value on the in-memory key-value store. Creates the resource if it does not exist, otherwise replaces it under optimistic concurrency. Use this to write simple key/value state."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key":   { "type": "string", "description": "The key to set" },
                "value": { "description": "The value to associate with the key (any JSON)" }
            },
            "required": ["key", "value"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        let obj = args.as_object().ok_or_else(|| Error::ToolTypeMismatch {
            tool: "kv_set".into(),
            detail: "args must be an object".into(),
        })?;
        let _key =
            obj.get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::ToolTypeMismatch {
                    tool: "kv_set".into(),
                    detail: "missing string 'key'".into(),
                })?;
        if !obj.contains_key("value") {
            return Err(Error::ToolTypeMismatch {
                tool: "kv_set".into(),
                detail: "missing 'value'".into(),
            });
        }
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        use thymos_core::delta::DeltaOp;
        use thymos_core::world::ResourceKey;

        let key = inv.args["key"].as_str().unwrap().to_string();
        let value = inv.args["value"].clone();

        let delta_op = match inv.world.get(&ResourceKey::new("kv", &key)) {
            None => DeltaOp::Create {
                kind: "kv".into(),
                id: key.clone(),
                value: value.clone(),
            },
            Some(state) => DeltaOp::Replace {
                kind: "kv".into(),
                id: key.clone(),
                expected_version: state.version,
                value: value.clone(),
            },
        };

        Ok(ToolOutcome {
            delta: StructuredDelta::single(delta_op),
            observation: Observation {
                tool: "kv_set".into(),
                output: serde_json::json!({ "key": key, "wrote": value }),
                latency_ms: 0,
            },
        })
    }
}

// ---- Stock tool: kv_get ------------------------------------------------

/// Read-only tool: `kv_get(key: string)` -> returns the current value.
/// Pure/Read/Low. Produces an empty delta.
pub struct KvGetTool {
    meta: ToolContractMeta,
}

impl Default for KvGetTool {
    fn default() -> Self {
        KvGetTool {
            meta: ToolContractMeta {
                name: "kv_get".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Read,
                risk_class: RiskClass::Low,
            },
        }
    }
}

impl ToolContract for KvGetTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Read a value from the in-memory key-value store. Returns null if the key is not set. Use this to observe state before acting."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "The key to read" }
            },
            "required": ["key"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        args.as_object()
            .and_then(|o| o.get("key").and_then(|v| v.as_str()))
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "kv_get".into(),
                detail: "missing string 'key'".into(),
            })?;
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        use thymos_core::world::ResourceKey;

        let key = inv.args["key"].as_str().unwrap().to_string();
        let out = inv
            .world
            .get(&ResourceKey::new("kv", &key))
            .map(|s| s.value.clone())
            .unwrap_or(Value::Null);

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "kv_get".into(),
                output: serde_json::json!({ "key": key, "value": out }),
                latency_ms: 0,
            },
        })
    }
}

// ---- Memory tools ---------------------------------------------------------

/// `memory_store(key, content, source_commits)` — promote information into
/// long-term memory. Stores a `memory:{key}` resource whose value includes
/// `content` plus `source_commits` provenance. Write/Low.
pub struct MemoryStoreTool {
    meta: ToolContractMeta,
}

impl Default for MemoryStoreTool {
    fn default() -> Self {
        MemoryStoreTool {
            meta: ToolContractMeta {
                name: "memory_store".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Write,
                risk_class: RiskClass::Low,
            },
        }
    }
}

impl ToolContract for MemoryStoreTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Promote a distilled fact into long-term memory. Requires a key, \
         content string, and an array of source_commits (commit IDs that this \
         memory derives from) for provenance. Creates or replaces a \
         memory:{stratum}:{key} resource. `stratum` is one of 'working' \
         (ephemeral, current-task scratch), 'episodic' (per-trajectory \
         experiences), or 'semantic' (default, distilled long-term knowledge)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key":            { "type": "string", "description": "Memory key (unique identifier)" },
                "content":        { "type": "string", "description": "Distilled fact or knowledge" },
                "stratum":        {
                    "type": "string",
                    "enum": ["working", "episodic", "semantic"],
                    "description": "Memory stratum (default: semantic)"
                },
                "source_commits": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Commit IDs this memory derives from (provenance chain)"
                }
            },
            "required": ["key", "content"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        let obj = args.as_object().ok_or_else(|| Error::ToolTypeMismatch {
            tool: "memory_store".into(),
            detail: "args must be an object".into(),
        })?;
        obj.get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "memory_store".into(),
                detail: "missing string 'key'".into(),
            })?;
        obj.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "memory_store".into(),
                detail: "missing string 'content'".into(),
            })?;
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        use thymos_core::delta::DeltaOp;
        use thymos_core::world::ResourceKey;

        let key = inv.args["key"].as_str().unwrap().to_string();
        let content = inv.args["content"].as_str().unwrap().to_string();
        let stratum = inv
            .args
            .get("stratum")
            .and_then(|v| v.as_str())
            .unwrap_or("semantic")
            .to_string();
        let source_commits = inv
            .args
            .get("source_commits")
            .cloned()
            .unwrap_or(serde_json::json!([]));

        let resource_id = format!("{stratum}:{key}");
        let value = serde_json::json!({
            "content": content,
            "stratum": stratum,
            "source_commits": source_commits,
        });

        let delta_op = match inv.world.get(&ResourceKey::new("memory", &resource_id)) {
            None => DeltaOp::Create {
                kind: "memory".into(),
                id: resource_id.clone(),
                value: value.clone(),
            },
            Some(state) => DeltaOp::Replace {
                kind: "memory".into(),
                id: resource_id.clone(),
                expected_version: state.version,
                value: value.clone(),
            },
        };

        Ok(ToolOutcome {
            delta: StructuredDelta::single(delta_op),
            observation: Observation {
                tool: "memory_store".into(),
                output: serde_json::json!({
                    "key": key,
                    "stratum": stratum,
                    "stored": true,
                }),
                latency_ms: 0,
            },
        })
    }
}

/// `memory_recall(key)` — read a memory resource. Read/Low.
pub struct MemoryRecallTool {
    meta: ToolContractMeta,
}

impl Default for MemoryRecallTool {
    fn default() -> Self {
        MemoryRecallTool {
            meta: ToolContractMeta {
                name: "memory_recall".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Read,
                risk_class: RiskClass::Low,
            },
        }
    }
}

impl ToolContract for MemoryRecallTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Recall a previously stored memory by key. Returns the content and \
         its provenance (source_commits). `stratum` defaults to 'semantic'. \
         Returns null if the key is not found in that stratum."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key":     { "type": "string", "description": "Memory key to recall" },
                "stratum": {
                    "type": "string",
                    "enum": ["working", "episodic", "semantic"],
                    "description": "Memory stratum (default: semantic)"
                }
            },
            "required": ["key"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        args.as_object()
            .and_then(|o| o.get("key").and_then(|v| v.as_str()))
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "memory_recall".into(),
                detail: "missing string 'key'".into(),
            })?;
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        use thymos_core::world::ResourceKey;

        let key = inv.args["key"].as_str().unwrap().to_string();
        let stratum = inv
            .args
            .get("stratum")
            .and_then(|v| v.as_str())
            .unwrap_or("semantic")
            .to_string();
        let resource_id = format!("{stratum}:{key}");
        let out = inv
            .world
            .get(&ResourceKey::new("memory", &resource_id))
            .map(|s| s.value.clone())
            .unwrap_or(Value::Null);

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "memory_recall".into(),
                output: serde_json::json!({
                    "key": key,
                    "stratum": stratum,
                    "memory": out,
                }),
                latency_ms: 0,
            },
        })
    }
}

// ---- Delegation tool (synthetic) ------------------------------------------

/// Synthetic tool for delegation. The compiler uses it for resolution and
/// validation; the runtime intercepts proposals whose plan.tool == "delegate"
/// and spawns a child trajectory instead of calling `execute`.
pub struct DelegateTool {
    meta: ToolContractMeta,
}

impl Default for DelegateTool {
    fn default() -> Self {
        DelegateTool {
            meta: ToolContractMeta {
                name: "delegate".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Write,
                risk_class: RiskClass::Medium,
            },
        }
    }
}

impl ToolContract for DelegateTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Delegate a sub-task to a child agent running under a narrower capability \
         writ. The runtime spawns a child trajectory, runs it to completion, and \
         returns the child's final answer as the observation. Args: task (string), \
         tool_scopes (array of tool patterns for the child writ)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task":        { "type": "string", "description": "The sub-task description" },
                "tool_scopes": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tool patterns the child agent is authorized to use (must be subset of parent writ)"
                }
            },
            "required": ["task"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        args.as_object()
            .and_then(|o| o.get("task").and_then(|v| v.as_str()))
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "delegate".into(),
                detail: "missing string 'task'".into(),
            })?;
        Ok(())
    }

    /// The runtime intercepts delegation before `execute` is called. If this
    /// is reached, something is wrong.
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        Err(Error::Other(
            "delegate tool should be intercepted by the runtime, not executed directly".into(),
        ))
    }
}

// ---- Sandbox: shell tool --------------------------------------------------

/// Sandbox configuration for process-level isolation.
#[derive(Clone, Debug)]
pub struct SandboxConfig {
    /// Working directory for the subprocess. If None, inherits the parent's.
    pub working_dir: Option<String>,
    /// If true, the subprocess gets a minimal environment (only PATH and HOME).
    pub restricted_env: bool,
    /// Additional environment variables to set (merged on top of restricted env).
    pub env_overrides: BTreeMap<String, String>,
    /// Maximum output bytes captured from stdout+stderr. Prevents OOM.
    pub max_output_bytes: usize,
    /// Command prefix/wrapper (e.g. ["firejail", "--noprofile"] or ["docker", "run", "--rm", "sandbox"]).
    /// If non-empty, the command is wrapped: `wrapper... /bin/sh -c <command>`.
    pub wrapper: Vec<String>,
    /// Blocked command patterns (substring match). Prevents dangerous commands.
    pub blocked_patterns: Vec<String>,
    /// Optional worker binary used for subprocess isolation.
    pub worker_bin: Option<String>,
    /// Allowed directory roots for command execution.
    pub allowed_roots: Vec<String>,
    /// If true, restricted environments receive an isolated temporary HOME.
    pub isolate_home: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        SandboxConfig {
            working_dir: None,
            restricted_env: false,
            env_overrides: BTreeMap::new(),
            max_output_bytes: 1_048_576, // 1 MB
            wrapper: Vec::new(),
            blocked_patterns: vec![
                "rm -rf /".into(),
                "mkfs".into(),
                "dd if=".into(),
                ":(){".into(), // fork bomb
            ],
            worker_bin: std::env::var("THYMOS_WORKER_BIN").ok(),
            allowed_roots: std::env::current_dir()
                .ok()
                .map(|p| vec![p.display().to_string()])
                .unwrap_or_default(),
            isolate_home: true,
        }
    }
}

/// Execute a shell command as a subprocess with a timeout. Captures stdout and
/// stderr. Effect class: External, Risk: High. The Writ must explicitly
/// authorize `shell`.
pub struct ShellTool {
    meta: ToolContractMeta,
    /// Maximum seconds a command may run before being killed.
    pub timeout_secs: u64,
    /// Sandbox configuration for process isolation.
    pub sandbox: SandboxConfig,
    /// Execution mode for the secure tool fabric.
    pub execution_mode: ToolExecutionMode,
}

impl Default for ShellTool {
    fn default() -> Self {
        ShellTool {
            meta: ToolContractMeta {
                name: "shell".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::External,
                risk_class: RiskClass::High,
            },
            timeout_secs: 30,
            sandbox: SandboxConfig::default(),
            execution_mode: match std::env::var("THYMOS_TOOL_FABRIC").ok().as_deref() {
                Some("worker") => ToolExecutionMode::Worker,
                _ => ToolExecutionMode::InProcess,
            },
        }
    }
}

impl ShellTool {
    pub fn with_sandbox(mut self, sandbox: SandboxConfig) -> Self {
        self.sandbox = sandbox;
        self
    }

    pub fn with_execution_mode(mut self, mode: ToolExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }
}

impl ToolContract for ShellTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Execute a command through the THYMOS secure shell. This shell is \
         receipt-bearing and capability-aware: provide a clear purpose, keep \
         commands bounded, and prefer inspect/build/mutate style work over \
         arbitrary interactive scripting. Returns stdout, stderr, exit code, \
         and an execution receipt."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "purpose": {
                    "type": "string",
                    "description": "Why this command is needed for the current run"
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory override for this execution"
                },
                "capability_profile": {
                    "type": "string",
                    "enum": ["inspect", "build", "mutate", "networked"],
                    "default": "inspect",
                    "description": "Declared execution profile used in the receipt"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Override timeout in seconds (default: 30)"
                }
            },
            "required": ["command"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        args.as_object()
            .and_then(|o| o.get("command").and_then(|v| v.as_str()))
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "shell".into(),
                detail: "missing string 'command'".into(),
            })?;
        Ok(())
    }

    fn estimate_cost(&self, _args: &Value) -> BudgetCost {
        BudgetCost {
            tokens: 0,
            tool_calls: 1,
            wall_clock_ms: self.timeout_secs * 1000,
            usd_millicents: 0,
        }
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let command = inv.args["command"].as_str().unwrap();
        let timeout = inv
            .args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.timeout_secs);

        let capability_profile = inv
            .args
            .get("capability_profile")
            .and_then(|v| v.as_str())
            .unwrap_or("inspect");
        let purpose = inv.args.get("purpose").and_then(|v| v.as_str());
        let cwd = inv
            .args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.sandbox.working_dir.clone());

        let request = ToolWorkerRequest::Shell {
            command: command.to_string(),
            cwd,
            timeout_secs: timeout,
            purpose: purpose.map(|s| s.to_string()),
            capability_profile: capability_profile.to_string(),
            restricted_env: self.sandbox.restricted_env,
            env: self.sandbox.env_overrides.clone(),
            max_output_bytes: self.sandbox.max_output_bytes,
            blocked_patterns: self.sandbox.blocked_patterns.clone(),
            wrapper: self.sandbox.wrapper.clone(),
            allowed_roots: self.sandbox.allowed_roots.clone(),
            isolate_home: self.sandbox.isolate_home,
        };

        let response = match self.execution_mode {
            ToolExecutionMode::InProcess => in_process_worker_execute(request)?,
            ToolExecutionMode::Worker => {
                let worker_bin = self.sandbox.worker_bin.as_deref().ok_or_else(|| {
                    Error::ToolExecution(
                        "worker execution mode requires THYMOS_WORKER_BIN or sandbox.worker_bin"
                            .into(),
                    )
                })?;
                subprocess_worker_execute(worker_bin, &request)?
            }
        };

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "shell".into(),
                output: response.output,
                latency_ms: response.latency_ms,
            },
        })
    }
}

// ---- Sandbox: HTTP tool ---------------------------------------------------

/// Make an HTTP request to an external URL. Supports GET and POST. An optional
/// allowlist restricts which domains may be contacted. Effect class: External,
/// Risk: Medium.
pub struct HttpTool {
    meta: ToolContractMeta,
    /// If non-empty, only these domain suffixes are allowed.
    pub domain_allowlist: Vec<String>,
    /// Execution mode for the secure tool fabric.
    pub execution_mode: ToolExecutionMode,
    /// Timeout for outbound requests.
    pub timeout_secs: u64,
    /// If true, block loopback/private/internal targets before request dispatch.
    pub block_private_hosts: bool,
}

impl Default for HttpTool {
    fn default() -> Self {
        HttpTool {
            meta: ToolContractMeta {
                name: "http".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::External,
                risk_class: RiskClass::Medium,
            },
            domain_allowlist: vec![],
            execution_mode: match std::env::var("THYMOS_TOOL_FABRIC").ok().as_deref() {
                Some("worker") => ToolExecutionMode::Worker,
                _ => ToolExecutionMode::InProcess,
            },
            timeout_secs: 30,
            block_private_hosts: true,
        }
    }
}

impl HttpTool {
    pub fn with_allowlist(mut self, domains: Vec<String>) -> Self {
        self.domain_allowlist = domains;
        self
    }

    pub fn with_execution_mode(mut self, mode: ToolExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }
}

impl ToolContract for HttpTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Make an HTTP request through the THYMOS secure tool fabric. Supports \
         GET and POST, returns response data plus a worker receipt, and can be \
         pinned to an allowlist for controlled network access."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url":    { "type": "string", "description": "Full URL to request" },
                "method": { "type": "string", "enum": ["GET", "POST"], "default": "GET" },
                "body":   { "type": "string", "description": "Request body (for POST)" },
                "headers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Optional request headers"
                }
            },
            "required": ["url"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        args.as_object()
            .and_then(|o| o.get("url").and_then(|v| v.as_str()))
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "http".into(),
                detail: "missing string 'url'".into(),
            })?;
        Ok(())
    }

    fn check_preconditions(&self, inv: &ToolInvocation<'_>) -> Result<()> {
        if self.domain_allowlist.is_empty() {
            return Ok(());
        }
        let url = inv.args["url"].as_str().unwrap();
        let host = url
            .split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .and_then(|s| s.split(':').next())
            .unwrap_or("");
        let allowed = self
            .domain_allowlist
            .iter()
            .any(|d| host == d.as_str() || host.ends_with(&format!(".{d}")));
        if !allowed {
            return Err(Error::PreconditionFailed(format!(
                "domain '{}' not in allowlist: {:?}",
                host, self.domain_allowlist
            )));
        }
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let url = inv.args["url"].as_str().unwrap();
        let method = inv
            .args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET");
        let body = inv
            .args
            .get("body")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let headers = inv
            .args
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|headers| {
                headers
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|v_str| (k.clone(), v_str.to_string())))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let request = ToolWorkerRequest::Http {
            url: url.to_string(),
            method: method.to_string(),
            body,
            headers,
            allowlist: self.domain_allowlist.clone(),
            timeout_secs: self.timeout_secs,
            block_private_hosts: self.block_private_hosts,
        };
        let response = match self.execution_mode {
            ToolExecutionMode::InProcess => in_process_worker_execute(request)?,
            ToolExecutionMode::Worker => {
                let worker_bin = std::env::var("THYMOS_WORKER_BIN").map_err(|_| {
                    Error::ToolExecution("worker execution mode requires THYMOS_WORKER_BIN".into())
                })?;
                subprocess_worker_execute(&worker_bin, &request)?
            }
        };

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "http".into(),
                output: response.output,
                latency_ms: response.latency_ms,
            },
        })
    }
}

// ---- Manifest-driven tool -------------------------------------------------

/// A tool manifest loaded from a JSON file on disk. The manifest declares
/// metadata, input schema, effect/risk class, and an executor kind. This lets
/// operators add tools without writing Rust — they drop a `.json` file and
/// the registry picks it up.
///
/// Supported executor kinds:
///   - `"shell"`: runs a command template with `{arg_name}` interpolation
///   - `"http"`:  hits a URL template with JSON body forwarding
///   - `"noop"`:  pure schema-only tool (useful for testing / dry-run)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub effect_class: EffectClass,
    pub risk_class: RiskClass,
    pub input_schema: Value,
    #[serde(default)]
    pub executor: ManifestExecutor,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestExecutor {
    /// Run a shell command template. `{arg}` placeholders are replaced with
    /// the corresponding arg values (shell-escaped).
    Shell { command_template: String },
    /// Hit an HTTP endpoint. `{arg}` placeholders in url/body are replaced.
    Http {
        url_template: String,
        #[serde(default = "default_get")]
        method: String,
        #[serde(default)]
        body_template: Option<String>,
    },
    /// No-op executor — tool resolves and validates but produces an empty
    /// delta with a canned observation. Useful for dry-run / schema testing.
    #[default]
    Noop,
}

fn default_get() -> String {
    "GET".into()
}

/// A ToolContract implementation backed by a JSON manifest.
pub struct ManifestTool {
    meta: ToolContractMeta,
    manifest: ToolManifest,
}

impl ManifestTool {
    /// Load a tool manifest from a JSON file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| Error::Other(format!("reading manifest {}: {e}", path.display())))?;
        let manifest: ToolManifest = serde_json::from_str(&contents)
            .map_err(|e| Error::Other(format!("parsing manifest {}: {e}", path.display())))?;
        Ok(Self::from_manifest(manifest))
    }

    /// Create from an already-parsed manifest.
    pub fn from_manifest(manifest: ToolManifest) -> Self {
        let meta = ToolContractMeta {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            effect_class: manifest.effect_class,
            risk_class: manifest.risk_class,
        };
        ManifestTool { meta, manifest }
    }
}

/// Shell-escape a string value for safe interpolation into command templates.
fn shell_escape(s: &str) -> String {
    // Single-quote the value, escaping any existing single quotes.
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Replace `{arg_name}` placeholders in a template with values from args.
fn interpolate(template: &str, args: &Value, escape_fn: fn(&str) -> String) -> String {
    let mut result = template.to_string();
    if let Some(obj) = args.as_object() {
        for (k, v) in obj {
            let placeholder = format!("{{{k}}}");
            let replacement = match v {
                Value::String(s) => escape_fn(s),
                other => escape_fn(&other.to_string()),
            };
            result = result.replace(&placeholder, &replacement);
        }
    }
    result
}

/// Identity escape (no transformation) for HTTP templates.
fn identity_escape(s: &str) -> String {
    s.to_string()
}

impl ToolContract for ManifestTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn input_schema(&self) -> Value {
        self.manifest.input_schema.clone()
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        match &self.manifest.executor {
            ManifestExecutor::Noop => Ok(ToolOutcome {
                delta: StructuredDelta::default(),
                observation: Observation {
                    tool: self.meta.name.clone(),
                    output: serde_json::json!({ "result": "noop" }),
                    latency_ms: 0,
                },
            }),

            ManifestExecutor::Shell { command_template } => {
                use std::process::Command;
                use std::time::Instant;

                let cmd = interpolate(command_template, inv.args, shell_escape);
                let start = Instant::now();
                let output = Command::new("/bin/sh")
                    .arg("-c")
                    .arg(&cmd)
                    .output()
                    .map_err(|e| Error::ToolExecution(format!("spawn failed: {e}")))?;
                let elapsed = start.elapsed();

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                Ok(ToolOutcome {
                    delta: StructuredDelta::default(),
                    observation: Observation {
                        tool: self.meta.name.clone(),
                        output: serde_json::json!({
                            "stdout": stdout,
                            "stderr": stderr,
                            "exit_code": exit_code,
                        }),
                        latency_ms: elapsed.as_millis() as u64,
                    },
                })
            }

            ManifestExecutor::Http {
                url_template,
                method,
                body_template,
            } => {
                use std::time::Instant;

                let url = interpolate(url_template, inv.args, identity_escape);
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .map_err(|e| Error::ToolExecution(format!("http client: {e}")))?;

                let start = Instant::now();
                let mut req = match method.as_str() {
                    "POST" => client.post(&url),
                    "PUT" => client.put(&url),
                    "DELETE" => client.delete(&url),
                    _ => client.get(&url),
                };

                if let Some(bt) = body_template {
                    let body = interpolate(bt, inv.args, identity_escape);
                    req = req.header("content-type", "application/json").body(body);
                }

                let resp = req
                    .send()
                    .map_err(|e| Error::ToolExecution(format!("http: {e}")))?;
                let elapsed = start.elapsed();
                let status = resp.status().as_u16();
                let body = resp.text().unwrap_or_default();

                let body_truncated = if body.len() > 10_000 {
                    format!("{}... (truncated, {} bytes)", &body[..10_000], body.len())
                } else {
                    body
                };

                Ok(ToolOutcome {
                    delta: StructuredDelta::default(),
                    observation: Observation {
                        tool: self.meta.name.clone(),
                        output: serde_json::json!({
                            "status": status,
                            "body": body_truncated,
                        }),
                        latency_ms: elapsed.as_millis() as u64,
                    },
                })
            }
        }
    }
}

// ---- MCP bridge tool ------------------------------------------------------

/// Bridge to an MCP (Model Context Protocol) server running as a subprocess.
/// Communicates via JSON-RPC 2.0 over stdin/stdout.
///
/// Usage:
///   1. Create with `McpBridgeTool::spawn("server-name", &["uvx", "my-mcp-server"])`
///   2. This discovers tools via `tools/list` and exposes the first (or named) tool
///   3. On `execute`, it sends `tools/call` and returns the result as an observation
///
/// For multi-tool MCP servers, use `McpBridge::spawn_all()` to get one
/// `McpBridgeTool` per discovered tool.
pub struct McpBridgeTool {
    meta: ToolContractMeta,
    description_text: String,
    schema: Value,
    /// The MCP tool name on the server side.
    mcp_tool_name: String,
    /// Shared handle to the MCP subprocess.
    bridge: Arc<McpBridge>,
}

/// Shared handle to an MCP server subprocess.
pub struct McpBridge {
    child_stdin: Mutex<std::process::ChildStdin>,
    child_stdout: Mutex<std::io::BufReader<std::process::ChildStdout>>,
    next_id: Mutex<u64>,
    server_name: String,
}

use std::io::BufRead;
use std::sync::{Arc, Mutex};

/// A single tool descriptor returned from `tools/list`.
#[derive(Debug, Clone, Deserialize)]
struct McpToolInfo {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    input_schema: Value,
}

impl McpBridge {
    /// Spawn an MCP server subprocess and perform the `initialize` handshake.
    pub fn spawn(server_name: &str, command: &[&str]) -> Result<Arc<Self>> {
        if command.is_empty() {
            return Err(Error::Other("MCP bridge: empty command".into()));
        }

        let mut child = std::process::Command::new(command[0])
            .args(&command[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| Error::ToolExecution(format!("MCP spawn {server_name}: {e}")))?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let bridge = Arc::new(McpBridge {
            child_stdin: Mutex::new(stdin),
            child_stdout: Mutex::new(std::io::BufReader::new(stdout)),
            next_id: Mutex::new(1),
            server_name: server_name.to_string(),
        });

        // Send initialize request.
        let _init_resp = bridge.call_method(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "thymos", "version": "0.0.1" }
            }),
        )?;

        // Send initialized notification (no response expected — but some
        // servers ignore it and some require it).
        bridge.send_notification("notifications/initialized", serde_json::json!({}))?;

        Ok(bridge)
    }

    /// Discover all tools from the MCP server and return one McpBridgeTool per tool.
    pub fn spawn_all(server_name: &str, command: &[&str]) -> Result<Vec<McpBridgeTool>> {
        let bridge = Self::spawn(server_name, command)?;
        let tools_resp = bridge.call_method("tools/list", serde_json::json!({}))?;

        let tools_array = tools_resp
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let mut result = Vec::new();
        for tool_val in tools_array {
            let info: McpToolInfo = serde_json::from_value(tool_val)
                .map_err(|e| Error::Other(format!("parsing MCP tool info: {e}")))?;

            let tool_name = format!("mcp_{}_{}", server_name, info.name);
            let description = info
                .description
                .unwrap_or_else(|| format!("MCP tool {} from {}", info.name, server_name));

            result.push(McpBridgeTool {
                meta: ToolContractMeta {
                    name: tool_name,
                    version: "0.0.1".into(),
                    effect_class: EffectClass::External,
                    risk_class: RiskClass::Medium,
                },
                description_text: description,
                schema: if info.input_schema.is_null() {
                    serde_json::json!({ "type": "object" })
                } else {
                    info.input_schema
                },
                mcp_tool_name: info.name,
                bridge: bridge.clone(),
            });
        }

        Ok(result)
    }

    fn next_request_id(&self) -> u64 {
        let mut id = self.next_id.lock().unwrap();
        let current = *id;
        *id += 1;
        current
    }

    fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let serialized = serde_json::to_string(&msg).unwrap();
        let mut stdin = self.child_stdin.lock().unwrap();
        writeln!(stdin, "{serialized}")
            .map_err(|e| Error::ToolExecution(format!("MCP write {}: {e}", self.server_name)))?;
        stdin
            .flush()
            .map_err(|e| Error::ToolExecution(format!("MCP flush {}: {e}", self.server_name)))?;
        Ok(())
    }

    fn call_method(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_request_id();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let serialized = serde_json::to_string(&msg).unwrap();

        {
            let mut stdin = self.child_stdin.lock().unwrap();
            writeln!(stdin, "{serialized}").map_err(|e| {
                Error::ToolExecution(format!("MCP write {}: {e}", self.server_name))
            })?;
            stdin.flush().map_err(|e| {
                Error::ToolExecution(format!("MCP flush {}: {e}", self.server_name))
            })?;
        }

        // Read response lines until we get one with a matching id.
        let mut stdout = self.child_stdout.lock().unwrap();
        loop {
            let mut line = String::new();
            let bytes = stdout
                .read_line(&mut line)
                .map_err(|e| Error::ToolExecution(format!("MCP read {}: {e}", self.server_name)))?;
            if bytes == 0 {
                return Err(Error::ToolExecution(format!(
                    "MCP server {} closed stdout",
                    self.server_name
                )));
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let resp: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // skip non-JSON lines (stderr leaks, etc.)
            };

            // Skip notifications (no "id" field).
            if resp.get("id").is_none() {
                continue;
            }

            // Check for matching id.
            if resp.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = resp.get("error") {
                    return Err(Error::ToolExecution(format!(
                        "MCP error from {}: {}",
                        self.server_name, err
                    )));
                }
                return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
            }
            // Not our response — skip and keep reading.
        }
    }
}

impl ToolContract for McpBridgeTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        &self.description_text
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        use std::time::Instant;

        let start = Instant::now();
        let result = self.bridge.call_method(
            "tools/call",
            serde_json::json!({
                "name": self.mcp_tool_name,
                "arguments": inv.args,
            }),
        )?;
        let elapsed = start.elapsed();

        // MCP tools/call returns { content: [...] }. Flatten text content.
        let content = result
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|| result.to_string());

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: self.meta.name.clone(),
                output: serde_json::json!({ "result": content }),
                latency_ms: elapsed.as_millis() as u64,
            },
        })
    }
}

impl ToolRegistry {
    /// Load a single tool manifest from a JSON file and register it.
    pub fn load_manifest(&mut self, path: &Path) -> Result<()> {
        let tool = ManifestTool::from_file(path)?;
        self.tools.insert(tool.meta().name.clone(), Box::new(tool));
        Ok(())
    }

    /// Scan a directory for `*.json` manifest files and register each one.
    /// Skips files that fail to parse (logs to stderr).
    pub fn load_manifest_dir(&mut self, dir: &Path) -> Result<usize> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| Error::Other(format!("reading manifest dir {}: {e}", dir.display())))?;

        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                match self.load_manifest(&path) {
                    Ok(()) => count += 1,
                    Err(e) => eprintln!("warning: skipping manifest {}: {e}", path.display()),
                }
            }
        }
        Ok(count)
    }

    /// Spawn an MCP server and register all tools it exposes.
    /// Tool names are prefixed: `mcp_{server_name}_{tool_name}`.
    pub fn register_mcp_server(&mut self, server_name: &str, command: &[&str]) -> Result<usize> {
        let tools = McpBridge::spawn_all(server_name, command)?;
        let count = tools.len();
        for tool in tools {
            self.tools.insert(tool.meta().name.clone(), Box::new(tool));
        }
        Ok(count)
    }
}

#[cfg(test)]
mod secure_tool_fabric_tests {
    use super::*;

    #[test]
    fn shell_rejects_forbidden_sequence() {
        let err = validate_shell_command("ls && pwd", "inspect", &[]).unwrap_err();
        assert!(err.to_string().contains("forbidden sequence"));
    }

    #[test]
    fn shell_rejects_command_outside_profile() {
        let err = validate_shell_command("curl https://example.com", "inspect", &[]).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn resolve_working_dir_rejects_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let allowed = vec![temp.path().display().to_string()];
        let err = resolve_working_dir(Some("/tmp"), &allowed).unwrap_err();
        assert!(err.to_string().contains("escapes allowed roots"));
    }

    #[test]
    fn http_rejects_private_host() {
        let err = execute_http_request(
            "http://127.0.0.1:3001",
            "GET",
            None,
            &BTreeMap::new(),
            &[],
            5,
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("private or loopback host"));
    }
}
