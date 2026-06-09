# RFC: Incremental projection cache (submit hot path)

## Status

Draft → implementing.

## Summary

A runtime-internal optimization. The agent's hot path (`Run::submit_with_trace`)
re-reads the **entire trajectory ledger** on every intent and re-folds the
`World` + accumulated budget from scratch — O(n) per submit, O(n²) over a run.
This RFC memoizes that projection on the live `Run`, validated by the ledger
head sequence, so a submit becomes a cheap `head()` check + an incremental delta
apply instead of a full read-and-fold.

**This does not affect runtime semantics, ledger format, replay, writs, policy,
or the authority boundary.** It changes *how* the world/budget handed to the
compiler are computed, not *what* they are — the values are byte-identical to a
from-scratch projection.

## Current semantics

Per submit: `ledger.entries(traj)` (full read) → `project_world_from` (fold all
commits) → `project_budget_used_from` (sum all) → idempotency scan over all
entries. The compiler then receives `(world, budget_used)`.

## Proposed semantics

`Run` holds a memoized `ProjCache { seq, world, budget, proposals }`:

- **Validation token = ledger head seq.** The ledger is append-only with a
  unique `(trajectory, seq)` invariant, so two reads at the same head seq are
  the *same* projection. `head()` is an O(1) indexed lookup on the `heads`
  table.
- **`sync_to_head()`** (called at submit start, and by `project_world`): if
  `cache.seq == head.seq`, the cache is current — use it. Otherwise re-fold from
  the ledger (the source of truth) and record the new seq. First submit and any
  out-of-band append (resume, compensation) take this path.
- **On the run's own commit:** apply the just-committed delta to the cached
  world, add its budget cost, index `proposal_id → commit_id`, and bump
  `cache.seq` — so the *next* submit's head check matches and skips the read.
- **Idempotency** reads `cache.proposals.get(proposal_id)` instead of scanning.

Net: the common case (a run is the sole writer of its trajectory) does **zero**
full reads after the first — `head()` + an O(world) clone per submit. O(n²) → O(n)
over a run.

## Invariants

- The world + budget the compiler receives are identical to a from-scratch fold
  of the same ledger at the same head seq. (Append-only + unique `(traj,seq)`
  ⇒ head seq is a sound cache-version token.)
- A cache that cannot be advanced incrementally (seq gap, apply error) is
  **invalidated**, forcing a full re-fold next submit. The cache never serves a
  stale or partial projection.
- **Replay is untouched** — it folds the ledger independently and never consults
  a live `Run`'s cache. The ledger remains the single source of truth.

## Ledger / replay / writ / policy / provider / tool impact

None. No new ledger entries, no schema or trait change, no replay change, no
authority change. `entries()` and the existing projection helpers remain for the
non-hot paths (resume, compensation, public `project_budget_used`).

## Test plan

Existing suite is the guard (must stay green): `replay_safety`, `agent_loop`,
`idempotency`, `compensation`(+gate, +cross-trajectory), `cognition_budget`,
`revocation`, `json_policy_e2e`. Replay independently re-folds and must match the
cache-driven run. Add a regression asserting a multi-commit run's cache-driven
world equals a fresh `project_world` and that replay verifies.

## Alternatives

- *`entries_since(seq)` ledger API + cache* — avoids even the fallback full read,
  but needs a trait change across SQLite/Postgres. Deferred; the head-seq cache
  already removes the per-submit full read in the common case with no ledger
  change.
- *Do nothing* — fine for short runs; the O(n²) only bites long/high-volume
  trajectories (the enterprise/scale path).
