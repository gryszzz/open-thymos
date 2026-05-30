# OpenThymos RFC

## Title

Proposal Contract v1

## Status

Accepted

## Summary

This RFC formalizes the proposal contract as it exists after Phase I stabilization.
It specifies the canonical `Proposal` type, the `ProposalStatus` enumeration, the
`RoutingEvidence` supplementary field, and the content-addressing rules for
`ProposalId`. It affects runtime semantics (compilation output), ledger
compatibility (serialized `PendingApproval` payloads), and provider semantics
(routing metadata boundary). It does not affect replay correctness for entries
that contain no `PendingApproval` payloads.

## Motivation

Before this RFC, `ProposalStatus` had unit variants that did not carry the
`channel` and `reason` fields mandated by Section 2 of the specification. The
`Suspended` variant was named `SuspendedForApproval` and carried no associated
data; the `Rejected` variant had no reason field. Both mismatched the grammar:

```text
Status := Staged | Suspended { channel, reason } | Rejected { reason }
```

Additionally, the specification allows providers to attach routing metadata that
influences capability registry resolution (step 5 of compilation) without
granting additional authority. No type existed to carry this metadata, leaving
providers with no protocol-compliant way to communicate routing decisions to the
compiler.

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_evidence: Option<RoutingEvidence>,
}

pub struct RoutingEvidence {
    pub decision_hash: String,
    pub selected: String,
    pub alternatives: Vec<String>,
    pub confidence: u32,           // basis points, 0–10000
    pub reason_codes: Vec<String>,
    pub latency_estimate_ms: u64,
    pub cost_estimate_usd: u64,    // USD millicents
    pub fallback_hint: Option<FallbackHint>,
}

pub struct FallbackHint {
    pub provider: String,
    pub model: Option<String>,
    pub reason: String,
}
```

`ProposalId` remains `content_hash(body)`. `routing_evidence` is NOT part of
`ProposalBody` and therefore does NOT affect `ProposalId`.

The `Suspended` variant now embeds `channel` and `reason` directly in the
status, matching Section 2. The compiler populates both fields from the policy
engine's `RequireApproval` decision; the runtime reads them from the status
when appending `PendingApproval` ledger entries.

`confidence` and `cost_estimate_usd` use fixed-point integers to avoid
floating-point values in canonical data paths (Section 10 requires deterministic
serialization).

## Invariants

- `ProposalId = blake3(canonical_json(ProposalBody))`.
- `routing_evidence` MUST NOT influence ProposalId.
- `routing_evidence` MUST NOT influence writ authority, budget checks, policy
  decisions, or ledger entry hashes for any entry kind.
- `routing_evidence` MAY influence step 5 (capability registry resolution) of
  the compilation pipeline, and only step 5.
- Provider adapters MUST NOT use `routing_evidence` to grant additional tool
  authority, bypass policy, or mutate ledger state.
- `ProposalStatus::Suspended` MUST carry the same `channel` and `reason` values
  that appear in the `PendingApproval` ledger entry.
- `ProposalStatus::Rejected::reason` is a human-readable summary string; it is
  distinct from `RejectionReason`, which is the structured enum used in
  `Rejection` ledger entries.
- Floating-point values MUST NOT appear in any type that is part of a canonical
  payload hash input. `RoutingEvidence` fields use fixed-point integers.

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

Providers that supply routing metadata MAY attach a `RoutingEvidence` value to
the `Proposal` via `Proposal::with_routing_evidence`. They MUST NOT use this
field to influence compilation authority. The runtime MUST ignore
`routing_evidence` for all purposes other than step 5 capability registry
resolution.

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

- `routing_evidence` is unauthenticated supplementary data. A malicious or
  misconfigured provider could supply false routing metadata. The runtime MUST
  enforce that `routing_evidence` cannot influence authority, budget, policy, or
  ledger write access — only step 5 registry resolution.
- `decision_hash` in `RoutingEvidence` is informational only. The runtime MUST
  NOT use it as an authority proof.
- `ProposalId` continues to be derived from `ProposalBody` only. A provider
  cannot influence `ProposalId` by manipulating `routing_evidence`.
- The `Rejected { reason }` string in `ProposalStatus` is a human-readable
  summary and MUST NOT be used for authorization or policy decisions.

## Alternatives

**Embed `routing_evidence` in `ProposalBody`**: Rejected because this would
include routing metadata in `ProposalId`, making provider routing decisions part
of the proposal's canonical identity. This would couple provider behavior to the
hash chain and complicate determinism guarantees.

**Use `channel` and `reason` as separate fields on `Proposal` alongside
`status`**: Rejected because it duplicates data already carried by the status
variant. Embedding in `Suspended { channel, reason }` keeps the status
self-describing.

**Floating-point `confidence` and `cost_estimate_usd`**: Rejected because
floating-point JSON serialization is not guaranteed to be identical across
platforms or Rust releases. Fixed-point integers (basis points, millicents)
are deterministic and follow the pattern established by `Budget` fields.

## Test Plan

- [x] `proposal_id_is_content_addressed`: same `ProposalBody` inputs → same ID
- [x] `different_tool_yields_different_id`: tool name change → different ID
- [x] `routing_evidence_does_not_affect_id`: presence of evidence → same ID
- [x] `proposal_status_staged_serializes`: `{"kind":"staged"}`
- [x] `proposal_status_suspended_serializes`: embeds channel and reason
- [x] `proposal_status_rejected_serializes`: embeds reason
- [x] `proposal_status_roundtrips`: all three variants survive serde roundtrip
- [x] Compiler: `RequireApproval` decision → `Suspended { channel, reason }` status
- [x] Integration: `PendingApproval` ledger entry round-trips with new status format
- [x] Replay: pre-RFC ledger with `PendingApproval` entries returns a clear
  deserialization error rather than silently misfiring

## Unresolved Questions

These questions do not affect **replay correctness**: `routing_evidence` lives on
`Proposal` (not `ProposalBody`), is excluded from `ProposalId`, is not part of any
commit delta, and is never folded by the replay verifier.

They are **not** compatibility-neutral, however. `routing_evidence` is serialized
inside `PendingApproval` ledger payloads, so its on-wire shape is part of the
ledger compatibility surface, and answering either question later will change that
wire format. For that reason `routing_evidence` is marked **experimental** and is
**excluded from the v1 compatibility guarantee** (see `thymos-core`
`Proposal::routing_evidence`). It is also currently **inert** — the runtime does
not read it.

- Should `routing_evidence` be signed by the provider to provide an audit trail?
  Until resolved, `routing_evidence` is unauthenticated (its `decision_hash` is
  provider-self-asserted) and MUST NOT be surfaced as a trustworthy audit
  artifact. Adding a signature will add fields and break the `PendingApproval`
  wire format. (Phase II concern, requires RFC.)
- Should the ledger store `routing_evidence` separately from the `Proposal` to
  avoid bloating `PendingApproval` payloads with supplementary data? Moving it
  will also break the `PendingApproval` wire format. (Phase III ledger segment
  format RFC.)
- If `routing_evidence` is ever allowed to influence step 5 (capability registry
  resolution), an unauthenticated provider field would influence execution. That
  must be resolved (at minimum, signing) **before** any step-5 use is enabled.

A v1 "stable" promise MUST NOT silently cover `routing_evidence` until the
signing and storage questions are closed.
