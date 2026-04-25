//! Coding-agent tool surface.
//!
//! Typed `ToolContract` implementations for the loop a coding agent runs:
//! read → patch → list → map → grep → test. Every tool is path-confined to a
//! configured set of allowed roots. The runtime is the authority — the model
//! proposes paths and content, the tool validates and either executes inside
//! the sandbox or returns an error that becomes a rejection in the ledger.
//!
//! All tools are read-only against the `World` projection (they treat the
//! filesystem itself as the substrate, not the world). They emit observations
//! summarising what they did so the cognition loop has structured feedback.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::Value;

use thymos_core::{
    commit::Observation,
    delta::StructuredDelta,
    error::{Error, Result},
};

use crate::{EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome};

#[derive(Clone, Debug)]
pub struct CodingSandbox {
    pub allowed_roots: Vec<String>,
    pub max_read_bytes: usize,
    pub max_grep_matches: usize,
    pub max_list_entries: usize,
}

impl Default for CodingSandbox {
    fn default() -> Self {
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        CodingSandbox {
            allowed_roots: if cwd.is_empty() { vec![] } else { vec![cwd] },
            max_read_bytes: 256 * 1024,
            max_grep_matches: 256,
            max_list_entries: 512,
        }
    }
}

impl CodingSandbox {
    pub fn confine(&self, path: &str) -> Result<PathBuf> {
        let requested = PathBuf::from(path);
        let absolute = if requested.is_absolute() {
            requested
        } else if let Some(root) = self.allowed_roots.first() {
            PathBuf::from(root).join(requested)
        } else {
            std::env::current_dir()
                .map_err(|e| Error::ToolExecution(format!("no cwd: {e}")))?
                .join(requested)
        };

        // Canonicalise against the parent if the file does not yet exist
        // (we still need to create files).
        let canonical = match absolute.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                let parent = absolute.parent().ok_or_else(|| {
                    Error::ToolExecution(format!("path '{}' has no parent", absolute.display()))
                })?;
                let canonical_parent = parent.canonicalize().map_err(|e| {
                    Error::ToolExecution(format!("parent '{}' is invalid: {e}", parent.display()))
                })?;
                let name = absolute.file_name().ok_or_else(|| {
                    Error::ToolExecution(format!("path '{}' has no file name", absolute.display()))
                })?;
                canonical_parent.join(name)
            }
        };

        if self.allowed_roots.is_empty() {
            return Ok(canonical);
        }

        for root in &self.allowed_roots {
            let canonical_root = PathBuf::from(root).canonicalize().map_err(|e| {
                Error::ToolExecution(format!("allowed root '{root}' is invalid: {e}"))
            })?;
            if canonical.starts_with(&canonical_root) {
                return Ok(canonical);
            }
        }

        Err(Error::ToolExecution(format!(
            "path '{}' escapes allowed roots",
            canonical.display()
        )))
    }

    pub fn primary_root(&self) -> PathBuf {
        self.allowed_roots
            .first()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

// ---- fs_read --------------------------------------------------------------

pub struct FsReadTool {
    meta: ToolContractMeta,
    pub sandbox: CodingSandbox,
}

impl Default for FsReadTool {
    fn default() -> Self {
        FsReadTool {
            meta: ToolContractMeta {
                name: "fs_read".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Read,
                risk_class: RiskClass::Low,
            },
            sandbox: CodingSandbox::default(),
        }
    }
}

impl ToolContract for FsReadTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Read a file from the working repository. Returns the content along \
         with line count. Optional `start` and `end` (1-indexed, inclusive) \
         restrict to a line range. Reads above the configured byte cap fail \
         loudly — slice the file or grep first."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path":  { "type": "string", "description": "File path relative to the repo root" },
                "start": { "type": "integer", "minimum": 1, "description": "First line to return (inclusive)" },
                "end":   { "type": "integer", "minimum": 1, "description": "Last line to return (inclusive)" }
            },
            "required": ["path"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        args.as_object()
            .and_then(|o| o.get("path").and_then(|v| v.as_str()))
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "fs_read".into(),
                detail: "missing string 'path'".into(),
            })?;
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let started = Instant::now();
        let path = inv.args["path"].as_str().unwrap();
        let start = inv.args.get("start").and_then(|v| v.as_u64());
        let end = inv.args.get("end").and_then(|v| v.as_u64());

        let resolved = self.sandbox.confine(path)?;
        let metadata = std::fs::metadata(&resolved)
            .map_err(|e| Error::ToolExecution(format!("stat {}: {e}", resolved.display())))?;
        if metadata.len() as usize > self.sandbox.max_read_bytes && start.is_none() && end.is_none()
        {
            return Err(Error::ToolExecution(format!(
                "file {} is {} bytes (cap {}); request a line range",
                resolved.display(),
                metadata.len(),
                self.sandbox.max_read_bytes
            )));
        }

        let raw = std::fs::read_to_string(&resolved)
            .map_err(|e| Error::ToolExecution(format!("read {}: {e}", resolved.display())))?;

        let total_lines = raw.lines().count();
        let content = match (start, end) {
            (None, None) => raw.clone(),
            (s, e) => {
                let s = s.unwrap_or(1).max(1) as usize;
                let e = e.unwrap_or(total_lines as u64) as usize;
                raw.lines()
                    .enumerate()
                    .filter(|(idx, _)| {
                        let ln = idx + 1;
                        ln >= s && ln <= e
                    })
                    .map(|(_, l)| l)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "fs_read".into(),
                output: serde_json::json!({
                    "path": resolved.display().to_string(),
                    "bytes": metadata.len(),
                    "total_lines": total_lines,
                    "content": content,
                }),
                latency_ms: started.elapsed().as_millis() as u64,
            },
        })
    }
}

