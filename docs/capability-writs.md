---
layout: default
title: Capability Writs
eyebrow: Authority model
subtitle: Writs are signed, delegable, bounded capability documents.
permalink: /capability-writs/
---

# Capability Writs

A capability writ is the sole source of authority in OpenThymos. It authorizes
a subject to emit intents within explicit constraints. The runtime validates
writs before proposal execution.

## Writ Body

A writ body includes:

- issuer name
- issuer public key
- subject name
- subject public key
- optional parent writ id
- tenant id
- tool scopes
- budget
- effect ceiling
- time window
- delegation bounds

The writ id is derived from canonical body content. The signature is an
ed25519 signature over the canonical body.

## Tool Scopes

Tool scopes are literal names or prefix patterns. A writ that authorizes
`fs_*` covers `fs_read` and `fs_patch`; a writ that authorizes `fs_read` does
not cover `fs_patch`.

Tool scope is checked before tool execution and before the tool result can be
committed.

## Budget

Writ budget is multi-dimensional:

- tokens
- tool calls
- wall-clock milliseconds
- USD millicents

The compiler checks projected cost against remaining budget. The commit records
budget cost incurred by the execution.

## Effect Ceiling

Effect ceilings distinguish classes of effect:

- write
- external
- irreversible

A child writ cannot grant an effect that the parent forbids.

## Time Window

A writ is valid only within its `[not_before, expires_at]` interval. The
compiler rejects intents bound to expired or not-yet-valid writs.

## Tenant Boundary

Every writ belongs to exactly one tenant id. Child writs must inherit the same
tenant id as their parent. Cross-tenant delegation is invalid.

## Delegation

A writ may be subdivided only when delegation bounds allow it. A child writ
must satisfy:

- child issuer public key equals parent subject public key
- child tenant id equals parent tenant id
- child tool scopes are covered by parent tool scopes
- child budget does not exceed parent budget
- child effect ceiling does not exceed parent effect ceiling
- child time window fits within parent time window
- child delegation depth is less than parent remaining depth

This prevents lateral minting and privilege expansion.

## Lifecycle

```text
issue body -> sign body -> admit writ -> bind intent -> compile proposal
          -> debit projected budget -> commit observed cost
          -> optionally mint strict child writ
```

## Invalid States

The runtime must reject:

- signature mismatch
- child scope not covered by parent
- child budget exceeding parent
- child effect ceiling exceeding parent
- child time window outside parent window
- cross-tenant delegation
- expired writ
- unknown or unauthorized tool

## Audit Value

Writ ids appear on proposals and commits. This makes execution authority
traceable after the fact: an auditor can identify which capability authorized
each effect.
