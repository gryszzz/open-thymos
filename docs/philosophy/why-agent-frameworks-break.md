---
layout: default
title: Why Agent Frameworks Break
eyebrow: Systems critique
subtitle: Agent failures are often runtime failures disguised as model behavior.
permalink: /philosophy/why-agent-frameworks-break/
---

# Why Agent Frameworks Break

Agent frameworks usually begin as a convenience layer around model output and
tool calls. That architecture is sufficient for demos. It is not sufficient
for governed execution.

The failure is structural: cognition, authority, execution, state, and audit
are allowed to collapse into the same loop.

## Non-Determinism

Language models are stochastic. Tool outputs may be time-dependent. Network
services drift. Local files change. A runtime that treats the whole loop as a
single opaque process cannot explain why a later run differs from an earlier
run.

Non-determinism is not the problem by itself. The problem is failing to isolate
where non-determinism enters and how it is recorded.

OpenThymos admits stochastic cognition at the intent boundary. After that, the
runtime path is explicit: compile, policy, execute, commit, replay.

## Hidden Execution

Many frameworks allow models to select and invoke tools in one motion. The
user sees an action after it has already happened. Authority is inferred from
application code, prompt rules, or tool naming conventions.

Hidden execution destroys auditability. It prevents meaningful approval,
because there is no stable proposal object to approve.

OpenThymos requires an intent to become a proposal before execution. A proposal
contains writ identity, execution plan, and policy trace.

## Stochastic Tool Chains

Tool chains often combine model parsing, tool selection, argument generation,
execution, and result interpretation. Each step may be stochastic or loosely
typed. A small provider change can alter the execution path.

OpenThymos uses typed tool contracts. A provider may propose arguments, but the
runtime validates schemas, preconditions, budgets, and policy before a tool can
run.

## Unsafe Autonomy

Unsafe autonomy is not autonomy with bad prompts. It is autonomy without an
authority model. If a system cannot say why an actor was allowed to perform an
effect, it is not governed.

OpenThymos uses capability writs. Writs bind subjects to tool scopes, budgets,
time windows, effect ceilings, tenant ids, and delegation bounds.

## Lack Of Replayability

Logs are not replay. A text transcript can describe what happened, but it
cannot prove that world state follows from recorded effects.

OpenThymos commits structured deltas and observations into an append-only
ledger. Replay folds those deltas and verifies hash chain integrity, parent
linkage, sequence continuity, and compiler version constraints.

## No Governance Layer

Many frameworks put safety in ad hoc callbacks. This makes policy a local
programming pattern rather than a runtime guarantee.

OpenThymos makes policy part of proposal compilation. Policy decisions are
recorded in policy traces. A requirement for approval becomes a ledger entry,
not a UI convention.

## Invisible Reasoning

Some systems attempt to expose hidden reasoning as a safety mechanism. That is
the wrong boundary. Reasoning text is not authority. It is not an execution
record. It is not a proof.

OpenThymos records runtime facts: intent, proposal, policy trace, rejection,
approval, observation, commit, delegation, branch, and replay report. The
auditable object is the execution path, not private model cognition.

## Execution Drift

Execution drift occurs when the same task takes different paths because
providers, tools, prompts, or application code changed. Without content
identity and compiler versioning, drift is difficult to detect.

OpenThymos records compiler version on commits and derives ids from canonical
payloads. Replay can detect drift where historical records no longer satisfy
current verifier assumptions.

## Provider Inconsistency

Provider APIs change. Model families differ in tool syntax, context handling,
streaming semantics, retry behavior, and output shape.

OpenThymos confines provider variation to the cognition adapter. Providers
produce intents. They do not own the tool gateway, policy engine, ledger, or
world projection.

## Inability To Audit Actions

If an action cannot be connected to authority, policy, tool input, observed
output, and resulting state change, it cannot be audited.

OpenThymos makes those links explicit:

```text
Intent -> Proposal -> PolicyTrace -> Tool Observation -> Commit -> Ledger Fold
```

The audit trail is not an afterthought. It is the runtime shape.

## How OpenThymos Solves The Class Of Failure

OpenThymos does not try to make cognition intrinsically safe. It builds a
runtime around cognition so that unsafe proposals can be rejected, suspended,
or constrained before effects occur.

The system response is:

- isolate cognition from execution
- require proposals before effects
- express authority as signed writs
- evaluate policy as runtime semantics
- represent approval as ledger state
- commit structured deltas
- replay from ledgered facts
- keep providers behind a narrow intent contract
- treat world state as a fold, not mutable chat memory

The result is not less capable execution. It is execution with a shape that can
be inspected, replayed, and governed.
