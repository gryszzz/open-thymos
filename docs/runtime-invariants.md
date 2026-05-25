---
layout: default
title: Runtime Invariants
eyebrow: Formal guarantees
subtitle: Invariants that define acceptable OpenThymos runtime behavior.
permalink: /runtime-invariants/
---

# Runtime Invariants

This document states the runtime guarantees OpenThymos is designed to preserve.

## Deterministic Replay Guarantee

Given a valid ordered ledger for a trajectory, replay MUST reconstruct the same
world projection by applying committed deltas in sequence order. Replay MUST
reject hash mismatch, parent mismatch, non-contiguous sequence, invalid delta
application, and pinned compiler version drift.

## Runtime Isolation Guarantee

Cognition MUST NOT execute tools, mutate world state, append ledger entries, or
grant authority. Cognition may emit intents. All effects must pass through the
runtime.

## Execution Integrity Guarantee

The runtime MUST execute only staged proposals or approved suspended proposals.
The runtime MUST NOT execute a rejected intent. The runtime MUST NOT commit a
tool result until postconditions have passed and the delta can be applied to a
trial world projection.

## Capability Constraint Guarantee

A writ MUST bound execution by tool scope, budget, time window, tenant id,
effect ceiling, and delegation bounds. A child writ MUST be a strict subset of
its parent. A child writ MUST NOT cross tenant boundaries.

## Policy Enforcement Guarantee

The policy engine MUST evaluate before execution. A proposal MUST include a
policy trace. A `Deny` decision MUST prevent execution. A `RequireApproval`
decision MUST suspend execution and be represented as a pending approval in the
ledger.

## Provider Abstraction Guarantee

A provider MAY influence which intents are proposed. A provider MUST NOT alter
compiler ordering, policy evaluation, writ validation, tool execution rules,
commit construction, ledger append semantics, or replay behavior.

## Auditability Guarantee

Runtime-significant outcomes MUST be inspectable as structured data. The audit
surface MUST expose enough information to identify trajectory id, entry kind,
sequence, payload, and creation time.

## Trace Persistence Guarantee

Execution traces MAY be projected into operator sessions, event streams, and
logs. These traces MUST NOT replace the ledger as source of truth. A persisted
run must be restorable from ledger-backed runtime state.

## Execution Reproducibility Guarantee

Reproduction of historical state MUST depend on ledgered data, not on fresh
provider calls or fresh tool calls. Diagnostic re-execution MAY exist, but it
MUST be identified as a new execution attempt rather than historical replay.

## Runtime State Folding Guarantee

World state MUST be derived by folding structured deltas over an initial empty
world or over a branch source projection. A world projection is valid only if
all applied deltas satisfy resource version and existence constraints.

## Ledger Integrity Guarantee

Every ledger entry MUST be content-addressed by payload. Every non-root entry
MUST reference the prior entry. Sequence numbers MUST be contiguous within a
trajectory.

## Approval Persistence Guarantee

Operator approval requirements MUST be ledger-visible. A suspended proposal
must retain proposal id, channel, and reason so approval can be resolved after
a restart.

## Delegation Containment Guarantee

Delegated work MUST be represented by a child trajectory and constrained by a
child writ. Parent and child histories MUST remain linkable by ledger entries.
