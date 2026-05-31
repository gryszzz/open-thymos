# OpenThymos RFC

## Title

Compensation & Saga Rollback v1

## Status

Accepted — MVP implemented (Option A). Shipped:

- `ToolContract::compensable()` + `ToolContract::compensate(observation, world)`.
- `CommitBody.compensates: Option<CommitId>` (backward-compatible via
  `skip_serializing_if`) tagging each rollback commit with the commit it undoes.
- `Run::compensate_to(target, writ)`: undoes committed steps after `target`,
  newest-first; each compensation is appended as a normal commit (recorded,
  replayable); halts if any step's tool is not compensable; idempotent
  (already-compensated steps and compensation commits are skipped); runs under
  the supplied writ (authorizes the tool, must not be revoked).

Deferred (see Unresolved Questions): declaring `compensable` in
`ToolContractMeta` so the effect gate can *require approval* for an
irreversible-and-uncompensable tool; cross-trajectory/delegated-child
compensation; compensating past an expired writ window; partial-failure policy
beyond "halt and surface".

## Summary

Idempotency (shipped) guarantees an External/Irreversible tool runs *at most
once* per proposal. It does **not** provide a way to *undo* an effect that
already happened, nor to roll back a multi-step trajectory when a later step
fails. This RFC explores **compensation** — tool-declared "undo" actions the
runtime can invoke in reverse order to unwind a partially-completed sequence,
i.e. the saga pattern — while preserving the ledger and replay guarantees.

## Motivation

Real authority means irreversible acts: a payment, a deploy, a resource
provision. When step 3 of a 5-step trajectory fails, steps 1–2 may have already
taken external effect. Today the runtime has no way to unwind them; an operator
must reconcile by hand. A governed execution kernel should be able to:

- record, per tool, whether and how an effect can be compensated;
- on a declared failure boundary, invoke the compensations for the
  already-committed steps in reverse order;
- record each compensation as a first-class ledger event, so the rollback is
  itself auditable and replayable;
- refuse, up front, to execute an irreversible-and-uncompensable effect without
  elevated approval (ties into the multi-party quorum already shipped).

## Non-Goals

- Distributed two-phase commit across external systems. Compensation is
  best-effort forward/backward recovery, not atomic global commit.
- Automatic compensation of *every* effect. Some effects are genuinely
  irreversible (an email was sent); those must be declared uncompensable and
  gated by approval instead.

## Options

**Option A — Tool-declared `compensate`, runtime-orchestrated saga.**
Add `ToolContract::compensate(&self, original_args, observation) -> Result<ToolOutcome>`
(default: `Unsupported`). The runtime tracks the committed, compensable steps of
a trajectory; on a failure boundary it invokes each step's `compensate` in
reverse, recording every compensation as a `Commit` (or a new `Compensation`
ledger entry).
- Pros: compensations are ordinary tools (deltas + observations), so they stay
  deterministic and replayable; the rollback is fully auditable.
- Cons: requires a clear "failure boundary" definition and careful ordering;
  tool authors must write correct, idempotent compensators.

**Option B — Forward-recovery only.**
No rollback; rely on idempotent retry to drive the trajectory to completion.
- Pros: simplest; no new surface.
- Cons: cannot undo a committed irreversible effect when the goal is abandoned.

**Option C — External saga coordinator.**
Delegate compensation to an outside orchestrator.
- Pros: offloads complexity.
- Cons: moves authority and audit outside the kernel — against the project's
  thesis that governance is structural.

## Recommendation (for review)

**Option A**, staged:

1. Tools declare compensability in `ToolContractMeta` (e.g. `compensable: bool`)
   and implement `compensate` when true. The compiler's effect gate can then
   *require approval/quorum* for an `Irreversible` tool that is **not**
   compensable — connecting this RFC to the existing effect ceiling + quorum.
2. The runtime records the ordered list of compensable committed steps per
   trajectory (derivable from the ledger: commits whose tool is compensable).
3. A new `Run::compensate_to(commit_id)` (or `rollback`) invokes the
   compensators for steps after `commit_id` in reverse, each recorded as a
   ledger entry, under the **same writ authority** as the original step.
4. Compensations are idempotent and themselves subject to the idempotency guard.

Defer cross-trajectory / delegated-child compensation to a follow-up.

## Determinism & Replay Constraints (binding)

- A compensation is an ordinary tool invocation: it produces a structured delta
  and an observation, and is recorded as a ledger entry. Replay folds it like
  any other commit — no special-casing, no re-execution.
- Compensators must be deterministic over `(original_args, observation, world)`
  and must not read ambient state (same rule as all tools).
- Compensation MUST NOT exceed the authority of the writ that authorized the
  original step (no privilege escalation via rollback).

## Security Considerations

- A rollback is an action; it passes the full compiler pipeline (writ,
  effect ceiling, budget, policy) under the original writ. It cannot do anything
  the forward step could not.
- Uncompensable irreversible effects are the dangerous case: they should be
  gated by quorum approval *before* execution, never silently performed.
- Compensation entries are signed/anchored like any other commit, so the
  rollback is tamper-evident.

## Unresolved Questions

- Failure-boundary semantics: per-step automatic, or operator-triggered
  `rollback`? (Recommend operator/coordinator-triggered for v1.)
- Partial compensation: what if a compensator itself fails? (Surface, halt,
  require manual reconciliation — do not loop.)
- Representation: reuse `Commit` with a `compensates: Option<CommitId>` field, or
  add a dedicated `Compensation` entry kind? (Wire-format decision; pre-v1.)
- Interaction with delegation: can a parent compensate a child trajectory's
  committed steps? (Defer.)
- Time/window: can a compensation run after the original writ's window expired?
  (Likely yes, under a dedicated compensation grant — needs design.)

## References

- Idempotency guard (`Run::find_commit_for_proposal`; exactly-once for
  External/Irreversible tools).
- Effect ceiling enforcement (compiler stage 5b) and multi-party quorum
  (`Run::resume_with_quorum`) — the gates an uncompensable irreversible effect
  should pass through.
- `proposal-contract-v1.md` (determinism / fixed-point rules) and signed commits
  (`Commit::new_signed`) for the audit/replay pattern compensations must follow.
