---
layout: default
title: Execution Ledger
eyebrow: Append-only history
subtitle: The ledger records the execution path that replay and audit consume.
permalink: /execution-ledger/
---

# Execution Ledger

The execution ledger is the authoritative history of a trajectory. It records
what the runtime accepted, rejected, suspended, delegated, branched, and
committed.

## Entry Shape

A ledger entry contains:

- `id`
- `trajectory_id`
- `parent`
- `seq`
- `kind`
- `payload`

The id is the content hash of the payload. The parent is the previous entry id.
The sequence number is monotonically increasing and contiguous.

## Entry Kinds

| Kind | Meaning |
| --- | --- |
| `root` | Begins a trajectory. |
| `commit` | Records a committed structured delta and observation. |
| `rejection` | Records a typed compiler or policy rejection. |
| `pending_approval` | Records a suspended proposal awaiting operator decision. |
| `delegation` | Links a parent trajectory to a child trajectory. |
| `branch` | Begins a trajectory from an existing source commit. |

## Append Discipline

Ledger entries are append-only. Prior entries must not be modified. Any state
that appears to require mutation should be represented as a new entry.

This rule allows replay, audit, and forensic review to share one source of
truth.

## Hash Chain

For each entry:

```text
entry.id = hash(canonical_json(entry.payload))
entry.parent = previous_entry.id
entry.seq = previous_entry.seq + 1
```

Replay recomputes these values. A mismatch invalidates the trajectory.

## Commit Payloads

A commit payload contains:

- proposal id
- writ id
- trajectory id
- parent commit
- sequence number
- structured delta
- observations
- compiler version
- budget cost
- optional signature

Commit payloads are sufficient for deterministic world folding.

## Rejections

Rejections are ledger entries because they are runtime facts. They explain why
an intent did not become an effect. Rejection reasons include invalid authority,
policy denial, budget exhaustion, precondition failure, unknown tool, and type
mismatch.

## Pending Approvals

A pending approval records the exact proposal, approval channel, and reason.
This makes human approval a replay-visible part of execution rather than a UI
side effect.

## Branches

A branch root records:

- source trajectory id
- source commit id
- note

World projection for a branch first folds the source trajectory up to the
source commit, then folds branch-local commits.

## Delegations

A delegation entry records:

- child trajectory id
- delegated task
- optional child final answer

Delegation turns multi-agent execution into an explicit graph of trajectories.

## Storage Backends

The ledger abstraction supports SQLite and Postgres feature paths. Storage
backends must preserve the same entry semantics. Replay results must not
depend on backend choice.
