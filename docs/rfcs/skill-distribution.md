# OpenThymos RFC

## Title

Skill distribution — a ledger-backed registry and signed marketplace sharing for skills.

## Status

Draft

## Summary

The [skills RFC](./skills.md) defined a content-addressed, authority-narrowing
skill and shipped a **local** registry (`provider.json`-style files) plus
`/skills` CRUD. This RFC adds two things on top, both additive:

1. **A ledger-backed registry** so skill authoring is itself **auditable** — each
   create/tune is recorded as a content-addressed `skill.published` entry on a
   dedicated registry trajectory, replayable like every other ledger fact.
2. **Marketplace distribution** so skills can be **shared** the same way tools
   are: as **signed packages** (reusing `thymos-marketplace`'s ed25519
   `Package` signing + trusted-publisher verification).

This affects operator surfaces and the ledger (one new entry kind); it does
**not** change the run-time authority model. Crucially, a shared or installed
skill **still only narrows** at bind time — distribution cannot make a skill grant
authority, so it is strictly safer to distribute than a tool.

## Motivation

Today a skill lives in one operator's local registry. Two gaps:

- **No provenance for authoring.** The local registry is a mutable file store;
  there is no auditable record of *who created or tuned a skill and when*. For a
  system whose value proposition is "provable + auditable," the templates that
  shape runs should themselves be on the hash-chained ledger.
- **No sharing.** Useful skills ("triage an inbox", "review a diff under a
  read-only ceiling") can't be published, discovered, or installed across teams.
  The tool marketplace already solves discovery + signing for tools; skills want
  the same path.

## Current Semantics

- Skills resolve through a local `SkillRegistry` (in `thymos-server`): file-backed
  per-name JSON, `GET/POST /skills`, `GET /skills/{id}`. Editing bumps `version`
  and mints a fresh content-addressed `SkillId = blake3(canonical_json(def))`.
- `thymos-marketplace` publishes **tool** `Package`s: a `content_hash`, an
  ed25519 `signature` over it, a `publisher_pubkey`, trusted-publisher checks,
  and `verify_integrity`.
- The ledger has entry kinds incl. `skill_bound` (run-time binding). There is no
  authoring entry.

## Proposed Semantics

### 1. Ledger-backed registry (provenance)

Add an entry kind `skill.published`, written to a dedicated, well-known **registry
trajectory** (one per tenant; seed = `blake3("thymos.skill-registry/" + tenant)`):

```jsonc
EntryPayload::SkillPublished {
  skill_id,                 // blake3(canonical_json(skill))
  skill,                    // full definition, inlined (self-verifying)
  author,                   // operator/subject id from the request context
  supersedes: Option<SkillId> // prior version of the same name, if any
}
```

`SkillRegistry::save` becomes ledger-first: it appends `skill.published`, then
projects the current name→id map by **folding the registry trajectory** (latest
entry per `name` wins). The file store is retained only as an optional cache. The
registry's truth is therefore the replayable ledger, exactly like world state.

### 2. Marketplace distribution (sharing)

A **skill package** is a `thymos-marketplace` `Package` whose body is a `SkillDef`
(a new `PackageKind::Skill`). It reuses the existing pipeline verbatim:

- `publish`: sign `blake3(canonical_json(skill))` with the publisher's ed25519
  key → `signature` + `publisher_pubkey`; trusted-publisher policy applies.
- `search` / `get`: discover by name/tag; the package carries the full def.
- `install`: `verify_integrity` (+ trusted-publisher), then hand the `SkillDef`
  to `SkillRegistry::save` (which records a local `skill.published` with the
  installing operator as `author` and the publisher recorded in metadata).

Installation **never** auto-binds a skill to a run and **never** elevates
authority; it only makes the template available to *narrow* future runs.

### Surfaces

- CLI: `thymos skill publish <name>`, `thymos skill search <q>`,
  `thymos skill install <name|id>`, `thymos skill log` (authoring history from the
  registry trajectory).
- Desktop: a "browse/share" affordance on the Skills tab.

## Invariants

- Distribution MUST NOT change run-time authority: an installed/shared skill is
  still only a narrowing template; binding it still computes
  `effective = caller_writ ∩ skill` and can never widen.
- `skill.published` MUST inline the full definition and be content-addressed; the
  registry's projected name→id map MUST be a pure fold of that trajectory (no
  out-of-band mutation).
