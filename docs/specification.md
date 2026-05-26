---
layout: default
title: Specification
eyebrow: Runtime protocol
subtitle: Normative terminology and execution semantics for OpenThymos.
permalink: /specification/
---

# Specification

This document defines the OpenThymos runtime protocol. Keywords such as MUST,
MUST NOT, SHOULD, and MAY are used in their ordinary specification sense.

## 1. Terms

`Cognition` is a process that proposes actions. It MUST NOT execute tools,
persist ledger entries, or mutate world state.

`Intent` is the only object emitted by cognition. An intent has no authority.

`Proposal` is the compiler output. A proposal binds an intent to a writ, a
validated execution plan, and a policy trace.

`Commit` is the only object that mutates projected world state. A commit
contains a structured delta and one or more observations.

`Writ` is a signed capability document. It defines subject authority, tool
scope, budget, effect ceiling, time window, tenant boundary, and delegation
bounds.

`Capability` is a registered effect contract. A capability MAY be implemented
as a Rust `ToolContract`, a JSON manifest tool, or an MCP bridge tool. It MUST
declare metadata, an input schema, an effect class, and a risk class.

`Ledger` is the append-only record of trajectory entries.

`World` is a deterministic projection obtained by folding committed deltas from
the ledger.

`Provider` is an adapter that implements the cognition contract.

`Surface` is a client attached to the runtime, such as the CLI, VS Code
sidebar, interactive terminal shell, HTTP API client, or web console. A surface
MUST NOT become the source of execution truth.

## 2. Execution Grammar

The runtime execution grammar is:

```text
Run       := Root Entry*
Entry     := Commit | Rejection | PendingApproval | Delegation | Branch
Commit    := Proposal Observation Delta
Proposal  := Intent Writ ExecutionPlan PolicyTrace Status
Intent    := Author Kind Target Args Rationale Nonce
```

The runtime MUST NOT execute an `Intent` directly. The runtime MUST compile an
intent into a `Proposal` before any capability invocation.

## 3. Compilation

Compilation MUST evaluate the following stages in order:

1. intent kind support
2. writ signature verification
3. writ time-window check
4. writ tool-scope binding
5. capability registry resolution
6. budget projection
7. tool argument validation
8. tool precondition evaluation
9. policy evaluation
10. proposal emission

Compilation returns one of:

- `Staged(Proposal)`
- `Suspended(Proposal, channel, reason)`
- `Rejected(RejectionReason)`

If any authority check fails, the compiler MUST reject before capability
execution.

## 4. Policy Decisions

A policy engine is an ordered collection of pure policy functions:

```text
(Intent, Writ, World) -> PolicyDecision
```

`PolicyDecision` is one of:

- `Permit`
- `Deny(reason)`
- `RequireApproval { channel, reason }`

The compiler MUST attach a policy trace to every emitted proposal. The trace
MUST include evaluated rule names and the final decision.

## 5. Execution

The runtime MAY invoke a capability only when a proposal is staged or when a
previously suspended proposal has been approved.

A capability invocation receives validated arguments and a world projection. It
returns an observation and a structured delta. The runtime MUST check
postconditions before committing the result.

## 6. Commit

A commit MUST include:

- parent commit or ledger parent
- trajectory id
- proposal id
- writ id
- monotonically increasing sequence number
- structured delta
- observation list
- compiler version
- budget cost
- optional signature field

The runtime MUST append the commit to the ledger before the committed delta is
considered part of projected world state.

## 7. Ledger

A ledger entry MUST be content-addressed from its canonical payload. Entries
MUST form a parent chain. Sequence numbers MUST be contiguous inside a
trajectory.

Supported entry kinds:

- `root`
- `commit`
- `rejection`
- `pending_approval`
- `delegation`
- `branch`

The ledger MUST reject entries that violate sequence or parent-chain rules.

## 8. Replay

Replay MUST:

- recompute every entry hash
- verify parent linkage
- verify sequence continuity
- apply commit deltas in order
- report compiler versions seen
- optionally reject compiler version drift
- optionally compare rebuilt world hash with observed world hash

Replay MUST NOT call a provider for new cognition. Replay MUST NOT execute
tools for new observations.

## 9. Provider Abstraction

A provider MAY be stochastic. Runtime semantics MUST remain deterministic after
the provider emits an intent. Provider identity MUST NOT grant additional tool
authority, budget, policy exceptions, or ledger write access.

## 10. Compatibility

Changes to canonical serialization, content hash inputs, ledger entry shapes,
writ bodies, commit bodies, or replay verification are protocol changes. They
SHOULD use the RFC process described in [../RFC_TEMPLATE.md](../RFC_TEMPLATE.md).
