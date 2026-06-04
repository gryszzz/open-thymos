# OpenThymos RFC

## Title

Ledger Backend Trait — make SQLite and Postgres interchangeable on the runtime path

## Status

Draft — design for review. No implementation is authorized by this document; it
exists so the trait surface and the sync/async boundary are agreed *before* code
touches the authority/replay core. (Per the project standard: RFC before code for
core-touching changes.)

## Summary

The Postgres ledger backend exists and is tested in isolation (gated
`postgres_integration`, CI compile guard), but the **HTTP runtime still uses the
synchronous SQLite path**. So "production Postgres ledger" is aspirational. This
RFC abstracts the ledger behind a trait so the synchronous SQLite backend and the
asynchronous Postgres backend are interchangeable *without changing the execution
protocol or replay determinism*, and specifies how the synchronous runtime calls
the asynchronous backend safely.

## Problem

- `Runtime` / `Run` are synchronous: `submit`, `project_world`, `compensate_to`,
  etc. call `self.runtime.ledger.entries(..)`, `.append_commit(..)`, `.head(..)`
  directly and synchronously.
- `thymos_ledger::SqliteLedger` is synchronous (rusqlite).
- `thymos_ledger::postgres::PostgresLedger` is **async** (`async fn append_commit`,
  tokio-postgres).
- The default `Ledger` is a type alias to `SqliteLedger`. There is no abstraction
  point, so the server cannot select Postgres at runtime.

The hard part is therefore not the abstraction — it is calling async Postgres
from the synchronous runtime, which itself runs inside the server's async (axum)
task tree.

## Goals / Non-Goals

Goals:
- One trait the runtime depends on; SQLite and Postgres both implement it.
- The server selects the backend at startup (e.g. `THYMOS_POSTGRES_URL`).
- **Byte-identical hash chain and replay** across backends — determinism is the
  invariant that must not move.

Non-goals:
- Rewriting the runtime to be `async` end-to-end. That is a larger change with
  its own RFC; this proposal keeps the synchronous execution path.
- Changing the proposal/commit/replay wire formats.

## The trait

The trait is exactly the surface `Runtime` / `Run` use today (all synchronous):

```rust
pub trait LedgerBackend: Send + Sync {
    fn append_root(&self, traj: TrajectoryId, note: &str) -> Result<Entry>;
    fn append_branch_root(&self, new: TrajectoryId, src: TrajectoryId, src_commit: CommitId, note: &str) -> Result<Entry>;
    fn append_commit(&self, commit: Commit) -> Result<Entry>;
    fn append_rejection(&self, traj: TrajectoryId, intent_id: IntentId, reason: RejectionReason) -> Result<Entry>;
    fn append_pending_approval(&self, traj: TrajectoryId, proposal: Proposal, channel: String, reason: String) -> Result<Entry>;
    fn append_delegation(&self, traj: TrajectoryId, child: TrajectoryId, task: &str, final_answer: Option<String>) -> Result<Entry>;
    fn entries(&self, traj: TrajectoryId) -> Result<Vec<Entry>>;
    fn head(&self, traj: TrajectoryId) -> Result<(ContentHash, u64)>;
    fn has_trajectory(&self, traj: TrajectoryId) -> bool;
    fn verify_integrity(&self, traj: TrajectoryId) -> Result<()>;
    // audit/query surface used by the server:
    fn query_entries(&self, /* filters */) -> Result<Vec<AuditEntry>>;
    fn count_entries(&self, /* filters */) -> Result<u64>;
}
```

`SqliteLedger` implements it directly (it already has every method — a pure
refactor, no logic change). `Runtime` holds `Arc<dyn LedgerBackend>` (or stays
generic `<L: LedgerBackend>` to avoid dynamic dispatch on the hot path — to be
decided in review; `Arc<dyn>` is simpler for the server's runtime-selected
backend).

## The sync ⇄ async boundary (the crux)

`PostgresLedger`'s methods are `async`. The trait above is synchronous. The
Postgres `impl LedgerBackend` must bridge them. Options:

