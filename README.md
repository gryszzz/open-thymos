<div align="center">
  
<div align="center">
  <img src="./thymos/thymosG.png" alt="OpenThymos" width="325" />
</div>

# open-thymos

</div>

---

**Cognition proposes. The runtime governs. The ledger records.**

A model cannot call a tool, mutate state, spend budget, delegate authority, or erase history  not by convention, by runtime semantics. Every effect passes through a typed proposal, a signed capability writ, a policy trace, and an append-only execution ledger.

```text
Intent → Proposal → Commit
```

| Stage | Type | Authority |
|-------|------|-----------|
| `Intent` | Emitted by cognition | None — content-addressed, no execution rights |
| `Proposal` | Compiled by the runtime | Bound to a signed `Writ` + policy trace |
| `Commit` | Written to the ledger | The only record that mutates world state |

<p align="center">
  <a href="https://gryszzz.github.io/open-thymos/specification/">Specification</a>
  &nbsp;·&nbsp;
  <a href="docs/architecture.md">Architecture</a>
  &nbsp;·&nbsp;
  <a href="https://gryszzz.github.io/open-thymos/replay/">Replay</a>
  &nbsp;·&nbsp;
  <a href="https://gryszzz.github.io/open-thymos/capability-writs/">Capability Writs</a>
  &nbsp;·&nbsp;
  <a href="docs/rfcs/">RFCs</a>
</p>

---

## The Threat Model

OpenThymos treats cognition as **untrusted input**. The runtime enforces this structurally:

- Cognition emits intents — it cannot execute
- Authority is carried by signed writs — it cannot be asserted inline
- State is a projection of committed ledger deltas — it cannot be mutated in place
- Every rejection, approval, and delegation is a ledger event — it cannot be erased
- Replay proves the world projection from the commit sequence — it cannot call providers or re-run tools

If a proposal reaches the tool boundary, it was already authorized by a writ, cleared by the compiler, and permitted by the policy engine. The ledger records everything that happened and everything that was refused.

## Execution Model

The compiler is a pure function:

```
(Intent, Writ, World, ToolRegistry, PolicyEngine) → Proposal
```

A proposal is one of three outcomes:

- **Staged** — authority, budget, time window, scope, and policy all passed. Reaches the tool boundary.
- **Suspended** — policy returned `RequireApproval { channel, reason }`. Written to the ledger as `PendingApproval`.
- **Rejected** — writ check, budget, scope, or policy `Deny` failed. Written to the ledger as `Rejection`.

Only a `Staged` proposal executes. Only a `Commit` mutates world state.

## Five Runtime Guarantees

These are invariants, not goals. They are checked structurally by the runtime, recorded in the ledger, and verifiable by replay.

| | Guarantee |
|--|-----------|
| **I** | A valid ledger can be folded into the same world projection under the recorded commit sequence. |
| **II** | Cognition cannot execute tools or mutate state directly. The provider boundary is enforced at the type level. |
| **III** | Only staged proposals may reach the tool boundary. Only commits may mutate projected world state. |
| **IV** | Tool scopes, budgets, time windows, effect ceilings, tenant boundaries, and delegation bounds are checked before execution. |
| **V** | Policy decisions are recorded as proposal traces and cannot be erased by a client surface. |

## Capability Writs

Authority is carried by ed25519-signed capability writs. A writ declares:

- who issued the authority (issuer pubkey)
- which subject may act (subject pubkey)
- which tools are in scope (glob patterns)
- which effects are allowed (effect ceiling)
- how much budget is available (tokens, tool calls, wall clock, USD millicents)
- when the authority is valid (not_before, expires_at)
- whether the subject may subdivide authority (delegation bounds)

Child writs must be strict subsets of parent writs. Cross-tenant delegation is forbidden. Provider identity grants no authority.

## Replay

Replay is a proof procedure over the execution ledger:

```bash
thymos replay run_847 --verify --fold-world --policy-trace
```

The verifier walks every ledger entry, recomputes hashes, checks the parent chain, verifies sequence continuity, re-applies committed deltas in order, and reports the compiler versions seen. It cannot call providers. It cannot execute tools. It cannot mutate state.

```bash
cargo test -p thymos-ledger --features sqlite bench -- --include-ignored --nocapture
```

Phase I baseline (macOS arm64, SQLite in-memory, mock provider, 1 root + 1000 commits):

```text
replay_speed   ~12,400 entries/sec   (hash verify + parent chain + world fold)
ledger_folding ~656,000 commits/sec  (delta application only)
exec_overhead  ~1.35 ms/proposal     (compile + policy + tool execute + ledger append)
```

## Workspace

The runtime is implemented as a Rust workspace under [`thymos/`](thymos):

| Crate | Responsibility |
|-------|----------------|
| `thymos-core` | Intent, Proposal, Commit, Writ, World, structured deltas |
| `thymos-compiler` | Pure proposal compilation — writ check, budget, scope, policy, type |
| `thymos-policy` | Policy evaluation, `PolicyDecision`, `PolicyTrace` |
| `thymos-ledger` | Append-only entries, BLAKE3 hash chain, replay verifier |
| `thymos-runtime` | IPC cycle, approvals, delegation, projection, resume |
| `thymos-cognition` | Provider abstraction — emits intents, no authority |
| `thymos-tools` | Rust tool contracts, JSON manifests, MCP bridges, observed effects |
| `thymos-server` | HTTP runtime server — sessions, approvals, SSE streams |
| `thymos-cli` | Terminal access to the runtime — `thymos replay`, `thymos run` |

## Quick Start

```bash
cd thymos
cargo test --workspace --features sqlite
```

```bash
cargo run -p thymos-server
```

```bash
# Follow a run from terminal
thymos run "summarize the open issues" --writ ./writs/dev.json
thymos replay run_847 --verify
```

## Repository

| Path | Purpose |
|------|---------|
| [`thymos/`](thymos) | Rust workspace — runtime, compiler, ledger, policy, tools, server, CLI |
| [`docs/`](docs) | Specification, architecture, replay, capability writs, invariants |
| [`docs/rfcs/`](docs/rfcs) | Accepted RFCs for protocol-level changes |
| [`docs/benchmarks.md`](docs/benchmarks.md) | Benchmark matrix, reporting format, Phase I baseline |
| [`GOVERNANCE.md`](GOVERNANCE.md) | Project authority and decision process |
| [`RFC_TEMPLATE.md`](RFC_TEMPLATE.md) | Protocol change template |

## Design Philosophy

OpenThymos is not a model wrapper. It is an execution substrate with durable runtime semantics.

The existing agent ecosystem collapses cognition and execution into one loop  a model chooses a tool, calls it, reads the result, and continues. That design is easy to demo and hard to govern. Tool calls happen before authority is modeled. Policy is applied as application code. State is reconstructed from logs after the fact, if at all.

OpenThymos separates intent from authority, authority from compilation, and compilation from execution. None of these boundaries are optional.

The goal is not to maximize surface area. The goal is to define small, durable runtime semantics for governed cognition  semantics that remain legible decades from now.

<img width="1402" height="1122" alt="0700A45D-0DDB-4919-B931-23FCAC999AAA" src="https://github.com/user-attachments/assets/132526e7-a94b-47fe-b80b-9dc72c88e9a2" />

