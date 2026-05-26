---
layout: default
title: API Reference
eyebrow: HTTP surface
subtitle: The Thymos server exposes run creation, execution-session state, ledger streams, world state, approvals, and control endpoints.
permalink: /api-reference/
---

Base URL: `http://localhost:3001`

## Health

### GET /health

Returns server liveness and runtime mode.

## Runs

### POST /runs

Start a new backend run.

Example request:

```json
{
  "task": "Inspect the repo and explain how the runtime works",
  "max_steps": 24,
  "tool_scopes": ["repo_map", "fs_read", "grep", "test_run"],
  "cognition": {
    "provider": "mock"
  }
}
```

Example response:

```json
{
  "run_id": "uuid",
  "task": "Inspect the repo and explain how the runtime works",
  "status": "running"
}
```

`tool_scopes` binds the run writ to registered capability names. Built-in
coding tools, manifest-backed capabilities loaded from
`THYMOS_TOOL_MANIFEST_DIRS`, and MCP bridge tools all use the same scope check.

### GET /runs/:id

Returns the persisted run record and summary.

Example summary fields:

- `steps_executed`
- `intents_submitted`
- `commits`
- `rejections`
- `failures`
- `final_answer`
- `terminated_by`

### GET /runs/:id/execution

Returns the live **execution session** used by the web console, CLI, and VS Code sidebar.

Example fields:

- `status`
- `phase`
- `operator_state`
- `current_step`
- `max_steps`
- `active_tool`
- `final_answer`
- `counters`
- `log`

### GET /runs/:id/execution/stream

SSE stream of execution-session snapshots.

This is the best stream to consume if you want operator-facing runtime truth.
Clients should keep this connection open and allow browser/EventSource reconnects.
For production UIs, pair it with periodic `GET /runs/:id/execution` refreshes so
the screen remains current through proxy restarts, network blips, or tab sleep.

### GET /runs/:id/stream

SSE stream of raw cognition events such as tokens and tool-use deltas.

This is useful for model-side visibility, but it is not the authoritative execution state.
Use the execution-session stream for user-facing status, counters, active tool,
approvals, and final outcome.

### GET /runs/:id/events

SSE stream of ledger entry events.

### GET /runs/:id/world

Returns the current projected world state for the run.

### GET /runs/:id/world/at?seq=N

Returns the projected world state replayed up to a specific sequence number.

### GET /runs/:id/replay

Verifies the execution ledger for the run and folds committed deltas into a
replay report.

Useful query parameters:

- `require_compiler`: reject commits whose recorded compiler version differs
  from the given value

### POST /runs/:id/resume

Resume a previously started or failed run.

### POST /runs/:id/cancel

Cancel a currently running run.

### POST /runs/:id/branch

Create a shadow branch from a specific commit.

### GET /runs/:id/delegations

List child trajectories created through delegation.

## Approvals

### POST /runs/:id/approvals/:channel

Approve or deny a pending proposal.

Example request:

```json
{ "approve": true }
```

## Audit

### GET /audit/entries

Query ledger entries with optional filters.

Useful filters:

- `run_id`
- `kind`
- `from`
- `to`
- `limit`
- `format=json|csv`

### GET /audit/entries/count

Count matching ledger entries without fetching them.

## Usage

### GET /usage

Per-key usage stats when the API gateway is configured.

## Marketplace

### GET /marketplace/packages

List published packages.

### GET /marketplace/packages/:name

Get one package.

### POST /marketplace/packages

Publish a new package.

## Which endpoint should a new client use?

Use:

- `/runs` to create work
- `/runs/:id/execution` for current operator state
- `/runs/:id/execution/stream` for live runtime updates
- `/runs/:id/world` for current projected world state
- `/runs/:id/replay` for ledger verification and deterministic fold reports
- `/runs/:id/approvals/:channel` for human-in-the-loop actions

Use `/runs/:id/stream` only when you specifically want raw cognition streaming.
