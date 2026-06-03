//! `thymos-worker` — the process-isolation boundary for tool execution.
//!
//! This binary is intentionally tiny. Its job is not to *hold* logic but to *be
//! a separate OS process*: the runtime spawns it to execute side-effecting tools
//! (shell, filesystem, http) under a restricted capability profile — restricted
//! environment, isolated `$HOME`, blocked-command patterns, output caps — so a
//! tool invocation cannot reach beyond what its writ authorizes even if the tool
//! itself misbehaves.
//!
//! The substance lives in [`thymos_tools::worker_entrypoint`], which reads a
//! `ToolWorkerRequest` from stdin, executes it in this confined process, and
//! writes a `ToolWorkerResponse` to stdout. Keeping the entrypoint in the
//! library (not here) means the same code is unit-tested in `thymos-tools` and
//! can also run in-process for tests; this crate is just the deployable process
//! shell. The thinness is the design, not a stub.

fn main() {
    if let Err(err) = thymos_tools::worker_entrypoint() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
