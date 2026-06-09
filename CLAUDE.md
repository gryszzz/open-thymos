# Working on OpenThymos

Guidance for any agent (or human) contributing to this repo. The goal is a
runtime whose claims are **provable, demonstrable, and documented** — never
vaporware.

## The one rule everything serves

**Cognition proposes. The runtime governs. The ledger records.**

A model cannot call a tool, mutate state, spend budget, delegate authority, or
erase history — not by convention, by runtime semantics. Effects pass through a
typed `Intent → Proposal → Commit` pipeline bound to a signed writ, a policy
trace, and an append-only ledger.

Before any change, ask: *does this strengthen that boundary, or blur it?* If it
blurs it (e.g. lets cognition assert authority inline, mutates projected state
outside commit folding, or makes replay depend on a live provider), don't.

## Architecture map

| Crate | Role |
|---|---|
| `thymos-core` | Types: Intent, Proposal, Commit, Writ, World, deltas, ids (blake3 content-addressing) |
| `thymos-ledger` | Append-only hash-chained ledger; SQLite (default) + Postgres backends; replay |
| `thymos-compiler` | Compiles intents → proposals; enforces writ scope, effect ceiling, budget, time windows |
| `thymos-policy` | Policy engine + traces (first-class proposal data) |
| `thymos-tools` | Typed tool contracts; sandboxed shell/HTTP/fs fabric; `worker_entrypoint` |
| `thymos-worker` | Thin binary = process-isolation boundary for tool execution (substance is in `thymos-tools`) |
| `thymos-cognition` | Provider adapters (Anthropic, OpenAI, Mock) — emit intents only, no execution authority |
| `thymos-runtime` | Drives the agent loop; owns runs, trajectories, budget projection, revocation |
| `thymos-server` | HTTP surface (`/runs`, `/routed-submit`, `/routing-outcomes`, `/health`) |
| `thymos-marketplace`, `thymos-cli`, `thymos-client` | Tool marketplace, CLI, Rust client |

## Roadmap (what "aligned with our future" means)

- **Phase I — Unified deterministic runtime.** Largely done: I→P→C, ledger as
  truth, replay, signed writs, typed tools, policy traces.
- **Phase II — Multi-agent coordination.** Delegation, child writs (strict
  subsets of parent), child trajectories — implemented + tested, with a
  demonstrable walkthrough (`cargo run --example delegation_lineage -p
  thymos-runtime`, `docs/demos/delegation-lineage.md`). The same strict-subset
  rule now also governs authority-narrowing **skills** (`skill_narrowing`
  example + `docs/demos/skill-narrowing.md`).
- **Phase III — Distributed execution ledger.** The Postgres backend **is wired
  into the HTTP runtime** (feature-gated): `Runtime<L: LedgerStore>` with a
  `BlockingPostgresLedger` facade, selected at startup via `THYMOS_POSTGRES_URL`
  (default build stays SQLite, no Postgres dep compiled). It is **continuously
  proven in CI** — the `postgres-integration` job in `rust.yml` runs the gated
  tests against a real Postgres service container (append → read-back →
  `verify_integrity` → replay). Remaining Phase-III work is operational hardening
  (multi-writer/distributed semantics), not the basic wiring.

Source of truth for current reality: **`STATUS.md`** (CI-proven vs gated vs
caveats). Reconcile it every release; if it drifts from the code, the code wins.

## Non-negotiable invariants (don't regress these)

- No tool execution without a staged, authorized proposal.
- No projected-state mutation outside commit folding.
- No provider-specific execution authority; replay never calls a provider or
  re-runs a tool.
- Ledger entries are content-addressed (`id = blake3(canonical_json(payload))`);
  replay verifies sequence continuity + parent linkage.
- **No phone-home / no silent egress.** Telemetry is operator-owned (OTLP to the
  operator's own endpoint, else stderr). This is a core value prop — never add
  analytics or usage beacons. If usage signal is ever wanted, it must be
  opt-in, off by default, and documented.

## How we work (pro hygiene)

- **Small, single-purpose PRs** with honest descriptions. State explicitly what
  was *not* verified (e.g. "compiles + gates correctly, not run against a live
  resource here"). Candor is a credibility signal.
- **Gated tests** for anything needing external resources: `#[ignore]` and/or a
  feature/env gate; skip cleanly (print `SKIP`, pass) when the resource is
  absent. Never fake a green.
  - Live LLM: `ANTHROPIC_API_KEY` → `thymos-runtime` test `live_provider`.
  - Postgres: `THYMOS_TEST_POSTGRES_URL` → `thymos-ledger --features postgres`
    test `postgres_integration`.
- **Big/architectural changes get an RFC first** (`docs/rfcs/`, see
  `RFC_TEMPLATE.md`) before code — especially anything touching the ledger,
  replay, or the authority boundary.
- **Release discipline:** tag every version `vX.Y.Z` so "pin to vX" is always
  real. Tags trigger `.github/workflows/release.yml` (binaries + GHCR + GitHub
  Release). Note: tag pushes require a credential that can write `refs/tags/*`.

## Build / test / verify

```bash
cd thymos
cargo build --workspace
cargo test  --workspace                 # default: mock cognition, SQLite, governance
cargo clippy --workspace --all-targets
# gated proofs (need resources):
ANTHROPIC_API_KEY=…       cargo test -p thymos-runtime --test live_provider -- --ignored --nocapture
THYMOS_TEST_POSTGRES_URL=… cargo test -p thymos-ledger --features postgres --test postgres_integration -- --ignored
```

A change isn't "done" until: workspace tests pass, clippy is clean, `STATUS.md`
still matches reality, and the PR description is honest about what was verified.
