<div align="center">

<img src="thymos/Thymos-logo.PNG" alt="Thymos" width="132" height="132" />

# OpenThymos

### Unified AI execution runtime, framework, and sandbox for coding agents.

**Intent -> Proposal -> Commit**

<p>
  <a href="https://gryszzz.github.io/OpenThymos/"><strong>Documentation</strong></a>
  |
  <a href="docs/specification.md"><strong>Specification</strong></a>
  |
  <a href="docs/architecture.md"><strong>Architecture</strong></a>
  |
  <a href="docs/programmable-capabilities.md"><strong>Capabilities</strong></a>
  |
  <a href="docs/replay.md"><strong>Replay</strong></a>
  |
  <a href="docs/package-distribution.md"><strong>Packages</strong></a>
  |
  <a href="GOVERNANCE.md"><strong>Governance</strong></a>
</p>

</div>

---

OpenThymos is a Rust execution runtime and programmable capability framework
for coding agents across CLI, VS Code, terminal, and web surfaces.

Agents do not act autonomously. They propose. The runtime compiles the
proposal, checks authority, routes approved capabilities through governed
execution boundaries, and records the outcome in a replayable ledger.

The runtime treats cognition as an untrusted source of intent. A model,
planner, or local rule engine may request an action, but it cannot directly
mutate state, call a tool, spend budget, delegate authority, or erase history.
Every effect must pass through a typed proposal, a capability writ, a policy
trace, and an append-only execution ledger.

OpenThymos is not a model wrapper. It is an execution system: a unified
backend for agents that need authority boundaries, programmable capabilities,
runtime sandboxing, reproducible state, and durable runtime semantics.

## What OpenThymos Is

OpenThymos defines a small runtime protocol:

```text
Intent -> Proposal -> Commit
```

An `Intent` is emitted by cognition. It has no authority.

A `Proposal` is compiled by the runtime. It binds the intent to a writ, a tool
contract, a budget projection, and a policy trace.

A `Commit` is the only record that mutates world state. It contains the
structured delta, the observed tool output, the writ id, the proposal id, the
compiler version, and the parent ledger head.

The runtime is implemented as a Rust workspace under [`thymos`](thymos):

| Plane | Crate | Responsibility |
| --- | --- | --- |
| Core protocol | `thymos-core` | Intent, Proposal, Commit, Writ, World, structured deltas |
| Compiler | `thymos-compiler` | Deterministic proposal compilation and rejection |
| Policy | `thymos-policy` | Pure policy evaluation and policy traces |
| Runtime | `thymos-runtime` | IPC cycle, approvals, delegation, projection, resume |
| Ledger | `thymos-ledger` | Append-only entries, hash chain, replay verifier |
| Cognition | `thymos-cognition` | Provider abstraction for proposers |
| Capabilities | `thymos-tools` | Rust tool contracts, JSON manifest tools, MCP bridges, and observed effects |
| Surfaces | `thymos-server`, `thymos-cli`, `clients/vscode`, `src` | CLI, VS Code, terminal, and web access to one shared run |

## Unified Agent Surfaces

OpenThymos is one runtime with several clients, not several agents with
several histories.

- `thymos-server` owns execution sessions, approvals, streams, and ledger state
- `thymos-cli` starts and follows runs from terminal workflows
- `thymos shell` gives terminal-first users a persistent runtime session
- `clients/vscode` exposes editor-native run visibility and approvals
- the Next.js web console in `src` watches the same run state over HTTP and SSE

The surface can change without changing runtime truth. A run can start from
the CLI, be monitored in the browser, approved in VS Code, and replayed later
from the terminal.

## Programmable Capabilities

Capabilities are the effect boundary. They can be added in three ways:

- implement `ToolContract` in Rust for first-party, strongly typed tools
- drop JSON tool manifests into `THYMOS_TOOL_MANIFEST_DIRS` for configurable
  shell, HTTP, or no-op capabilities
- register MCP tools through the `thymos-tools` MCP bridge

Every capability declares a schema, effect class, risk class, and observation
shape. Writ scopes and policy decide whether a capability may execute. Built-in
coding tools are path-confined, and the stock shell/HTTP capabilities can cross
the worker-backed sandbox when production isolation is required.
Manifest capabilities are validated at startup, loaded in deterministic order,
and cannot shadow existing built-in tools.

## Why Current Agent Frameworks Fail

Most agent systems collapse cognition and execution into one loop. A model
chooses a tool, executes it, reads the result, and continues. That design is
easy to demonstrate and hard to govern.

It fails because execution is hidden inside a stochastic process:

