---
layout: default
title: Replay
eyebrow: Ledger verification
subtitle: Replay reconstructs runtime state from append-only execution history.
permalink: /replay/
---

# Replay

Replay is the process of proving that a trajectory ledger can be folded into a
valid world projection.

Replay is not a transcript viewer. It is a verifier over structured runtime
records.

## Replay Inputs

Replay consumes ordered ledger entries for a trajectory:

- `Root`
- `Commit`
- `Rejection`
- `PendingApproval`
- `Delegation`
- `Branch`

Only `Commit` entries mutate world projection. Other entries remain part of
the audit history and execution path.

## Verification Procedure

Replay performs these checks:

1. Recompute each entry id from its canonical payload.
2. Verify that every non-root entry points to the previous entry id.
3. Verify contiguous sequence numbers.
4. Apply each committed structured delta in order.
5. Record head commit and head sequence.
6. Record compiler versions seen.
7. Optionally require all commits to match a pinned compiler version.
8. Optionally compare the rebuilt world hash with an observed world hash.

The current Rust implementation exposes this path through `thymos-ledger`
functions:

- `replay(entries, cfg)`
- `replay_and_match(entries, observed, cfg)`
- `ReplayConfig::pinned_to_current()`

## Replay Output

A successful replay returns:

- rebuilt world projection
- trajectory id
- entries seen
- commits replayed
- head commit id
- head sequence
- compiler versions seen

Failure is an invariant violation. Common causes are hash mismatch, parent
mismatch, non-contiguous sequence, compiler version drift, or invalid delta
application.

## Replaying Policy

Replay reports policy-visible ledger outcomes: rejections, pending approvals,
and commit proposal ids. `PendingApproval` entries include the suspended
proposal and its policy trace. Plain committed entries currently store the
proposal id, not the full staged proposal body, so full policy-trace replay for
every committed action is future work.

Replay does not re-run policy against a different policy engine unless an audit
tool is explicitly performing historical policy simulation.

This distinction matters. Verification answers "what happened under the
recorded runtime." Simulation answers "what would happen under another policy
version."

## Replaying Providers

Replay MUST NOT call cognition providers. Provider outputs are not stable
enough to serve as replay input. The replay boundary begins after the provider
has emitted intents and after runtime outcomes have been ledgered.

## Replaying Tools

Replay MUST NOT call tools to rediscover observations. Tool observations and
structured deltas are committed data. The replay engine folds committed deltas.

Tool re-execution is a separate diagnostic mode and should be labeled as such.
It cannot be used as the primary proof of historical state.

## Branch Replay

A branch first folds the source trajectory up to the source commit, then folds
the branch trajectory entries. The branch root records its source trajectory
and source commit. This allows isolated exploration without rewriting the
original trajectory.

## Delegation Replay

Delegation entries link parent and child trajectories. A parent replay can
verify that delegation occurred and identify the child trajectory. A complete
multi-agent replay traverses the execution DAG and verifies child trajectories
under their own ledgers and writs.

## CLI

```bash
thymos replay run_847 --verify --fold-world --policy-trace
```

Expected verifier phases:

```text
load ledger entries
verify content hashes
verify parent chain
verify sequence continuity
fold committed deltas
compare projected world
emit replay report
```

See [demos/deterministic-replay.md](demos/deterministic-replay.md) for a full
terminal walkthrough.
