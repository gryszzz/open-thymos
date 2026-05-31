# OpenThymos RFC

## Title

Declarative Policy Language v1

## Status

Accepted — MVP implemented (Option A: minimal JSON predicate DSL). Shipped in
`thymos-policy::json_policy`:

- `JsonPolicySet::from_json(&str)` loads a bundle `{ name, version, rules[] }` at
  runtime (no recompile) and implements the existing `Policy` trait, so it plugs
  straight into `PolicyEngine::with(...)`.
- Rule `when` is a closed predicate DSL: leaf `{field, op, value}` with ops
  `eq/ne/gt/lt/gte/lte/contains/starts_with/in`, combined by `all`/`any`/`not`.
  Field paths resolve over `(Intent, Writ)` (`intent.target`, `intent.kind`,
  `intent.author`, `intent.args.<k>`, `writ.tenant_id`, `writ.subject`,
  `writ.issuer`).
- Decisions: `permit` / `deny{reason}` / `require_approval{channel,reason}`.
- Evaluation is deterministic over `(Intent, Writ)` — no clock, RNG, network, or
  floats in rule data — and fail-closed (a bundle that does not parse, or names
  an unknown op, is rejected at load).
- The bundle `name@version` already flows into `PolicyEngine::policy_set_hash`,
  so a changed bundle is detected by replay drift checks.

Deferred to a follow-up (see Unresolved Questions): **signed bundles**
(ed25519 over the canonical bundle, verified at load), `world.*` accessors, and
CEL/Rego (Options B/C) if the closed DSL proves too limited.

## Summary

Policies in OpenThymos are currently compiled Rust types implementing the
`thymos_policy::Policy` trait (e.g. `WritAuthorityPolicy`,
`TenantIsolationPolicy`, `ThresholdApprovalPolicy`). They are expressive and
fast, but they cannot be authored, reviewed, versioned, signed, or changed
without rebuilding and redeploying the runtime binary. This RFC explores a
**declarative, signed, versioned policy language** that can be loaded at runtime
while preserving the determinism guarantees that replay depends on.

This RFC builds on the provenance hook already in place:
`PolicyEngine::policy_set_hash()` is recorded in every commit
(`CommitBody::policy_set_hash`) and can be pinned by
`ReplayConfig::require_policy_set_hash`. A declarative language would make that
hash the digest of an explicit, signable artifact rather than of an in-binary
rule list.

## Motivation

- **Authorability.** Operators and compliance owners need to express and review
  policy without Rust or a redeploy.
- **Independent versioning + signing.** A policy set should be a first-class,
  signed artifact with its own lifecycle, so a deployment can prove *which*
  rules governed a trajectory and *who* authorized them.
- **Auditability.** A declarative rule set is reviewable as data; a compiled
  trait is not.
- **Replay integrity.** Policy evaluation is part of the compile path. Any
  language MUST be deterministic over `(Intent, Writ, World)` — no wall clock,
  no RNG, no network, no floating point — or it breaks the determinism the
  ledger and replay rely on.

## Non-Goals

- Turing-completeness. Policies must terminate and be statically bounded.
- Side effects. Policies decide; they never mutate state or call out.
- Replacing capability writs. Authority remains carried by signed writs; the
  policy layer constrains *within* granted authority, it does not grant it.

## Options

**Option A — Minimal JSON predicate DSL (custom).**
A small, closed grammar: boolean combinators over typed accessors into
`Intent`/`Writ`/`World`, comparison operators, and a fixed set of decisions
(`permit` / `deny{reason}` / `require_approval{channel,reason}`).
- Pros: fully controllable determinism; trivial to hash/sign; small attack
  surface; no third-party runtime.
- Cons: we own the parser, evaluator, and docs; expressiveness grows by
  committee.

**Option B — CEL (Common Expression Language), deterministic subset.**
Adopt CEL with a vetted profile (no `timestamp(now)`, no extensions, integer/
decimal-as-fixed-point only).
- Pros: documented, familiar, widely tooled.
- Cons: must prove the chosen subset is deterministic across versions; floats
  and time functions must be statically forbidden; larger dependency.

**Option C — Rego / OPA.**
- Pros: powerful, battle-tested in policy-as-code.
- Cons: heavy runtime; non-trivial to guarantee determinism and to embed; large
  surface; likely overkill for Phase I/II.

**Option D — Keep Rust policies, add signed bundles.**
Don't add a language; instead formalize a *registry* of named Rust policies and
sign the *selection + parameters* (e.g. `ThresholdApprovalPolicy` params) as a
bundle, hashed by the existing `policy_set_hash`.
- Pros: smallest change; preserves performance; no new evaluator.
- Cons: still can't add genuinely new logic without a redeploy; only the
  configuration is data.

## Recommendation (for review)

Start with **Option A** (a minimal, closed JSON predicate DSL) for new logic,
while keeping **Option D**'s signing/bundling model for both Rust and DSL
policies:

1. A **policy bundle** is a signed artifact: `{ version, rules[], issuer_pubkey,
   signature }`, where `signature` is ed25519 over `canonical_json(bundle
   without signature)` — mirroring `Writ` and the new signed-commit scheme.
2. `PolicyEngine` gains a loader that validates the bundle signature, compiles
   the DSL to an internal evaluator, and exposes the existing
   `policy_set_hash()` as the bundle digest.
3. Each commit already records `policy_set_hash`; extend the record to also
   carry the bundle `version` and issuer for human-readable provenance.
4. Evaluation MUST be pure and statically bounded; the loader rejects any rule
   referencing time, randomness, floats, or external data (fail-closed).

Defer CEL/Rego (Options B/C) unless the DSL proves too limited; revisit by
follow-up RFC.

## Determinism & Replay Constraints (binding on any option)

- Inputs: exactly `(Intent, Writ, World)`. No ambient inputs.
- No floating point anywhere in the evaluation or the serialized rule set
  (consistent with the proposal-contract RFC's fixed-point rule).
- Evaluation order is the declared rule order; first non-permit short-circuits
  (matches today's `PolicyEngine::evaluate`).
- The bundle is content-addressed; `policy_set_hash` is its digest and is
  pinned by replay to detect drift.

## Security Considerations

- Policy bundles are **signed**; the runtime verifies the signature before
  loading and records the issuer. An unsigned or malformed bundle fails closed
  (deny-all or refuse to start — to be decided).
- Policies constrain within writ-granted authority and MUST NOT be able to
  grant authority, widen effect ceilings, or alter budgets.
- Parse/eval errors fail closed (treated as `deny`), never `permit`.

## Unresolved Questions

- DSL surface: which accessors and operators are in v1? (Needs a concrete grammar
  proposal.)
- Failure posture when no valid bundle is present: deny-all vs refuse-to-start.
- Whether bundle provenance (version + issuer) belongs in `CommitBody` or in a
  separate, less hot record.
- Migration: how the existing Rust stock policies are expressed as (or coexist
  with) bundles.
- Whether `require_approval` channels are validated against an allowlist at load
  time.

## References

- `thymos-policy` (current trait-based engine; `policy_set_hash`).
- `proposal-contract-v1.md` (determinism / fixed-point rules, signing pattern).
- `Writ` signing (`thymos-core::writ`) and signed commits
  (`Commit::new_signed`) for the signature/verification pattern to reuse.
