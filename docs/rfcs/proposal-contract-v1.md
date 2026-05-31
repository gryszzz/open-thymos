# OpenThymos RFC

## Title

Proposal Contract v1

## Status

Stable (v1). The proposal contract is frozen: the `Proposal` / `ProposalBody` /
`ProposalStatus` shapes below are the stable surface downstreams may depend on.

## Summary

This RFC specifies the stable v1 proposal contract: the canonical `Proposal`
type, the `ProposalStatus` enumeration, and the content-addressing rules for
`ProposalId`. A `Proposal` is fully content-addressed — it carries only its id
and its body, with no experimental or provider-supplied fields. It affects
runtime semantics (compilation output) and ledger compatibility (serialized
`PendingApproval` payloads). It does not affect replay correctness for entries
that contain no `PendingApproval` payloads.

## Motivation

Before this RFC, `ProposalStatus` had unit variants that did not carry the
`channel` and `reason` fields mandated by Section 2 of the specification. The
`Suspended` variant was named `SuspendedForApproval` and carried no associated
data; the `Rejected` variant had no reason field. Both mismatched the grammar:

```text
Status := Staged | Suspended { channel, reason } | Rejected { reason }
```

An earlier draft of this RFC also proposed an experimental `routing_evidence`
field for provider routing metadata. It was never read by the runtime and was
removed before stabilization (see "Proposed Semantics") so the v1 contract has
no inert fields on its compatibility surface. Provider routing metadata, if ever
needed, will be introduced by a separate RFC.

## Current Semantics

Before this change:

```rust
pub enum ProposalStatus {
    Staged,
    Rejected,
    SuspendedForApproval,
}

pub struct ProposalBody {
    pub intent_id: IntentId,
    pub writ_id: WritId,
    pub plan: ExecutionPlan,
    pub policy_trace: PolicyTrace,
    pub status: ProposalStatus,
}

pub struct Proposal {
    pub id: ProposalId,
    pub body: ProposalBody,
}
```

`ProposalId` was `content_hash(body)`.

## Proposed Semantics

After this change:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalStatus {
    Staged,
    Suspended { channel: String, reason: String },
    Rejected { reason: String },
}

pub struct ProposalBody {
    pub intent_id: IntentId,
    pub writ_id: WritId,
    pub plan: ExecutionPlan,
    pub policy_trace: PolicyTrace,
    pub status: ProposalStatus,
}

