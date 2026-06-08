# OpenThymos RFC

## Title

Skills — reusable, tunable, content-addressed agent capabilities over tools and writs.

## Status

Accepted

> **Resolved (impl decision):** the load-bearing open question — definition
> capture for replay — is resolved in favour of **inline capture**: the full
> skill definition is embedded in the `skill.bound` ledger entry so replay is
> fully self-verifying (recomputes `blake3(canonical_json)` and asserts it equals
> the recorded id) and never depends on a live registry. The remaining unresolved
> questions (registry backend richness, param typing, marketplace distribution)
> do not affect compatibility or replay correctness and are deferred.

## Summary

A **skill** is a named, versioned, content-addressed bundle that packages *how*
to do a recurring class of task: a system/instruction prompt, an allow-list of
tools, a maximum effect ceiling + writ-scope template, and a set of tunable
parameters. Skills are **authored and tuned by operators**, stored in a registry,
editable from both the CLI (`thymos skill …`) and the desktop **Skills** tab, and
referenced by a run (`POST /runs { "skill": "<name|id>" }`).

A skill is **not** a new execution authority. It is a *proposal template*.
Selecting a skill shapes the prompt cognition sees and **narrows** (never widens)
the writ the compiler issues; every tool call still passes the unchanged
`Intent → Proposal → Commit` pipeline, the writ still gates every effect, and the
ledger records exactly which skill **content hash** governed the run. This RFC
affects operator surfaces, the compiler (writ derivation), the ledger (one new
entry kind), and replay (hash verification). It does **not** change provider
authority, tool contracts, or the hash-chain rules.

## Motivation

Today every run starts from zero: the caller supplies a free-form task and
(optionally) a `cognition` block, and the compiler issues a writ from defaults or
the request. Recurring work — "triage this inbox", "review a diff", "research and
cite", "reconcile two ledgers" — is re-specified each time, with prompt quality,
tool scoping, and effect ceilings re-decided ad hoc. There is no first-class,
reviewable, *tunable* unit that captures "this is how we want the agent to do X,
and this is the most authority X may ever request."

Operators have asked to "create and tune a skill specific for Thymos" and to
"edit it" from the app. The missing capability is a governed, content-addressed
**capability template** that is safe by construction: tuning a skill can change
its behavior and its *ceiling*, but can never let cognition grant itself
authority, and every run remains independently replay-verifiable against the
exact skill version it used.

## Current Semantics

- `POST /runs` accepts `{ task, cognition?, writ? }`. `thymos-compiler` compiles
  intents into proposals, enforcing writ scope, effect ceiling
  (`Read ≤ Write ≤ External ≤ Irreversible`), budget, and time windows.
- Capability writs are signed; delegation issues **child writs that are strict
  subsets of the parent** (Phase II).
- Ledger entries are content-addressed: `id = blake3(canonical_json(payload))`;
  replay verifies sequence continuity + parent linkage and never calls a provider
  or re-runs a tool.
- Tools are typed contracts resolved from manifest dirs; the marketplace is a
  read-only catalog. There is **no** "skill" type anywhere in the codebase.

## Proposed Semantics

### Skill definition (content-addressed)

```jsonc
{
  "name": "diff-review",
  "version": 3,
  "title": "Review a code diff",
  "instructions": "You review diffs for correctness…",   // prompt fragment
  "tools": ["fs.read", "shell.run:git", "http.get"],      // allow-list (⊆ available)
  "ceiling": "External",                                   // max effect this skill may request
  "writ_template": {                                       // scope template, all optional
    "paths": ["./**"], "domains": ["api.github.com"], "budget": { "usd": 0.50 }
  },
  "params": [                                              // tunable knobs, typed + defaulted
    { "key": "strictness", "type": "enum", "values": ["low","high"], "default": "high" }
  ],
  "model_hint": { "provider": null, "model": null }        // optional, never authoritative
}
```

The **skill id** is `blake3(canonical_json(definition))`. Editing any field
("tuning") produces a **new id** and bumps `version`; the old version remains
addressable. A skill id is therefore an immutable, replay-stable reference.