// ---- fs_patch -------------------------------------------------------------

pub struct FsPatchTool {
    meta: ToolContractMeta,
    pub sandbox: CodingSandbox,
}

impl Default for FsPatchTool {
    fn default() -> Self {
        FsPatchTool {
            meta: ToolContractMeta {
                name: "fs_patch".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Write,
                risk_class: RiskClass::Medium,
            },
            sandbox: CodingSandbox::default(),
        }
    }
}

impl ToolContract for FsPatchTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Modify a file inside the repository. Two modes: \
         `write` overwrites the entire file with `content` (creating parent \
         directories as needed); `replace` substitutes the first occurrence \
         of `anchor` with `replacement` and fails if the anchor is missing or \
         appears more than once. Always confined to allowed roots."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path":        { "type": "string", "description": "Target file path" },
                "mode":        { "type": "string", "enum": ["write", "replace"], "default": "replace" },
                "content":     { "type": "string", "description": "Full file content (mode=write)" },
                "anchor":      { "type": "string", "description": "Exact text to locate (mode=replace)" },
                "replacement": { "type": "string", "description": "Text to substitute (mode=replace)" }
            },
            "required": ["path", "mode"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        let obj = args.as_object().ok_or_else(|| Error::ToolTypeMismatch {
            tool: "fs_patch".into(),
            detail: "args must be an object".into(),
        })?;
        let mode =
            obj.get("mode")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::ToolTypeMismatch {
                    tool: "fs_patch".into(),
                    detail: "missing string 'mode'".into(),
                })?;
        match mode {
            "write" => {
                obj.get("content").and_then(|v| v.as_str()).ok_or_else(|| {
                    Error::ToolTypeMismatch {
                        tool: "fs_patch".into(),
                        detail: "mode=write requires string 'content'".into(),
                    }
                })?;
            }
            "replace" => {
                obj.get("anchor").and_then(|v| v.as_str()).ok_or_else(|| {
                    Error::ToolTypeMismatch {
                        tool: "fs_patch".into(),
                        detail: "mode=replace requires string 'anchor'".into(),
                    }
                })?;
                obj.get("replacement")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::ToolTypeMismatch {
                        tool: "fs_patch".into(),
                        detail: "mode=replace requires string 'replacement'".into(),
                    })?;
            }
            other => {
                return Err(Error::ToolTypeMismatch {
                    tool: "fs_patch".into(),
                    detail: format!("unsupported mode '{other}'"),
                });
            }
        }
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let started = Instant::now();
        let path = inv.args["path"].as_str().unwrap();
        let mode = inv.args["mode"].as_str().unwrap();
        let resolved = self.sandbox.confine(path)?;

        let (bytes_written, summary) = match mode {
            "write" => {
                let content = inv.args["content"].as_str().unwrap();
                if let Some(parent) = resolved.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        Error::ToolExecution(format!("mkdir {}: {e}", parent.display()))
                    })?;
                }
                std::fs::write(&resolved, content).map_err(|e| {
                    Error::ToolExecution(format!("write {}: {e}", resolved.display()))
                })?;
                (content.len() as u64, "wrote".to_string())
            }
            "replace" => {
                let anchor = inv.args["anchor"].as_str().unwrap();
                let replacement = inv.args["replacement"].as_str().unwrap();
                let original = std::fs::read_to_string(&resolved).map_err(|e| {
                    Error::ToolExecution(format!("read {}: {e}", resolved.display()))
                })?;
                let count = original.matches(anchor).count();
                if count == 0 {
                    return Err(Error::ToolExecution(format!(
                        "anchor not found in {}",
                        resolved.display()
                    )));
                }
                if count > 1 {
                    return Err(Error::ToolExecution(format!(
                        "anchor matches {count} times in {} — narrow it",
                        resolved.display()
                    )));
                }
                let updated = original.replacen(anchor, replacement, 1);
                std::fs::write(&resolved, &updated).map_err(|e| {
                    Error::ToolExecution(format!("write {}: {e}", resolved.display()))
                })?;
                (updated.len() as u64, "patched".to_string())
            }
            _ => unreachable!("validate_args rejected unsupported mode"),
        };

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "fs_patch".into(),
                output: serde_json::json!({
                    "path": resolved.display().to_string(),
                    "mode": mode,
                    "bytes": bytes_written,
                    "result": summary,
                }),
                latency_ms: started.elapsed().as_millis() as u64,
            },
        })
    }
}

