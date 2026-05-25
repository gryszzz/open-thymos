---
layout: default
title: Threat Model
eyebrow: Runtime security
subtitle: The model is untrusted, providers are variable, tools are effectful, and the ledger is the audit boundary.
permalink: /threat-model/
---

# Threat Model

OpenThymos assumes cognition is not an authority boundary. A model may be
wrong, compromised, prompt-injected, inconsistent, or maliciously instructed.
The runtime must still preserve authority, traceability, and replay.

## Assets

The protected assets are:

- ledger integrity
- writ signing keys
- tenant boundaries
- tool scopes
- budget ceilings
- policy decisions
- approval records
- world projection correctness
- audit history
- provider credentials
- operator decisions

## Trust Boundaries

| Boundary | Trusted For | Not Trusted For |
| --- | --- | --- |
| Cognition provider | Producing candidate intents | Authority, policy, execution, state mutation |
| Compiler | Deterministic validation | External effects |
| Policy engine | Pure decisions | Tool execution |
| Tool gateway | Effectful work after approval | Authority creation |
| Ledger | Historical record | Deciding future policy |
| Operator surface | Submitting tasks and approvals | Rewriting runtime truth |

## Threats

### Prompt Injection

An input attempts to convince cognition to ignore policy, leak secrets, or call
dangerous tools. Mitigation: cognition cannot execute tools; tool access must
pass writ scope, policy, budget, and type checks.

### Authority Forgery

An actor attempts to mint or modify a writ. Mitigation: writs are signed over
canonical body data. Child writs must be strict subsets of parent writs and
must preserve tenant boundaries.

### Hidden Tool Execution

A provider or client attempts to call tools outside the runtime. Mitigation:
tool execution is centralized behind staged proposals and recorded commits.
Operational deployments should isolate workers and restrict direct tool access.

### Ledger Tampering

An actor modifies prior entries, reorders entries, or inserts entries with
invalid parents. Mitigation: replay recomputes hashes, checks parent linkage,
and verifies contiguous sequences.

### Policy Bypass

An actor tries to move approval logic into UI code or provider prompts.
Mitigation: policy decisions are compiler outputs and pending approvals are
ledger entries.

### Provider Drift

A provider changes behavior, tool-call syntax, or model output distribution.
Mitigation: provider adapters emit intents only; runtime semantics are provider
independent after intent admission.

### Replay Divergence

Replay produces a world that differs from observed state. Mitigation: replay
folds committed deltas and can compare rebuilt world hashes against observed
world hashes. Divergence is treated as an invariant failure.

### Cross-Tenant Delegation

A child writ attempts to access a different tenant. Mitigation: child writs
must inherit the parent tenant id; tenant isolation policy rejects mismatched
resource prefixes.

## Non-Goals

OpenThymos does not claim that a provider output is true, complete, or safe.
OpenThymos does not claim that an external tool is deterministic. It claims
that authority and effects are mediated through recorded runtime semantics.

## Required Controls For Production

Production deployments should enforce:

- worker-backed tool execution
- explicit allowed origins
- persistent ledger storage
- strict provider credential handling
- tenant-scoped writs
- policy review for irreversible tools
- audit export retention
- replay checks during incident review