`name` is a mutable human handle resolved through the registry to the *current*
id; a run may pin either `name` (resolved at submit time, the resolved id is
recorded) or an explicit `id` (already pinned).

### Applying a skill to a run

`POST /runs { "task": "...", "skill": "<name|id>", "skill_params": { … } }`:

1. Resolve the skill to a concrete id (recording it). Unknown id/name → 400.
2. Build the cognition prompt by prepending the skill `instructions` (with
   `params` interpolated) to the task. Cognition still only **proposes**.
3. **Derive the writ as an intersection, never a union:**
   `effective_writ = caller_writ ∩ skill_writ_template`, with
   `effective_ceiling = min(caller_ceiling, skill_ceiling)` and
   `tools = caller_tools ∩ skill_tools`. A skill can only *shrink* authority — it
   is exactly the child-writ (strict-subset) rule from delegation, applied to a
   template instead of a parent.
4. Compile + run unchanged. Every tool call is gated by `effective_writ`.

If the intersection is empty for a tool the skill needs, the run proceeds with
that tool denied (surfaced as a normal policy denial), not by widening authority.

### Registry

A `SkillRegistry` trait with a default local backend (JSON files under the
operator's data dir, mirroring how the desktop persists `provider.json`) and,
later, a ledger-backed backend so skill provenance is itself auditable. The
registry resolves `name → current id`, lists versions, and returns definitions by
id. The marketplace may later distribute skills, but distribution is out of scope
here.

## Invariants

- A skill MUST NOT grant authority. `effective_writ ⊆ caller_writ` and
  `effective_ceiling ≤ caller_ceiling` always hold; a skill can only narrow.
- A skill's `model_hint` MUST NOT confer execution authority and MUST be
  overridable by the request and operator config; it is advisory only.
- The skill id MUST be `blake3(canonical_json(definition))`; tuning MUST mint a
  new id, never mutate an existing one.
- A run MUST record the resolved skill id it used; replay MUST verify it.
- Cognition MUST NOT author or mutate a skill inline during a run (skills are
  operator-authored data, not a tool the model can call to escalate). Authoring
  happens only through the operator surfaces below.
- No projected-state mutation outside commit folding; skills change *inputs to*
  the pipeline, not the pipeline.

## Ledger Impact

Add one entry kind, `skill.bound`, written at run start: `{ run_id, skill_id,
skill_name, resolved_params_hash }`. It is content-addressed like every other
entry and chained normally. Existing ledgers are unaffected (the kind is additive;
runs without a skill simply never emit it). No existing entry changes shape.

## Replay Impact

Replay gains one check: when a `skill.bound` entry is present, replay recomputes
`blake3(canonical_json(definition))` for the recorded definition (captured inline
or resolved from a content-addressed store) and asserts it equals `skill_id`, and
that the prompt/writ actually used (already recorded in the proposal) is
consistent with the bound skill + params. Replay still **never** calls a provider
or re-runs a tool. The skill definition is captured by hash at bind time so replay
does not depend on a live registry; if only the hash (not the body) is retained,
replay verifies continuity but flags the definition as externally trusted — this
trade-off is an Unresolved Question.

## Writ And Policy Impact

This is the load-bearing section. Skill application is a **writ-narrowing**
operation reusing the delegation subset machinery:

- `tools`, `paths`, `domains`, `budget`, `time window`, and `ceiling` from the
  skill template are intersected with the caller's writ.
- Policy evaluation is unchanged and runs against the *effective* writ; policy
  traces remain first-class proposal data and will note the binding skill id.
- Approval flows are unchanged; a skill may *lower* the auto-approve ceiling
  (e.g. force `External`+ to require approval) but may never raise it.

## Provider Impact

None to provider authority. `model_hint` is advisory and resolved with the
existing precedence (request > operator `THYMOS_DEFAULT_*` > skill hint > mock).
Providers gain no fields and no execution authority. This composes directly with
the in-app/CLI provider configuration (any LLM / Ollama / OpenAI-compatible
adapter): a skill says *how* to think, provider config says *which mind* thinks.

## Tool Contract Impact

None. Skills reference existing tool contracts by id and can only allow-list a
**subset**; no schema, precondition, postcondition, or cost field changes. A
skill referencing an unknown tool id is a validation error at author time and a
denial at run time.

## Compatibility

- Compatible: all current `vX.Y.Z` runtimes for runs that omit `skill`
  (purely additive request field + additive ledger kind).
- Incompatible: an older runtime cannot *honor* a `skill` field — it ignores it,
  so a caller relying on skill-narrowing against an old server would get the
  caller's full writ. Servers therefore MUST reject `skill` with 400 if the
  feature is disabled, so silence never means "ran with more authority than
  intended."
- Migration: none for existing ledgers. New skills are created going forward.
- Cannot migrate: pre-existing ad-hoc runs are not retro-fitted with skills.

## Security Considerations

- **Authority escalation (primary risk):** mitigated by the intersection rule —
  a skill is mathematically incapable of widening a writ. Tests must assert this
  with adversarial templates (over-broad paths/domains/ceiling).
- **Prompt-injection via tuned instructions:** a malicious skill could instruct
  cognition to attempt harmful actions — but it still cannot *do* them past the
  writ. Authoring is operator-gated; skills are not a model-callable tool.
- **Replay divergence:** prevented by binding the skill id into the ledger and
  verifying the hash on replay.
- **Registry tampering:** a swapped local skill file changes the id; a run pinned
  to an id is unaffected, and a run pinned to a name records the resolved id, so
  tampering is detectable after the fact.

## Alternatives

- **Skills as marketplace tools.** Rejected: tools are single typed effects;
  skills are prompt + scope + params spanning many tools. Conflating them would
  blur the tool contract.
- **Skills that carry their own signed writ (grant authority).** Rejected
  outright: violates "cognition proposes, the runtime governs." Skills narrow;
  they never grant.
- **Mutable skills (edit in place, stable id).** Rejected: breaks replay
  determinism. Content-addressing + version bump is non-negotiable.
- **Prompt-only skills (no writ template).** Rejected as too weak: the value is
  binding *how to think* to *the most authority that thinking may request*.

## Test Plan

- Unit: skill id = `blake3(canonical_json)`; tuning mints a new id; param
  interpolation; registry name→id resolution + versioning.
- Writ: `effective_writ ⊆ caller_writ` for randomized templates (property test);
  ceiling intersection; empty-intersection → denial, never widening.
- Integration: `POST /runs { skill }` end-to-end on mock cognition; `/skills`
  CRUD; CLI `thymos skill new|show|tune|use`.
- Ledger/replay: `skill.bound` is written, chained, and hash-verified on replay;
  a tampered definition fails replay.
- Negative: unknown skill → 400; cognition attempting to author a skill mid-run
  is rejected; old-runtime rejection path returns 400 rather than silently
  ignoring `skill`.

## Operator Surfaces (informative)

Not protocol, but the reason this RFC exists — the "spot to edit it":

- **CLI:** `thymos skill list | show <name> | new | tune <name> | use <name>`;
  `thymos run <task> --skill <name>`.
- **Desktop:** a **Skills** tab — list/create/tune (name, instructions, tool
  allow-list, ceiling, writ template, typed params), backed by `GET/POST /skills`
  and `GET /skills/{id}`; a skill picker on the Chat composer. Mirrors the
  Providers tab: edit locally, applied by the governed runtime.

## Unresolved Questions

- **Definition capture for replay:** store the full skill body inline in
  `skill.bound`, or store only the hash + a content-addressed skill store? Inline
  is fully self-verifying but larger; hash-only keeps entries small but makes the
  body externally trusted. Must be resolved before implementation (affects replay
  correctness).
- **Registry backend:** ship local-JSON only first, or land the ledger-backed
  registry in the same change so skill provenance is auditable from day one?
- **Param typing surface:** how rich should `params` be (enums/strings/numbers
  only, vs. structured) without becoming a config language?
- **Marketplace distribution + signing of shared skills:** deferred; should it
  reuse the tool marketplace's signing path?
