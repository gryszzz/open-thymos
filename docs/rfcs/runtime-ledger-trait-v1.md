# RFC: Runtime/Ledger trait refactor (v1)

Status: Draft
Tracking: #22 (Phase III — distributed execution ledger)

## Summary

Abstract the ledger behind a trait so the synchronous SQLite backend and the
asynchronous Postgres backend are interchangeable **without changing the
execution protocol or replay determinism**. This unblocks running the HTTP
server on a Postgres-backed production ledger, which the roadmap promises but the
code does not yet deliver (the server prints a note that it "still uses the
synchronous SQLite ledger path").

## Motivation

- `STATUS.md` and the roadmap claim a "production-grade Postgres ledger." Today
  `PostgresLedger` exists and is integrity-tested in isolation, but the runtime
  is hard-wired to `thymos_ledger::Ledger` (= `SqliteLedger`). The claim is
  aspirational until the runtime can actually use it.
- `thymos-runtime`'s `Runtime` holds a concrete `pub ledger: Ledger;` and calls
  it directly. Swapping backends today means a type change, not a config change.

## Goals

- A single trait the runtime depends on instead of a concrete `SqliteLedger`.
- SQLite remains the default and the in-memory path for tests — zero behavior
  change for existing users.
- Postgres becomes a *selectable* HTTP runtime backend.
- Replay stays byte-identical across backends.

## Non-goals

- Rewriting the runtime to be fully async (tracked separately; see Alternatives).
- Distributed/multi-node ledger semantics (Phase III later work: merge entries,
  checkpoints). This RFC only makes the backend swappable on a single node.

## The trait surface (already small and stable)

The runtime + server call exactly these methods today (counts = call sites):

```
entries (17)  append_commit (3)  head (3)  has_trajectory (2)
append_rejection (2)  append_root (1)  append_pending_approval (1)
verify_integrity (1)
```

Plus `append_delegation`, `append_branch_root`, `query_entries`, `count_entries`
exist on both backends and belong in the trait for completeness.

```rust
pub trait LedgerStore {
    fn append_root(&self, trajectory_id: TrajectoryId, note: &str) -> Result<Entry>;
    fn append_commit(&self, commit: Commit) -> Result<Entry>;
    fn append_rejection(&self, /* … */) -> Result<Entry>;
    fn append_pending_approval(&self, /* … */) -> Result<Entry>;
    fn append_delegation(&self, /* … */) -> Result<Entry>;
    fn append_branch_root(&self, /* … */) -> Result<Entry>;
    fn has_trajectory(&self, trajectory_id: TrajectoryId) -> bool;
    fn head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)>;
    fn entries(&self, trajectory_id: TrajectoryId) -> Result<Vec<Entry>>;
    fn verify_integrity(&self, trajectory_id: TrajectoryId) -> Result<()>;
    fn query_entries(&self, /* … */) -> Result<Vec<Entry>>;
    fn count_entries(&self, /* … */) -> Result<u64>;
}
```

**Replay is unaffected.** `replay()`, `replay_and_match()`, `project_commits()`,
and `verify_integrity_entries()` already take `&[Entry]` — they are
storage-independent today. The trait only governs *how entries are read/written*,
not how the world is folded. This is the property that makes the refactor safe.

## The crux: sync runtime vs async Postgres

`SqliteLedger` is synchronous; `PostgresLedger` is `async`. `Runtime`/`Run` are
synchronous and borrow `&Runtime`. There are three ways to bridge:

**Option A (recommended): sync `LedgerStore` trait; Postgres gets a blocking
facade.** Keep the runtime synchronous. Implement `LedgerStore` directly for
SQLite. For Postgres, wrap the async pool in an adapter that drives futures to
completion on a dedicated Tokio runtime handle (`Handle::block_on` / a current-
thread runtime owned by the adapter). The server already calls the sync runtime
from async handlers via the blocking path, so this composes.
- *Pros:* smallest blast radius; runtime, compiler, replay, and every test stay
  unchanged; ships Phase III value now.
- *Cons:* a blocking hop per ledger op under Postgres (acceptable — ledger ops
  are not the hot loop; cognition latency dominates). Must avoid running the
  adapter's `block_on` *inside* an async executor thread (use a dedicated
  runtime/thread, not the request executor).

**Option B: async `LedgerStore` (via `async-trait`); async-ify the runtime.**
Cleanest long-term, but turns `Run`/`Runtime`/agent-loop and all call sites
async — a large, higher-risk change touching the authority boundary. Defer.

**Option C: two traits (sync + async) with a bridge.** More surface area, more
ways to drift. Rejected.

Recommendation: **Option A now**, with the trait shaped so a future move to
Option B is mechanical (method names/semantics identical).

## Migration plan (small, reviewable PRs)

1. Define `LedgerStore` in `thymos-ledger`; impl for `SqliteLedger`. No runtime
   change yet. (Pure addition.)
2. Change `Runtime` to hold `L: LedgerStore` (generic) **or** `Box<dyn
   LedgerStore>`. Prefer a generic to avoid dynamic dispatch in the hot path;
   fall back to `dyn` if the generic bound proves viral. Default stays SQLite —
   all existing tests pass untouched.
3. Implement the Postgres blocking facade + `LedgerStore` for it (feature
   `postgres`).
4. Server: select backend from config (`THYMOS_LEDGER_BACKEND` /
   `THYMOS_POSTGRES_URL`); remove the "still uses SQLite" note.
5. Backend-independence test: drive the same trajectory through both backends and
   assert `replay()` yields an identical world projection and the same
   content-addressed head.

## Test plan

- Existing workspace suite must pass unchanged after steps 1–2 (proves zero
  behavior change for SQLite).
- New gated test (extends `postgres_integration`): run an identical scripted
  trajectory on SQLite and Postgres; assert equal head hash + equal replayed
  world. Gated on `THYMOS_TEST_POSTGRES_URL`.
- CI keeps the compile guard for `--features postgres`.

## Risks

- **`block_on` misuse** (Option A): calling it on an async executor thread panics
  or deadlocks. Mitigation: the Postgres adapter owns its own runtime; document
  and test the boundary.
- **Generic bound virality**: `L: LedgerStore` may spread through many signatures.
  Mitigation: if it gets noisy, use `Box<dyn LedgerStore>` (ledger ops aren't hot
  enough for dynamic dispatch to matter).
- **Silent semantic drift between backends**: the backend-independence test is the
  guard; it must be part of the gated Postgres CI job once secrets exist.

## Decision needed

Confirm Option A (sync trait + Postgres blocking facade) before implementation,
and whether `Runtime` should be generic over `L: LedgerStore` or hold `Box<dyn
LedgerStore>`.
