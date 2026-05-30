---
layout: default
title: Roadmap
eyebrow: Protocol evolution
subtitle: The long-term path for OpenThymos as governed cognition infrastructure.
permalink: /roadmap/
---

# Roadmap

This roadmap tracks runtime semantics and infrastructure maturity for
OpenThymos as a unified Rust execution runtime, framework, and sandbox for
coding agents. The root roadmap is maintained in [../ROADMAP.md](../ROADMAP.md).

## Phase I - Unified Deterministic Runtime

Architectural goal: stabilize the Intent -> Proposal -> Commit cycle and make
the ledger the source of execution truth across CLI, VS Code, terminal, and web
surfaces.

Runtime capabilities:

- signed capability writs
- typed capability contracts
- programmable capability manifests
- path-confined coding sandbox and worker-backed shell/HTTP fabric
- deterministic proposal compilation
- local replay verification
- world projection by ledger fold
- effect-ceiling enforcement (tool effect class checked against the writ before execution)
- auditable commits (originating intent id, policy trace, and policy-set hash recorded per commit)
- optional ed25519-signed commits with replay-side verification
- secret redaction of tool observations before ledger persistence
- model-spend budgeting (cognition token/USD usage debited against the writ budget)
- fork-proof append (unique `(trajectory, seq)` invariant under an immediate transaction)
- replay drift detection (compiler version, policy-set hash, commit signatures)

Execution guarantee: no tool execution without a staged or approved proposal,
and no effect beyond the writ's effect ceiling.

Scaling implication: correctness remains local and inspectable before
distributed concerns are introduced.

Not yet in Phase I (tracked for later phases): idempotency and compensation for
irreversible tools, writ revocation and anti-replay, multi-party (quorum)
approval, external Merkle anchoring of the ledger, host-clock attestation, and a
declarative policy language ([policy-language-v1 RFC draft](rfcs/policy-language-v1.md)).

## Phase II - Multi-Agent Coordination

Architectural goal: represent delegation as explicit runtime structure.

Runtime capabilities:

- signed child writs
- child trajectories
- delegation DAG projection
- coordination policies

Execution guarantee: child authority is a strict subset of parent authority.

Scaling implication: concurrent work becomes possible without losing lineage.

## Phase III - Distributed Execution Ledger

Architectural goal: separate ledger protocol from storage backend and support
multi-node ingestion.

Runtime capabilities:

- Postgres-backed ledger mode
- ledger export and import
- hash-chain audit proofs
- snapshot-assisted replay

Execution guarantee: replay result is independent of storage backend.

Scaling implication: the runtime can move from local history to distributed
history without changing execution semantics.

## Phase IV - Runtime Federation

Architectural goal: allow independent runtimes to exchange authority and
execution records.

Runtime capabilities:

- federated writ verification
- remote trajectory references
- cross-runtime audit queries
- importable policy bundles

Execution guarantee: federation cannot bypass local policy.

Scaling implication: cooperation does not require one global control plane.

## Phase V - Autonomous Governance Layers

Architectural goal: allow governance agents to propose policy, writ, and
runtime changes while remaining subordinate to protocol rules.

Runtime capabilities:

- governance proposal queues
- policy simulation against historical ledgers
- writ issuance workflows
- governance audit projections

Execution guarantee: governance actions are themselves proposal, approval, and
commit events.

Scaling implication: OpenThymos becomes an institutional runtime substrate,
not merely an execution loop.