// ---- list_files -----------------------------------------------------------

pub struct ListFilesTool {
    meta: ToolContractMeta,
    pub sandbox: CodingSandbox,
}

impl Default for ListFilesTool {
    fn default() -> Self {
        ListFilesTool {
            meta: ToolContractMeta {
                name: "list_files".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Read,
                risk_class: RiskClass::Low,
            },
            sandbox: CodingSandbox::default(),
        }
    }
}

impl ToolContract for ListFilesTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "List files and directories under a given path (default: repo root). \
         Optional `depth` controls how many levels to descend (default 1, max 4). \
         Skips common noise dirs (target, node_modules, .git, dist, build)."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path":  { "type": "string", "description": "Directory path (default: repo root)" },
                "depth": { "type": "integer", "minimum": 1, "maximum": 4, "default": 1 }
            }
        })
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let started = Instant::now();
        let depth = inv
            .args
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, 4) as usize;
        let path = inv.args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let root = self.sandbox.confine(path)?;
        if !root.is_dir() {
            return Err(Error::ToolExecution(format!(
                "{} is not a directory",
                root.display()
            )));
        }

        let mut entries: Vec<Value> = Vec::new();
        walk(
            &root,
            &root,
            0,
            depth,
            &mut entries,
            self.sandbox.max_list_entries,
        )?;

        let truncated = entries.len() >= self.sandbox.max_list_entries;
        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "list_files".into(),
                output: serde_json::json!({
                    "root": root.display().to_string(),
                    "depth": depth,
                    "entries": entries,
                    "truncated": truncated,
                }),
                latency_ms: started.elapsed().as_millis() as u64,
            },
        })
    }
}

