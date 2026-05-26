<div align="center">

<img src="Thymos-logo.PNG" alt="Thymos" width="128" height="128" />

# Thymos Rust Workspace

**The Rust execution framework and sandbox behind the OpenThymos coding-agent surfaces.**

</div>

---

This directory contains the Rust implementation of the OpenThymos runtime,
server, CLI, terminal shell, worker sandbox, programmable capability layer, and
core execution model.

If the top-level README explains **what Thymos is**, this README explains **where the runtime lives**.

## What Lives Here

- `thymos-runtime` — the agent loop and execution orchestration
- `thymos-server` — the shared HTTP runtime used by the web app, CLI, and editor integrations
- `thymos-cli` — terminal client and interactive shell
- `thymos-worker` — worker boundary for higher-risk shell/http execution
- `thymos-cognition` — model adapters
- `thymos-tools` — typed tools, manifest capabilities, MCP bridge tools, shell, HTTP, memory, and delegation
- `thymos-ledger` — durable execution history
- `thymos-policy` — capability and approval enforcement

## What Thymos Does In Practice

The runtime takes a natural-language task and drives an execution loop:

`Intent -> Proposal -> Commit`

That loop is shared across:

- the web operator console
- the CLI and shell
- the VS Code sidebar
- any other client that attaches to the server

Every surface can observe the same run because the server exposes a unified execution session and live streaming state.

Capabilities are programmable. First-party tools implement `ToolContract` in
Rust; local operators can load JSON manifest tools at startup with
`THYMOS_TOOL_MANIFEST_DIRS`; MCP bridge tools register into the same tool
registry. Writ scopes and policy decide which capability can execute, while
path confinement and `thymos-worker` provide the sandbox boundary for built-in
coding, shell, and HTTP effects.

## Quick Start

### Start the runtime server

```bash
cargo run -p thymos-server
```

### Start with local capability manifests

```bash
THYMOS_TOOL_MANIFEST_DIRS=../tools cargo run -p thymos-server
```

### Run a task from the CLI

```bash
cargo run -p thymos-cli -- run "Inspect the repo and summarize the runtime" --provider mock --follow
```

### Follow a run later

```bash
cargo run -p thymos-cli -- status <run-id>
cargo run -p thymos-cli -- stream <run-id>
cargo run -p thymos-cli -- runs show <run-id>
```

## Current Runtime Shape

Today the runtime supports:

- autonomous step-by-step execution
- retries on transient cognition failures
- structured runtime recovery after tool execution failures
- approval suspension and resume flows
- cancellation and resume
- shared execution sessions for operator-facing clients
- programmable capability manifests loaded at runtime
- world replay and branching
- local and hosted cognition providers

## Surfaces Powered By This Workspace

- `POST /runs` starts a shared backend run
- `/runs/:id/execution` exposes the current execution session
- `/runs/:id/execution/stream` streams live execution state
- `/runs/:id/stream` streams raw cognition events
- `/runs/:id/world` exposes current projected world state

## Recommended Development Flow

### Backend only

```bash
cargo test --workspace
```

### Full product loop

1. Run `cargo run -p thymos-server`
2. Run the Next.js app from the repo root with `npm run dev`
3. Open `/runs` in the browser or use the CLI
4. If working on the VS Code client, compile `clients/vscode`

## More Docs

- [Top-level README](../README.md)
- [Getting Started](../docs/getting-started.md)
- [Architecture](../docs/architecture.md)
- [Programmable Capabilities](../docs/programmable-capabilities.md)
- [API Reference](../docs/api-reference.md)