- tool calls are made before authority is modeled
- policy is applied as application code instead of runtime semantics
- provider behavior changes the execution path
- state is reconstructed from logs after the fact, if at all
- approvals are UI events rather than ledger events
- failures, retries, and rejections are not part of the same history
- replay cannot prove that the same proposal would have produced the same
  world projection

OpenThymos separates proposal from authority and authority from effect. The
model can propose a next action, but the runtime decides whether the proposal
can exist, whether it may execute, and how the result is committed.

## Core Runtime Model

The runtime cycle is deliberately narrow.

```text
1. Fold ledger entries into the current World projection.
2. Pass task, writ, world, tools, and recent history to cognition.
3. Accept one or more Intents.
4. Compile each Intent against writ, tool registry, budget, time window, and policy.
5. Stage, reject, or suspend the Proposal.
6. Execute only staged proposals through typed capability contracts.
7. Commit structured deltas and observations to the ledger.
8. Feed committed, rejected, failed, or suspended outcomes back into cognition.
```

The compiler path is pure for a given `(Intent, Writ, World, ToolRegistry,
PolicyEngine, CompileContext)`. Tool execution is the effect boundary. World
state is not authoritative; it is a projection obtained by folding committed
deltas from the ledger.

## Deterministic Execution

OpenThymos makes determinism a runtime property, not a prompt convention.

The current implementation enforces:

- canonical content hashes for intents, proposals, commits, writs, and ledger
  entries
- parent-chained ledger entries with contiguous sequence numbers
- compiler-version recording on each commit
- explicit capability contracts for argument validation, preconditions,
  postconditions, estimated cost, and structured deltas
- policy traces attached to proposals
- world projection by deterministic ledger folding
- provider abstraction where cognition produces intents but never effects

Non-deterministic inputs are admitted only at controlled boundaries. Once an
observation is committed, replay uses the ledgered observation and delta, not a
new provider response or a new tool call.

## Replayable Cognition

Replay is a proof procedure over the execution ledger. The HTTP runtime exposes
the verifier at `GET /runs/:id/replay`, and the CLI exposes the same verifier
as `thymos replay`.

```bash
thymos replay run_847 --verify --fold-world --policy-trace
```

Replay verifies the hash chain, checks sequence continuity, reapplies committed
deltas in order, compares the rebuilt world projection, and reports the
compiler versions seen during the run. Suspensions, rejections, delegations,
branches, and approvals remain visible as ledger entries rather than
out-of-band control flow.

The replay model is documented in [docs/replay.md](docs/replay.md). A complete
terminal walkthrough is in
[docs/demos/deterministic-replay.md](docs/demos/deterministic-replay.md).

## Policy Engine

The policy engine is a set of ordered pure functions:

```text
(Intent, Writ, World) -> PolicyDecision
```

A policy decision is one of:

- `Permit`
- `Deny(reason)`
- `RequireApproval { channel, reason }`

The compiler records the evaluated rule names and final decision in the
proposal's policy trace. A proposal that requires approval is written to the
ledger as `PendingApproval`; the runtime can later resume it through the same
proposal id after an operator decision.

## Capability Writs

Authority in OpenThymos is carried by signed capability writs.

A writ authorizes a subject to emit intents within declared tool scopes,
tenant boundaries, effect ceilings, budgets, time windows, and delegation
bounds. Child writs must be strict subsets of parent writs. Cross-tenant
delegation is forbidden. Lateral minting is invalid.

Writs make authority inspectable:

- who issued the authority
- which subject may act
- which tools are in scope
- which effects are allowed
- how much budget remains
- when the authority becomes valid and when it expires
- whether the subject may subdivide authority

See [docs/capability-writs.md](docs/capability-writs.md).

## Runtime Guarantees

OpenThymos uses formal runtime guarantees as design constraints.

| Guarantee | Statement |
| --- | --- |
| Deterministic replay | A valid ledger can be folded into the same world projection under the recorded commit sequence. |
| Runtime isolation | Cognition cannot execute tools or mutate state directly. |
| Execution integrity | Only staged proposals may reach the tool boundary; only commits may mutate projected world state. |
| Capability constraints | Tool scopes, budgets, time windows, effect ceilings, tenant boundaries, and delegation bounds are checked before execution. |
| Policy enforcement | Policy decisions are recorded as proposal traces and cannot be erased by a client surface. |
| Provider abstraction | Providers can change intent generation but not runtime semantics. |
| Auditability | Ledger entries preserve root, commit, rejection, pending approval, delegation, and branch records. |
| Trace persistence | Execution sessions and audit entries expose runtime status without becoming the source of truth. |
| Reproducibility | World state is derived from committed structured deltas, not from transient chat transcript state. |

The complete invariant set is in
[docs/runtime-invariants.md](docs/runtime-invariants.md).

## Architecture Overview