fn walk(
    base: &Path,
    cur: &Path,
    level: usize,
    max_depth: usize,
    out: &mut Vec<Value>,
    cap: usize,
) -> Result<()> {
    if level >= max_depth || out.len() >= cap {
        return Ok(());
    }
    let read = std::fs::read_dir(cur)
        .map_err(|e| Error::ToolExecution(format!("readdir {}: {e}", cur.display())))?;
    let mut children: Vec<_> = read.flatten().collect();
    children.sort_by_key(|d| d.file_name());

    for entry in children {
        if out.len() >= cap {
            break;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        if matches!(
            name_str.as_str(),
            "target" | "node_modules" | ".git" | "dist" | "build" | ".next"
        ) {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(base).unwrap_or(&path);
        let kind = if path.is_dir() { "dir" } else { "file" };
        out.push(serde_json::json!({
            "path": rel.display().to_string(),
            "kind": kind,
        }));
        if path.is_dir() {
            walk(base, &path, level + 1, max_depth, out, cap)?;
        }
    }
    Ok(())
}

// ---- repo_map -------------------------------------------------------------

pub struct RepoMapTool {
    meta: ToolContractMeta,
    pub sandbox: CodingSandbox,
}

impl Default for RepoMapTool {
    fn default() -> Self {
        RepoMapTool {
            meta: ToolContractMeta {
                name: "repo_map".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Read,
                risk_class: RiskClass::Low,
            },
            sandbox: CodingSandbox::default(),
        }
    }
}

impl ToolContract for RepoMapTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Summarise the repository: top-level directory structure, detected \
         language/build system (Cargo, package.json, pyproject.toml, go.mod), \
         and primary entry points. Cheap orientation call — run this first \
         when you do not yet know the layout."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let started = Instant::now();
        let root = self.sandbox.primary_root();
        let canonical = root
            .canonicalize()
            .map_err(|e| Error::ToolExecution(format!("canonicalise {}: {e}", root.display())))?;

        let mut top_level = Vec::new();
        if let Ok(read) = std::fs::read_dir(&canonical) {
            let mut children: Vec<_> = read.flatten().collect();
            children.sort_by_key(|d| d.file_name());
            for entry in children {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules") {
                    continue;
                }
                let kind = if entry.path().is_dir() { "dir" } else { "file" };
                top_level.push(serde_json::json!({ "name": name, "kind": kind }));
            }
        }

        let mut markers = Vec::new();
        for marker in [
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "go.mod",
            "pnpm-workspace.yaml",
            "tsconfig.json",
            "Makefile",
        ] {
            if canonical.join(marker).exists() {
                markers.push(marker);
            }
        }

        let crates = collect_cargo_members(&canonical);

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "repo_map".into(),
                output: serde_json::json!({
                    "root": canonical.display().to_string(),
                    "top_level": top_level,
                    "markers": markers,
                    "cargo_crates": crates,
                }),
                latency_ms: started.elapsed().as_millis() as u64,
            },
        })
    }
}

fn collect_cargo_members(root: &Path) -> Vec<String> {
    let crates_dir = root.join("crates");
    let mut out = Vec::new();
    if let Ok(read) = std::fs::read_dir(&crates_dir) {
        for entry in read.flatten() {
            if entry.path().is_dir() {
                out.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out
}

// ---- grep -----------------------------------------------------------------

pub struct GrepTool {
    meta: ToolContractMeta,
    pub sandbox: CodingSandbox,
}

impl Default for GrepTool {
    fn default() -> Self {
        GrepTool {
            meta: ToolContractMeta {
                name: "grep".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::Read,
                risk_class: RiskClass::Low,
            },
            sandbox: CodingSandbox::default(),
        }
    }
}

impl ToolContract for GrepTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Search for a literal substring across files under a directory. \
         Returns up to 256 matches with `path:line: text`. Optional \
         `extension` filter (e.g. `rs`, `ts`) restricts the file set. Case \
         sensitive by default."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern":   { "type": "string", "description": "Literal substring to match" },
                "path":      { "type": "string", "description": "Directory to search (default: repo root)" },
                "extension": { "type": "string", "description": "File extension filter without dot (e.g. 'rs')" }
            },
            "required": ["pattern"]
        })
    }

    fn validate_args(&self, args: &Value) -> Result<()> {
        args.as_object()
            .and_then(|o| o.get("pattern").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::ToolTypeMismatch {
                tool: "grep".into(),
                detail: "missing non-empty string 'pattern'".into(),
            })?;
        Ok(())
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let started = Instant::now();
        let pattern = inv.args["pattern"].as_str().unwrap();
        let path = inv.args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let extension = inv.args.get("extension").and_then(|v| v.as_str());

        let root = self.sandbox.confine(path)?;
        let mut matches: Vec<Value> = Vec::new();
        let mut files_scanned = 0usize;

        scan(
            &root,
            &root,
            pattern,
            extension,
            &mut matches,
            &mut files_scanned,
            self.sandbox.max_grep_matches,
        )?;

        let truncated = matches.len() >= self.sandbox.max_grep_matches;
        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "grep".into(),
                output: serde_json::json!({
                    "pattern": pattern,
                    "root": root.display().to_string(),
                    "files_scanned": files_scanned,
                    "matches": matches,
                    "truncated": truncated,
                }),
                latency_ms: started.elapsed().as_millis() as u64,
            },
        })
    }
}