pub struct Proposal {
    pub id: ProposalId,
    pub body: ProposalBody,
}
```

`ProposalId` remains `content_hash(body)`. A `Proposal` carries exactly its
content-addressed id and its body — nothing else. There is no provider-supplied
or experimental field, so the contract is fully content-addressed and stable.

The `Suspended` variant embeds `channel` and `reason` directly in the status,
matching Section 2. The compiler populates both fields from the policy engine's
`RequireApproval` decision; the runtime reads them from the status when
appending `PendingApproval` ledger entries.

> **Provider routing metadata** (previously a proposed experimental
> `routing_evidence` field) is intentionally **not** part of this stable v1
> contract. It was removed to avoid shipping an inert field on the
> compatibility surface. If a future need arises to surface provider routing
> decisions, it must be introduced by a separate RFC — outside `ProposalBody`,
> signed, and never able to influence authority, policy, budget, or replay.

## Invariants

- `ProposalId = blake3(canonical_json(ProposalBody))`.
- A `Proposal` contains only `id` and `body`; there are no fields outside the
  content-addressed body.
- `ProposalStatus::Suspended` MUST carry the same `channel` and `reason` values
  that appear in the `PendingApproval` ledger entry.
- `ProposalStatus::Rejected::reason` is a human-readable summary string; it is
  distinct from `RejectionReason`, which is the structured enum used in
  `Rejection` ledger entries.
- Floating-point values MUST NOT appear in any type that is part of a canonical
  payload hash input.

## Ledger Impact

The `PendingApproval` ledger entry embeds a full `Proposal` in its payload:

```rust
EntryPayload::PendingApproval {
    proposal: Proposal,
    channel: String,
    reason: String,
}
```

Because `ProposalStatus` serialization changes (from a plain string to a
tagged object), any `PendingApproval` entry written before this RFC cannot be
deserialized by a runtime built after it. Ledgers that contain no
`PendingApproval` entries are unaffected.

Entries for `Root`, `Commit`, `Rejection`, `Delegation`, and `Branch` are
not affected by this change.

## Replay Impact

Replay reads `PendingApproval` payloads only to surface them in the
`ReplayReport`; it does not apply deltas from them. A replay verifier built
after this RFC will fail to deserialize a `PendingApproval` entry written
before it. Operators should treat pre-RFC `PendingApproval` entries as
incompatible and re-run the affected trajectories if replay verification is
required.

All five mandatory replay steps (hash verification, parent linkage, sequence
continuity, delta application, compiler version reporting) remain unchanged and
unaffected for trajectories that contain no `PendingApproval` entries.

## Writ And Policy Impact

None. Writs, budgets, time windows, tool scopes, delegation bounds, and policy
rules are unchanged. The policy engine continues to receive `(Intent, Writ,
World)` and returns `PolicyDecision`. The `channel` and `reason` from a
`RequireApproval` decision are now embedded in `ProposalStatus::Suspended`
rather than being passed out-of-band from the compiler.

## Provider Impact

Providers that implement the `Cognition` trait are unaffected. The trait
signature `step(ctx) -> Result<CognitionStep>` does not change.

Providers cannot attach metadata to a `Proposal`: the contract has no
provider-supplied field. Provider routing decisions, if ever surfaced, will be
specified by a separate RFC and carried outside this contract.

## Tool Contract Impact

None. Tool contracts receive `ToolInvocation { args, world }` and return
`ToolOutcome { observation, delta }`. The proposal contract change does not
affect this interface.

## Compatibility

- Compatible with: runtimes built after this RFC, ledgers with no
  `PendingApproval` entries.
- Incompatible with: runtimes built before this RFC for any ledger that
  contains `PendingApproval` entries (deserialization fails).
- Migration: re-run trajectories that contained suspended proposals. Because
  proposals are re-compiled at runtime rather than persisted authoritatively,
  re-running with the new runtime produces semantically equivalent outcomes.
- Pre-RFC `Staged` and `Rejected` status values serialized as plain strings
  (`"staged"`, `"rejected"`). Post-RFC they serialize as tagged objects
  (`{"kind":"staged"}`, `{"kind":"rejected","reason":"..."}`). This is a wire
  format break for any `PendingApproval` payload.

## Security Considerations

- `ProposalId` is derived from `ProposalBody` only and the body contains no
  provider-supplied data, so a provider cannot influence a proposal's identity.
- The `Rejected { reason }` string in `ProposalStatus` is a human-readable
  summary and MUST NOT be used for authorization or policy decisions.
- There is no unauthenticated metadata on the contract: removing the experimental
  `routing_evidence` field eliminates a class of provider-supplied data that
  would otherwise have to be carefully prevented from influencing authority.

## Alternatives

**Embed routing/provider metadata in `ProposalBody`**: Rejected because it would
include provider routing decisions in `ProposalId`, coupling nondeterministic
provider behavior to the proposal's canonical identity and the hash chain.

**Keep an experimental `routing_evidence` field on `Proposal` (outside the
body)**: Rejected for v1. The runtime never read it, yet it sat on the
`PendingApproval` wire format and carried unresolved signing/storage questions —
an inert field on the stability surface. Removed; reintroduce by RFC only when a
concrete consumer and a signed design exist.

**Use `channel` and `reason` as separate fields on `Proposal` alongside
`status`**: Rejected because it duplicates data already carried by the status
variant. Embedding in `Suspended { channel, reason }` keeps the status
self-describing.

## Test Plan

- [x] `proposal_id_is_content_addressed`: same `ProposalBody` inputs → same ID
- [x] `different_tool_yields_different_id`: tool name change → different ID
- [x] `proposal_status_staged_serializes`: `{"kind":"staged"}`
- [x] `proposal_status_suspended_serializes`: embeds channel and reason
- [x] `proposal_status_rejected_serializes`: embeds reason
- [x] `proposal_status_roundtrips`: all three variants survive serde roundtrip
- [x] `proposal_id_is_stable_across_serialization_boundary`: serialize → deserialize → recompute id is stable
- [x] Compiler: `RequireApproval` decision → `Suspended { channel, reason }` status
- [x] Integration: `PendingApproval` ledger entry round-trips with the status format
- [x] Replay: pre-RFC ledger with `PendingApproval` entries returns a clear
  deserialization error rather than silently misfiring

## Unresolved Questions

None. The v1 contract is stable: `Proposal` carries only `id` and `body`, with
no experimental or provider-supplied fields. Provider routing metadata is
explicitly deferred to a future RFC and is not part of this contract's
compatibility surface.