```text
                         +-------------------+
                         |   Cognition       |
                         | provider adapter  |
                         +---------+---------+
                                   |
                                   v
                         +-------------------+
                         | Intent            |
                         | no authority      |
                         +---------+---------+
                                   |
        +--------------------------+--------------------------+
        |                                                     |
        v                                                     v
+-------------------+                              +-------------------+
| Compiler          |                              | World projection  |
| writ, budget,     |<-----------------------------| ledger fold       |
| policy, tools     |                              +-------------------+
+---------+---------+
          |
          v
+-------------------+      permit       +-------------------+
| Proposal          |------------------>| Tool gateway      |
| policy trace      |                   | typed contracts   |
+----+---------+----+                   +---------+---------+
     |         |                                  |
     | deny    | require approval                 v
     v         v                         +-------------------+
+---------+ +-------------------+        | Observation +     |
|Reject   | | PendingApproval   |        | structured delta  |
+----+----+ +---------+---------+        +---------+---------+
     |                |                            |
     +----------------+----------------------------+
                      |
                      v
             +-------------------+
             | Execution ledger  |
             | append-only       |
             +---------+---------+
                       |
                       v
             +-------------------+
             | Replay / fold     |
             | audit projection  |
             +-------------------+
```

Deeper architecture notes are in [docs/architecture.md](docs/architecture.md).
Protocol-level diagrams are in [docs/diagrams.md](docs/diagrams.md).

## Benchmark Framework

OpenThymos benchmarks the runtime paths that matter for governed execution:

- replay speed
- execution overhead
- provider swap latency
- ledger folding performance
- tool execution latency
- state projection speed
- execution DAG traversal
- memory usage

The benchmark plan and reporting format are documented in
[docs/benchmarks.md](docs/benchmarks.md). Benchmark numbers should always
include hardware, storage backend, provider mode, compiler version, sample
ledger size, and whether the run was warm or cold.

## Install And Verify

```bash
./scripts/install.sh
export PATH="$HOME/.local/bin:$PATH"
source "$HOME/.config/thymos/thymos.env"
thymos doctor
```

Run the Rust runtime:

```bash
cd thymos
cargo run -p thymos-server
```

Load local programmable capabilities:

```bash
THYMOS_TOOL_MANIFEST_DIRS=../tools cargo run -p thymos-server
```

Run the operator console:

```bash
npm install
npm run dev
```

## GitHub Packages

OpenThymos publishes a container package through GitHub Packages on release
tags:

```bash
docker pull ghcr.io/gryszzz/openthymos-runtime:<tag>
docker run --rm -p 3001:3001 -v "$PWD/.thymos:/data" ghcr.io/gryszzz/openthymos-runtime:<tag>
```

The workflow also publishes `ghcr.io/gryszzz/thymos-server` as a compatibility
alias. Package publication is defined in `.github/workflows/release.yml` and
documented in [docs/package-distribution.md](docs/package-distribution.md).
Manual workflow dispatches can publish branch and SHA-tagged package images;
semver tags publish GitHub Releases and binary archives.

Run the verification loop:

```bash
npm run doctor
npm run verify
cd thymos
cargo test --workspace
```

## Repository Map

| Path | Purpose |
| --- | --- |
| [`thymos`](thymos) | Rust runtime, compiler, ledger, policy engine, tools, server, CLI, worker, clients. |
| [`src`](src) | Next.js operator console. |
| [`docs`](docs) | Specification, architecture, replay, governance, threat model, and demos. |
| [`docs/programmable-capabilities.md`](docs/programmable-capabilities.md) | Capability programming model for Rust contracts, manifests, MCP bridge tools, and sandboxing. |
| [`wiki`](wiki) | GitHub wiki source pages. |
| [`.github/workflows/release.yml`](.github/workflows/release.yml) | Release binaries and GitHub Packages container publication. |
| [`GOVERNANCE.md`](GOVERNANCE.md) | Project authority and decision process. |
| [`RFC_TEMPLATE.md`](RFC_TEMPLATE.md) | Protocol change template. |
| [`ROADMAP.md`](ROADMAP.md) | Long-term runtime roadmap. |

## Long-Term Vision

OpenThymos is built for a future where machine cognition runs inside durable
execution systems instead of ephemeral chat loops.

The long-term target is a federated runtime substrate:

- deterministic local execution
- multi-agent coordination through explicit delegation
- distributed execution ledgers
- replay across runtime boundaries
- portable provider semantics
- capability writs as a shared authority format
- policy engines that can be audited, versioned, and governed
- autonomous governance layers that remain subordinate to recorded protocol
  rules

The project should be understandable decades from now. The goal is not to
maximize surface area. The goal is to define small, durable runtime semantics
for governed cognition.