- **(A) Blocking facade over async (recommended).** The Postgres backend owns a
  handle to a multi-thread tokio runtime (or the server's) and runs each method
  with `tokio::task::block_in_place(|| handle.block_on(async { ... }))`.
  `block_in_place` is required because the runtime's synchronous `submit` is
  itself invoked from within the server's async task tree; calling `block_on`
  on a worker thread without it would panic/deadlock.
  - Pros: no async rewrite of the runtime; localized to the Postgres impl.
  - Cons: `block_in_place` only works on the multi-thread scheduler; a dedicated
    blocking thread-pool or a separate "ledger runtime" may be cleaner and must
    be specified. Connection pooling (deadpool/bb8) is required so blocking calls
    don't serialize on one connection.

- **(B) Make the runtime async.** Larger, separate RFC. Out of scope here.

This RFC adopts **(A)** and requires the design to pin down: which runtime handle
the Postgres backend uses, whether a dedicated blocking pool is used, and the
connection-pool sizing, before implementation.

## Invariants that MUST hold identically across backends

These are the authority/replay-core properties; the refactor is only acceptable
if every one is preserved on Postgres:

1. **Identical content addressing.** `id = blake3(canonical_json(payload))` for
   every entry; the root entry binds `trajectory_id` (already fixed). Replaying a
   trajectory produces the *same* world hash on SQLite and Postgres.
2. **Append-only.** No updates to entry rows.
3. **Fork-proof append.** SQLite uses a `UNIQUE(trajectory_id, seq)` index inside
   an `IMMEDIATE` transaction. Postgres MUST get the equivalent: a unique
   constraint on `(trajectory_id, seq)` **and** a serializable / `SELECT … FOR
   UPDATE` head read so two writers cannot both land `seq = n+1`.
4. **Atomic head advance.** The head read + entry insert + head update must be one
   transaction (as in the SQLite `IMMEDIATE` path).
5. **No determinism inputs from the backend.** Timestamps (`created_at`) are
   metadata only and never enter a hash; this must remain true on Postgres.

## Backend-independence test (acceptance)

A gated test drives the *same* sequence of governed actions against both
backends and asserts:
- identical entry ids and identical final world hash;
- `verify_integrity` passes on both;
- a concurrent double-append at the same seq is rejected on both (one winner).

This is the proof that "production Postgres ledger" is real, not aspirational.

## Sequenced plan (small, single-purpose PRs)

1. Define `LedgerBackend` + `impl` for `SqliteLedger` (pure refactor, zero
   behavior change; all existing tests green).
2. `Runtime` depends on the trait (`Arc<dyn LedgerBackend>`); construction
   unchanged for SQLite callers.
3. Postgres `impl LedgerBackend` via the blocking facade (A) + connection pool +
   the `(trajectory_id, seq)` unique constraint and transactional head advance.
4. Server selects the backend from `THYMOS_POSTGRES_URL`; remove the "still uses
   SQLite" startup note; `/health` reports the active ledger backend.
5. Backend-independence test (above), run in CI against an ephemeral Postgres
   service (gated, like `postgres_integration`).

## Risks

- **Blocking-in-async deadlock** if the facade is wrong (single-threaded runtime,
  missing `block_in_place`, or pool starvation). Mitigated by a dedicated pool +
  multi-thread handle, validated by load in the independence test.
- **Isolation mismatch.** If Postgres head-advance isn't serialized like SQLite's
  `IMMEDIATE` tx, concurrent submits could fork a chain. The unique constraint is
  the backstop; the transaction is the primary guard. Both are required.
- **Performance.** Blocking calls hold a worker; pool sizing and (later) batching
  matter. Out of scope to optimize here, but must not regress correctness.

## Unresolved Questions

- `Arc<dyn LedgerBackend>` vs generic `Runtime<L>` — dynamic dispatch on the hot
  read path vs. monomorphization + a server that can't choose at runtime.
- Dedicated "ledger runtime"/blocking pool vs. reusing the server's runtime.
- Whether to also expose async-native methods for the (future) async runtime so
  this trait isn't a dead-end for RFC (B).

## References

- `STATUS.md` — "Postgres is not yet the HTTP runtime path."
- `thymos-ledger::sqlite` (the synchronous reference impl + fork-proof append).
- `thymos-ledger::postgres` (the async backend + gated `postgres_integration`).
- Issue #22, Phase III.
