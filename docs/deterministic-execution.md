---
layout: default
title: Deterministic Execution
eyebrow: Runtime semantics
subtitle: Determinism is enforced by protocol boundaries, content identity, and ledger folding.
permalink: /deterministic-execution/
---

# Deterministic Execution

OpenThymos treats determinism as a runtime property. A run may begin with a
stochastic provider, but after an intent is emitted the runtime path is
explicit, typed, and recorded.

## Deterministic Region

The deterministic region begins at intent admission and ends at ledger commit.

Inside this region, the runtime controls:

- compiler stage order
- writ validation
- tool resolution
- budget projection
- policy evaluation
- proposal identity
- postcondition checks
- commit construction
- ledger append order
- world folding

The runtime does not ask the model whether authority is valid.

## Non-Deterministic Inputs

The following inputs can be non-deterministic:

- provider token generation
- wall-clock timestamps used for writ time windows
- external tool results
- network latency
- operator approval timing

OpenThymos admits these inputs only at defined boundaries. Once an observation,
approval, rejection, or commit is written to the ledger, replay uses the stored
record.

## Content Identity

Protocol objects derive identity from canonical serialized content. This gives
the runtime stable names for:

- intents
- proposals
- commits
- writs
- ledger payloads
- world projections

If two inputs produce the same canonical payload, they produce the same content
identity. If content changes, identity changes.

## Compiler Determinism

For a fixed `(Intent, Writ, World, ToolRegistry, PolicyEngine, CompileContext)`,
the compiler must produce the same result:

- staged proposal
- suspended proposal
- typed rejection

Compiler determinism requires stable tool schemas, stable policy ordering, and
stable budget estimates for the same input.

## Tool Determinism

Tools may observe external systems. The runtime therefore records tool
observations and structured deltas. Replay of a completed run does not re-call
the tool. It applies the committed delta.

For tools that must support deterministic dry-run verification, the tool
contract should expose a deterministic simulator. That simulator is outside the
minimum replay guarantee.

## Ledger Determinism

The ledger imposes deterministic ordering through sequence numbers and parent
links. A valid trajectory has one ordered path from root to head. Branches and
delegations create explicit ledger entries rather than implicit side channels.

## World Folding

World state is computed by folding commit deltas in sequence order:

```text
World_0 = empty
World_n = apply(World_n-1, Commit_n.delta)
```

The fold is deterministic when deltas are deterministic and conflict checks are
stable. A compare-and-swap conflict during folding is an invariant failure.

## Deterministic Execution Guarantees

- A provider cannot directly affect state after intent emission.
- A tool cannot mutate projected state outside a committed delta.
- A policy decision is recorded in the proposal that reached execution or
  suspension.
- A replay verifier can rebuild world state without new model or tool calls.
- Compiler version drift can be detected during replay when pinning is enabled.
