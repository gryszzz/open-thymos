---
layout: default
title: Multi-Agent Delegation
eyebrow: Demo
subtitle: A parent agent grants less than it holds; the ledger records the lineage.
permalink: /demos/delegation-lineage/
---

# Multi-Agent Delegation

Delegation is where most agent frameworks quietly lose the plot: a "sub-agent"
is spawned with the same powers as its parent, and nothing records who actually
did what. OpenThymos makes delegation a first-class, *bounded* operation —

> **cognition proposes, the runtime governs, the ledger records** — recursively.

A parent mints a **child writ that is a strict subset of its own authority**,
the child runs on its **own trajectory**, and the parent→child edge is written
to the ledger. This page walks the runnable example; every claim it makes is
asserted property-by-property in
[`tests/delegation.rs`](https://github.com/gryszzz/open-thymos/blob/main/thymos/crates/thymos-runtime/tests/delegation.rs).

## Run it

```bash
cargo run --example delegation_lineage -p thymos-runtime
```

## Scenario

An operations agent for tenant `acme` holds a writ that authorizes `kv_*` and
`delegate`, with `read+write` effect ceiling and delegation depth 2. It:

1. commits a write (`order = received`) on its own trajectory, then
2. **delegates** a read-only "audit the order" sub-task, restricting the child
   to `kv_get` only.

## What the runtime does

```text
== parent writ: subject=ops-agent tenant=acme scopes=[kv_*, delegate] depth=2

-> parent trajectory: traj:252795ab…
   parent kv_set(order=received): Committed(commit(275da9a2))

-> delegated 'audit the order' → child trajectory: traj:73e0172b…
   child writ ⊆ parent? true
   child: tenant=acme scopes=[kv_get] depth=1 can_write=false
   cross-tenant child rejected? true

-> child kv_get(order): Committed(commit(08c1c052))   (reads its OWN world)
   child kv_set(order=shipped): Rejected(AuthorityVoid("writ does not authorize tool 'kv_set'"))

== parent trajectory entries
   seq=0 Root(delegation demo)
   seq=1 Commit seq=1
   seq=2 Delegation(task="audit the order" → traj:73e0172b…)

== parent world: order = "received"
== replay: parent 1 commits verified, child 1 commits verified
```

## The four guarantees

**1. The child writ is a strict subset of the parent.**
`mint_child` only signs a child whose `verify_subset_of(parent)` holds: every
child tool scope is covered by a parent scope, budget is ≤ parent on every
dimension (here it is halved), the effect ceiling grants nothing the parent
forbids, the time window fits inside the parent's, and delegation depth is
decremented. The child above keeps `kv_get` but **loses `kv_set`** — narrower
authority by construction, not by convention.

**2. Tenant boundaries cannot be crossed by delegation.**
Every writ belongs to exactly one tenant, and a child must inherit it. A child
body that claims a different `tenant_id` is rejected by `verify_subset_of`
(`cross-tenant delegation forbidden`) before it can ever be signed — so a
compromised parent cannot launder authority into another tenant.

**3. The lineage is on the ledger.**
The parent trajectory carries a `Delegation` entry pointing at the child
trajectory id and naming the task; the child trajectory opens with a `Root`
whose note records that it was delegated. The parent→child DAG is reconstructable
from the ledger alone — no out-of-band bookkeeping.

**4. The parent's state isn't mutated by the child.**
State is a projection of a *trajectory's own* committed deltas. The child runs on
a separate trajectory with its own world — it cannot even *see* the parent's
`order = received` (the child's `kv_get` reads its own, empty world), let alone
change it. The only thing that mutates a trajectory's world is a commit on that
trajectory. After the child runs, the parent's world is byte-for-byte what it
was.

## Replay reconstructs the DAG

Replay folds each trajectory's committed deltas independently and verifies the
hash chain. Parent and child both replay deterministically — the delegation edge
is part of the parent's immutable record, so "which agent did what, under whose
authority" is a reproducible question, not a guess.

## Why this matters

This is the building block for **multi-tenant agent platforms** and **agent
swarms** that must stay inside their lane: you can hand a task to a sub-agent and
*know* — structurally, and provably after the fact — that it could not exceed the
authority you granted, touch another tenant, or silently mutate your state.

See also: [Deterministic Replay]({{ '/demos/deterministic-replay' | relative_url }}) ·
[Policy Intercept & Approval]({{ '/demos/policy-intercept-approval' | relative_url }}).