fn scan(
    base: &Path,
    cur: &Path,
    pattern: &str,
    extension: Option<&str>,
    out: &mut Vec<Value>,
    files_scanned: &mut usize,
    cap: usize,
) -> Result<()> {
    if out.len() >= cap {
        return Ok(());
    }
    let read = match std::fs::read_dir(cur) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    let mut children: Vec<_> = read.flatten().collect();
    children.sort_by_key(|d| d.file_name());

    for entry in children {
        if out.len() >= cap {
            break;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        if name_str.starts_with('.')
            || matches!(
                name_str.as_str(),
                "target" | "node_modules" | "dist" | "build" | ".next"
            )
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            scan(base, &path, pattern, extension, out, files_scanned, cap)?;
            continue;
        }
        if let Some(ext) = extension {
            if path.extension().and_then(|e| e.to_str()) != Some(ext) {
                continue;
            }
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        *files_scanned += 1;
        for (idx, line) in content.lines().enumerate() {
            if line.contains(pattern) {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                out.push(serde_json::json!({
                    "path": rel.display().to_string(),
                    "line": idx + 1,
                    "text": line,
                }));
                if out.len() >= cap {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

// ---- test_run -------------------------------------------------------------

pub struct TestRunTool {
    meta: ToolContractMeta,
    pub sandbox: CodingSandbox,
    pub timeout_secs: u64,
}

impl Default for TestRunTool {
    fn default() -> Self {
        TestRunTool {
            meta: ToolContractMeta {
                name: "test_run".into(),
                version: "0.0.1".into(),
                effect_class: EffectClass::External,
                risk_class: RiskClass::Medium,
            },
            sandbox: CodingSandbox::default(),
            timeout_secs: 300,
        }
    }
}

impl ToolContract for TestRunTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }

    fn description(&self) -> &str {
        "Run the project's test suite. Auto-detects the runner from repo \
         markers (Cargo.toml → `cargo test`, package.json → `npm test`, \
         pyproject.toml → `pytest`, go.mod → `go test ./...`). Pass `package` \
         to scope a Cargo workspace test. Returns stdout, stderr, and exit \
         code — the cognition loop should treat non-zero as a failure signal."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "package": { "type": "string", "description": "Cargo package name to scope (`-p <name>`)" },
                "filter":  { "type": "string", "description": "Test name filter passed to the runner" }
            }
        })
    }

    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let started = Instant::now();
        let root = self.sandbox.primary_root();
        let canonical = root
            .canonicalize()
            .map_err(|e| Error::ToolExecution(format!("canonicalise {}: {e}", root.display())))?;

        let package = inv.args.get("package").and_then(|v| v.as_str());
        let filter = inv.args.get("filter").and_then(|v| v.as_str());

        let (program, args) = if canonical.join("Cargo.toml").exists() {
            let mut a: Vec<String> = vec!["test".into()];
            if let Some(p) = package {
                a.push("-p".into());
                a.push(p.into());
            }
            if let Some(f) = filter {
                a.push(f.into());
            }
            ("cargo".to_string(), a)
        } else if canonical.join("package.json").exists() {
            ("npm".to_string(), vec!["test".into(), "--silent".into()])
        } else if canonical.join("pyproject.toml").exists() || canonical.join("setup.py").exists() {
            let mut a: Vec<String> = vec![];
            if let Some(f) = filter {
                a.push("-k".into());
                a.push(f.into());
            }
            ("pytest".to_string(), a)
        } else if canonical.join("go.mod").exists() {
            ("go".to_string(), vec!["test".into(), "./...".into()])
        } else {
            return Err(Error::ToolExecution(
                "no recognised test runner (need Cargo.toml, package.json, pyproject.toml, or go.mod)".into(),
            ));
        };

        let output = run_with_timeout(&program, &args, &canonical, self.timeout_secs)?;

        Ok(ToolOutcome {
            delta: StructuredDelta::default(),
            observation: Observation {
                tool: "test_run".into(),
                output: serde_json::json!({
                    "command": format!("{} {}", program, args.join(" ")),
                    "cwd": canonical.display().to_string(),
                    "exit_code": output.exit_code,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                    "passed": output.exit_code == 0,
                }),
                latency_ms: started.elapsed().as_millis() as u64,
            },
        })
    }
}

