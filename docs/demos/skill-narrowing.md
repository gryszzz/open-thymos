---
layout: default
title: Skills Narrow Authority
eyebrow: Demo
subtitle: A reusable skill can only shrink what a run may do — provably, and on the ledger.
permalink: /demos/skill-narrowing/
---

# Skills Narrow Authority

A **skill** packages *how* to do a recurring task: instructions for cognition, an
allow-list of tools, and caps on the effect ceiling and budget. The temptation in
most frameworks is to let a "skill" or "role" hand the model new powers. OpenThymos
does the opposite, by construction:

> a skill **never grants** authority — binding one to a run can only **narrow** it.

The effective authority is the *intersection* of the caller's writ and the skill:
`tools = requested ∩ allow-list`, `ceiling = AND`, `budget = min`. This is the same
strict-subset rule that governs [delegation](./delegation-lineage/), applied to a
template instead of a parent writ. The binding is recorded as a content-addressed
`skill_bound` ledger entry that [replay](../replay/) re-verifies by hash.

Every claim here is asserted in
[`thymos-core`'s `skill` tests](https://github.com/gryszzz/open-thymos/blob/main/thymos/crates/thymos-core/src/skill.rs)
(including a 4096-case randomized subset proof) and the server's e2e binding test.

## Run it

```bash
cargo run --example skill_narrowing -p thymos-runtime
```

## What you see

```text
ceiling   writ=read+write+external    skill=read         → effective=read
budget    writ.tool_calls=64    cap=4     → effective=4
tools     writ=["kv_*", "http_*"] ∩ skill=["kv_get"] → effective=["kv_get"]

✓ effective authority ⊊ writ — write + external stripped, tools + budget capped

ledger    seq 1 = skill_bound(read-only-triage v1)  id=skill:4cbc4f2a…
replay    verified 2 entries — incl. recomputing the skill hash ✓
✓ tampered skill definition rejected by replay: … claims skill:4cbc4f2a… but
  definition hashes to skill:7096ea8d…
```

A broad writ (read+write+external, `kv_*` and `http_*`) bound to a read-only
triage skill yields an **effective** authority that has lost `write` and
`external`, is restricted to `kv_get`, and is capped to 4 tool calls and \$0. The
skill could not have *added* anything the writ lacked — `AND` only clears bits,
`min` only lowers, an allow-list only removes.

## Use it for real

The narrowing happens server-side when a run binds a skill, so the signed writ
*already* reflects the intersection — every downstream policy check just works.

```bash
# author a skill (also: the desktop Skills tab)
thymos skill new read-only-triage \
  --instructions "Inspect state to answer; never mutate or call out." \
  --tools kv_get --ceiling read

# bind it to a run — the run's writ is narrowed before it is signed
thymos run "audit the latest order" --skill read-only-triage
```

The run's ledger opens with a `skill_bound` entry (seq 1, right after the genesis
root). `thymos replay <run> --verify` re-hashes the inlined definition and fails
if it was tampered with — the skill that governed a run is auditable forever,
without trusting any live registry.

## Why it matters

- **Safe by construction.** A skill is mathematically incapable of widening a
  writ; the property is randomized-tested over thousands of inputs.
- **Reusable + tunable.** Editing a skill bumps its version and mints a fresh
  content-addressed id; old runs stay pinned to the exact version they used.
- **Auditable.** The binding is on the hash-chained ledger and verified on replay.

See the [Skills RFC](https://github.com/gryszzz/open-thymos/blob/main/docs/rfcs/skills.md)
for the full design and the authority-boundary argument.
