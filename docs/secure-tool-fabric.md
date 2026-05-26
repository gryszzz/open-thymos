---
layout: default
title: Secure Tool Fabric
eyebrow: Isolation
subtitle: Risky tools don't run in the same process as the agent loop. They run behind a worker boundary with a typed contract.
permalink: /secure-tool-fabric/
---

## Threat model

The language model is an untrusted actor. The runtime must assume it will
propose the worst possible shell command. The secure tool fabric makes that
proposal's blast radius small and auditable.

## Architecture

```
Runtime (trusted) ─┬─ ToolWorkerRequest ──▶ thymos-worker (subprocess)
                   │                           │
                   │                           ├─ timeout kill
                   │                           ├─ capability gating
                   │                           ├─ path confinement
                   │                           └─ receipt-bearing response
                   ◀────── ToolWorkerResponse ─┘
```

The runtime never executes the model's shell string itself. It serializes a
`ToolWorkerRequest` and hands it to `thymos-worker`. The worker enforces the
policy the request declares and returns a `ToolWorkerResponse` with an
execution receipt.

This worker path is the production sandbox for high-risk built-in capabilities.
Manifest capabilities are intended for low-risk local extension; promote risky
manifest behavior into a Rust `ToolContract` or hardened external service when
you need worker receipts and stronger isolation. Manifest loading validates
tool names, prevents shadowing built-in capabilities, loads files in stable
order, and blocks private hosts for HTTP manifest calls by default.

## THYMOS-native shell

The shell tool is not a thin `Command::new` wrapper. Every invocation carries:

- `purpose` — free-text rationale (goes into the ledger observation).
- `capability_profile` — `inspect`, `build`, `mutate`, or `networked`.
- `cwd` — confined to the writ's allowed roots.
- `timeout_secs` — hard kill, not soft wait.
- Isolated `HOME` when `isolate_home` is set.
- Receipt — BLAKE3 digest of the canonical request payload.

### Profiles

| Profile    | Allows                                                                   |
|------------|--------------------------------------------------------------------------|
| `inspect`  | `ls`, `cat`, `rg`, `find`, `git`, `stat`, env / which, bounded viewing   |
| `build`    | `inspect` + `cargo`, `rustc`, `make`, `npm`, `pnpm`, `yarn`, `go`, `pytest` |
| `mutate`   | `build` + `cp`, `mv`, `mkdir`, `touch`, `chmod`, `rm`                    |
| `networked`| any command (egress allowed). Used only behind explicit writ scope.      |

Chaining sequences (`&&`, `||`, `;`) are rejected unless the profile's wrapper
explicitly allows them. The model can't smuggle a second command through the
first one.

## HTTP tool

The `http` tool shares the worker seam. It enforces:

- Domain allowlist (when non-empty).
- Private / loopback host blocking by default.
- Per-call timeout.
- Structured response: status, headers, body bytes.

## Execution modes

- **In-process** — runs the same request/response shape inside the runtime.
  Fast, no isolation. The default in development.
- **Worker** — `THYMOS_TOOL_FABRIC=worker` + `THYMOS_WORKER_BIN=<path>`.
  Each invocation spawns a subprocess. Required in production mode.

## Next hardening steps

- Container- or microVM-backed workers.
- Egress enforcement below the process layer.
- Signed worker attestation.
- Browser / code-exec worker classes with per-capability sandboxes.