struct ProcessOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn run_with_timeout(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout_secs: u64,
) -> Result<ProcessOutput> {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let (tx, rx) = mpsc::channel();
    let program_owned = program.to_string();
    let program_for_thread = program_owned.clone();
    let args_for_thread: Vec<String> = args.to_vec();
    let cwd_for_thread = cwd.to_path_buf();

    thread::spawn(move || {
        let result = Command::new(&program_for_thread)
            .args(&args_for_thread)
            .current_dir(&cwd_for_thread)
            .output();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
        Ok(Ok(output)) => Ok(ProcessOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: truncate(
                String::from_utf8_lossy(&output.stdout).into_owned(),
                16 * 1024,
            ),
            stderr: truncate(
                String::from_utf8_lossy(&output.stderr).into_owned(),
                16 * 1024,
            ),
        }),
        Ok(Err(e)) => Err(Error::ToolExecution(format!("spawn {program_owned}: {e}"))),
        Err(_) => Err(Error::ToolExecution(format!(
            "{program_owned} timed out after {timeout_secs}s"
        ))),
    }
}

fn truncate(mut s: String, cap: usize) -> String {
    if s.len() > cap {
        s.truncate(cap);
        s.push_str("\n... [truncated]");
    }
    s
}

// `BTreeMap` import is required by some downstream consumers in tests; suppress
// dead-code lint when not used directly here.
#[allow(dead_code)]
fn _unused_btreemap() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use thymos_core::world::World;

    fn invoke<'a>(args: &'a Value, world: &'a World) -> ToolInvocation<'a> {
        ToolInvocation { args, world }
    }

    #[test]
    fn fs_read_within_root() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();
        let mut tool = FsReadTool::default();
        tool.sandbox.allowed_roots = vec![dir.path().display().to_string()];

        let world = World::default();
        let args = serde_json::json!({ "path": file.display().to_string() });
        let outcome = tool.execute(&invoke(&args, &world)).unwrap();
        let content = outcome.observation.output["content"].as_str().unwrap();
        assert!(content.contains("line2"));
    }

    #[test]
    fn fs_read_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = std::env::temp_dir().join("definitely-outside.txt");
        std::fs::write(&outside, "x").unwrap();
        let mut tool = FsReadTool::default();
        tool.sandbox.allowed_roots = vec![dir.path().display().to_string()];

        let world = World::default();
        let args = serde_json::json!({ "path": outside.display().to_string() });
        let err = match tool.execute(&invoke(&args, &world)) {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("escapes allowed roots"));
    }

    #[test]
    fn fs_patch_replace_unique_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("greet.txt");
        std::fs::write(&file, "hello world\n").unwrap();
        let mut tool = FsPatchTool::default();
        tool.sandbox.allowed_roots = vec![dir.path().display().to_string()];

        let world = World::default();
        let args = serde_json::json!({
            "path": file.display().to_string(),
            "mode": "replace",
            "anchor": "world",
            "replacement": "thymos",
        });
        tool.execute(&invoke(&args, &world)).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello thymos\n");
    }

    #[test]
    fn fs_patch_rejects_ambiguous_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("dup.txt");
        std::fs::write(&file, "x x\n").unwrap();
        let mut tool = FsPatchTool::default();
        tool.sandbox.allowed_roots = vec![dir.path().display().to_string()];

        let world = World::default();
        let args = serde_json::json!({
            "path": file.display().to_string(),
            "mode": "replace",
            "anchor": "x",
            "replacement": "y",
        });
        let err = match tool.execute(&invoke(&args, &world)) {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("matches"));
    }

    #[test]
    fn grep_finds_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn alpha() {}\nfn beta() {}\n").unwrap();
        let mut tool = GrepTool::default();
        tool.sandbox.allowed_roots = vec![dir.path().display().to_string()];

        let world = World::default();
        let args = serde_json::json!({
            "pattern": "alpha",
            "path": dir.path().display().to_string(),
            "extension": "rs",
        });
        let outcome = tool.execute(&invoke(&args, &world)).unwrap();
        let matches = outcome.observation.output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
    }
}
