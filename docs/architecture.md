---
layout: default
title: Architecture
eyebrow: Runtime topology
subtitle: OpenThymos separates cognition, authority, programmable capabilities, sandboxed execution, ledgering, and replay into explicit runtime planes.
permalink: /architecture/
---

# Architecture

OpenThymos is organized around one rule:

```text
cognition proposes; the runtime governs; the ledger records
```

The system is not a tool loop hidden behind a model call. It is a set of
runtime planes that convert untrusted intent into authorized, policy-checked,
ledgered execution.

## Runtime Planes

| Plane | Crate | Authority |
| --- | --- | --- |
| Cognition plane | `thymos-cognition` | May emit `Intent`; may not execute tools. |
| Compilation plane | `thymos-compiler` | May stage, suspend, or reject proposals. |
| Governance plane | `thymos-policy`, writ types in `thymos-core` | May constrain authority through policy and writ validation. |
| Capability plane | `thymos-tools` | Registers Rust contracts, JSON manifests, MCP bridge tools, and coding tools. |
| Execution plane | `thymos-runtime`, `thymos-tools`, `thymos-worker` | May invoke typed capabilities after a staged proposal. |
| Ledger plane | `thymos-ledger` | Records root, commit, rejection, approval, delegation, and branch entries. |
| Projection plane | `World` in `thymos-core` | Folds committed deltas into current runtime state. |
| Surface plane | `thymos-server`, `thymos-cli`, terminal shell, web console, VS Code client | Observes and controls runs without becoming the source of truth. |

## Control Flow

The primary cycle is Intent -> Proposal -> Commit.

```text
Provider Adapter
      |
      v
   Intent
      |
      v
Compiler + Policy Engine + Writ Validator + Tool Registry
      |
      +---- reject ----------> Ledger: Rejection
      |
      +---- require approval -> Ledger: PendingApproval
      |
      v
   Proposal
      |
      v
Capability Contract
      |
      v
Observation + Structured Delta
      |
      v
   Commit
      |
      v
Append-only Ledger
      |
      v
World Projection / Replay
```

No client surface owns runtime truth. A command line client, terminal shell,
web console, and editor extension all read the same run state and
ledger-derived projection.

## Capability Boundary

Capabilities enter through `ToolRegistry`. A capability can be a Rust
`ToolContract`, a JSON manifest tool loaded from `THYMOS_TOOL_MANIFEST_DIRS`,
or an MCP bridge tool discovered from a subprocess server. Each capability
declares metadata, an input schema, an effect class, and a risk class.

The compiler resolves requested capability names against the registry before
execution. Unknown capabilities reject before the sandbox or tool boundary is
reached.

## Compile Boundary

The compiler is the first authority boundary. It evaluates an intent against:

- intent kind support
- writ signature validity
- writ time window
- writ tool scope
- tool registry resolution
- budget projection
- tool argument type validation
- tool preconditions
- policy engine decision

The compiler emits one of:

- staged proposal
- suspended proposal requiring approval
- typed rejection

Capability execution is unreachable unless compilation returns a staged proposal or
an approved suspended proposal.

## Effect Boundary

Capabilities are invoked only through the runtime. A capability receives:

- validated arguments
- the current world projection

A capability returns:

- an observation
- a structured delta

The runtime checks postconditions, trial-applies the delta, constructs a commit,
and appends it to the ledger. The capability does not directly mutate
authoritative state.

Sandboxing depends on the capability class. Built-in coding tools are
path-confined. Stock shell and HTTP capabilities can be routed through
`thymos-worker` in production mode. High-risk manifest capabilities should be
promoted to Rust contracts or hardened external services when worker receipts
are required.

## Ledger Boundary

The ledger is append-only and content-addressed. Each entry contains:

- entry id
- trajectory id
- parent entry id
- sequence number
- entry kind
- typed payload

The replay verifier recomputes payload hashes, verifies parent linkage, checks
sequence continuity, and folds commit deltas into a rebuilt world.

## Projection Boundary

`World` is a projection, not a database of record. It is rebuilt from ledger
entries by applying committed deltas in order. Branches first fold the ancestor
trajectory up to the branch point, then fold branch-local commits.

Projection failures are invariant failures. They indicate an invalid delta, an
invalid parent sequence, or incompatible runtime semantics.

## Provider Boundary

Providers are replaceable intent sources. Anthropic, OpenAI, Hugging Face,
local OpenAI-compatible servers, LM Studio, and mock cognition adapters all
enter the runtime through the same `Cognition::step` contract.

Provider outputs do not carry authority. Provider changes may affect which
intents are proposed, but they must not change how proposals are compiled,
authorized, executed, committed, or replayed.

## Coordination Boundary

Delegation creates child trajectories with child writs. A child writ must be a
strict subset of the parent writ. Parent and child histories remain linked by
ledger entries rather than hidden control flow.

Multi-agent coordination is therefore a graph of bounded trajectories, not a
set of unstructured model sessions.

## Failure Semantics

Failures are runtime events. They are not discarded control-flow exceptions.

OpenThymos distinguishes:

- compiler rejection
- policy suspension
- operator approval or denial
- tool execution failure
- postcondition failure
- commit failure
- cancellation
- cognition termination

Only committed deltas change projected world state. Failed attempts may affect
execution logs and summaries, but they do not become world state unless
recorded as ledger entries.

## Architecture Invariants

- Cognition cannot execute a tool.
- A proposal cannot exist without a writ id and policy trace.
- A commit cannot exist without a proposal id, writ id, parent, sequence, delta,
  observation list, and compiler version.
- A ledger entry cannot be replayed if its hash, parent, or sequence is invalid.
- A child writ cannot exceed its parent writ.
- Provider adapters cannot modify ledger semantics.
- Surfaces observe the runtime; they do not define the runtime.

Related documents:

- [Specification](specification.md)
- [Programmable Capabilities](programmable-capabilities.md)
- [Deterministic Execution](deterministic-execution.md)
- [Execution Ledger](execution-ledger.md)
- [Replay](replay.md)
- [Runtime Invariants](runtime-invariants.md)
