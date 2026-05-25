---
layout: default
title: Governance
eyebrow: Protocol stewardship
subtitle: OpenThymos treats runtime semantics as governed infrastructure.
permalink: /governance/
---

# Governance

OpenThymos governance protects the execution protocol. The repository root
contains the normative project process in [../GOVERNANCE.md](../GOVERNANCE.md).
This document explains how that process maps to runtime architecture.

## Governed Surfaces

The following surfaces are governed:

- Intent, Proposal, Commit, Writ, World, and Ledger types
- compiler stage ordering
- policy decision semantics
- approval and suspension semantics
- ledger entry kinds
- replay verification behavior
- provider adapter boundaries
- tool contract expectations
- compatibility of persisted execution records

Changes to these surfaces require design review because they can affect audit
validity and replay reproducibility.

## Protocol First

Runtime semantics should be documented before they are expanded. A change that
cannot be described as a protocol rule is usually not ready for the compiler,
policy engine, or ledger.

Examples of protocol statements:

- a child writ MUST be a strict subset of its parent
- a provider MUST NOT execute tools
- a pending approval MUST be represented in the ledger
- replay MUST NOT require fresh provider output

Examples of implementation statements:

- store entries in SQLite
- expose events over SSE
- render approvals in a web console

Both matter, but only the first category defines compatibility.

## RFC Requirement

An RFC is required for changes that affect:

- content-addressed identity
- canonical serialization
- ledger entry shape
- commit shape
- writ shape
- compiler stage order
- replay verification assumptions
- provider contract
- policy decision model
- tool contract model

Use [../RFC_TEMPLATE.md](../RFC_TEMPLATE.md).

## Compatibility Rule

Historical ledgers are archival records. A change that makes old ledgers
unreplayable is a compatibility break and must include a migration or a clear
deprecation boundary.

## Authority Rule

Authority is represented by writs and policy decisions. No client surface, API
flag, provider adapter, or tool implementation may bypass this path.

## Review Rule

Protocol review should ask:

- What invariant changes?
- What proof does replay retain?
- What authority is being granted or denied?
- What happens to old ledgers?
- Can a provider change alter execution semantics?
- Is the behavior visible in audit records?

If those questions cannot be answered, the change is underspecified.