- A skill package's signature MUST be over `blake3(canonical_json(skill))` — the
  same id the runtime binds and replay verifies — so a package and a bound run
  reference byte-identical content.
- Installing a skill MUST run the marketplace's signature + trusted-publisher
  checks; an unsigned/untrusted skill is rejected unless the operator explicitly
  opts in (same posture as tools).

## Ledger Impact

Adds one entry kind, `skill.published`, on registry trajectories. Additive: no
existing entry changes; ledgers without authored skills never emit it. Replay of
a registry trajectory folds the latest def per name; replay of a *run* trajectory
is unchanged (it already verifies `skill_bound`).

## Replay Impact

`skill.published` is self-verifying exactly like `skill_bound` (recompute
`blake3(canonical_json(skill))`, assert it equals `skill_id`). Registry state is
derived purely by replaying the registry trajectory, so "what skills exist and at
what version" is itself replayable and never depends on a live service.

## Writ And Policy Impact

None to the authority model. Optionally, **publishing** a skill can be gated by a
policy/permission (a tenant may restrict who may publish to its registry), but
that is an authoring-surface control, not a change to writ narrowing. Binding,
narrowing, and approval flows are untouched.

## Provider Impact

None. A skill's advisory `model_hint` travels with the package but remains
advisory and confers no execution authority.

## Tool Contract Impact

None. Skills reference tool ids; distributing a skill never distributes or alters
a tool contract. Installing a skill whose allow-list names a tool the installer
lacks simply yields denials at run time (narrowing against an empty intersection).

## Compatibility

- Compatible: all current runtimes for the local-registry path; the ledger-backed
  registry and marketplace kind are additive and opt-in.
- Migration: existing local skills can be back-filled by emitting a
  `skill.published` per current def on first start (idempotent by content id).
- Incompatible: an older runtime ignores `PackageKind::Skill` packages; the
  marketplace MUST refuse to *install* a skill package it cannot verify rather
  than silently treating it as a tool.

## Security Considerations

- **Distribution is safe by construction.** Unlike a tool (a real effect), a
  shared skill cannot exceed the installer's writ — worst case, malicious
  instructions are prompt-injection the writ still blocks. This is the strongest
  reason skills are a good first "social" artifact.
- **Supply-chain.** Signing + trusted-publisher verification (reused from tools)
  prevents a swapped definition; content-addressing makes any change detectable.
- **Registry tampering.** Because registry state is a fold of a hash-chained
  trajectory, a tampered local cache is detectable by re-projecting from the
  ledger.
- **Publish authorization.** Restrict who may append `skill.published` to a
  tenant's registry trajectory (optional policy), to prevent registry spam.

## Alternatives

- **Keep skills local-only.** Rejected: loses provenance + sharing, both core to
  the product thesis.
- **A separate signing scheme for skills.** Rejected: the tool marketplace's
  ed25519 `Package` model already fits; reuse beats divergence.
- **Mutable registry rows (no ledger).** Rejected: breaks the "registry is
  replayable truth" property and provenance.

## Test Plan

- Unit: `skill.published` id = `blake3(canonical_json)`; registry fold picks the
  latest version per name; supersedes linkage.
- Ledger/replay: `skill.published` round-trips + self-verifies (SQLite + a
  Postgres-gated case); a tampered registry entry fails replay.
- Marketplace: sign → verify → install a skill package; unsigned/untrusted
  rejected; installed def is byte-identical to the published one (same id).
- Authority: an installed skill bound to a run still yields
  `effective_writ ⊆ caller_writ` (reuse the narrowing proptest).
- Negative: install of a `PackageKind::Skill` on a runtime that can't verify it
  is refused, not coerced into a tool.

## Unresolved Questions

- **One registry trajectory per tenant, or per (tenant, namespace)?** Affects fold
  scope and listing; must be settled before implementation (replay/compat).
- **Publish authorization model:** a dedicated permission, a policy rule, or
  writ-gated? Leans toward a policy rule to stay inside the existing engine.
- **Marketplace coupling:** ship the ledger-backed registry first (provenance) and
  add marketplace distribution as a follow-up, or land both together? The ledger
  registry is the load-bearing half; distribution can follow.
