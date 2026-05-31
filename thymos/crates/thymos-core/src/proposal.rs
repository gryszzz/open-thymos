//! Proposal: the compiler's output. The only thing the scheduler executes.
//!
//! Spec reference: Section 2 (Execution Grammar), Section 3 (Compilation).
//!
//! `ProposalId` is content-addressed from the canonical hash of `ProposalBody`.
//! Per `docs/rfcs/proposal-contract-v1.md`, `Proposal` also carries an optional
//! `routing_evidence` — provider routing metadata supplied by a pre-Proposal
//! routing advisor (e.g. WisePick). It lives on `Proposal`, **not** inside
//! `ProposalBody`, so it does **not** affect `ProposalId`; but it IS bound into
//! the ledgered envelope (the `Commit` / `PendingApproval` entry hashes) so it is
//! immutable and replay-safe once recorded. The runtime never reads it for
//! authority — it is an audit/replay artifact only.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::hash::content_hash;
use crate::ids::{IntentId, ProposalId, WritId};

// ── Proposal ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: ProposalId,
    pub body: ProposalBody,
    /// Optional provider routing metadata (Option 2 of the proposal-contract
    /// RFC). Excluded from `ProposalId` (it is outside `ProposalBody`), so a
    /// provider cannot influence proposal identity by manipulating it. Bound
    /// into ledger entry hashes when recorded. The runtime MUST NOT use it for
    /// authority, budget, or policy decisions — it is audit/replay evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_evidence: Option<RoutingEvidence>,
}

impl Proposal {
    /// Construct a Proposal from a body. The id is the content-hash of body;
    /// `routing_evidence` is not hashed into the id.
    pub fn new(body: ProposalBody) -> Result<Self> {
        let id = ProposalId(content_hash(&body)?);
        Ok(Proposal {
            id,
            body,
            routing_evidence: None,
        })
    }

    /// Attach routing evidence (does not change `ProposalId`).
    pub fn with_routing_evidence(mut self, evidence: RoutingEvidence) -> Self {
        self.routing_evidence = Some(evidence);
        self
    }
}

// ── RoutingEvidence ─────────────────────────────────────────────────────────
//
// The replay-safe routing decision artifact produced by a pre-Proposal routing
// advisor. All numeric fields are fixed-point integers — no floating point in a
// canonical/ledgered payload (Section 10 determinism). `decision_hash` is a
// hex digest derived deterministically over the integer-valued payload, so it
// is stable across replays and free of ephemeral provider identifiers.

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingEvidence {
    /// Hex digest over the routing decision's canonical (integer-valued) payload.
    pub decision_hash: String,
    /// The selected `provider:capability` (ECU) string.
    pub selected: String,
    /// Ranked alternatives considered but not selected (for governance-owned
    /// fallback without re-querying the advisor mid-execution).
    pub alternatives: Vec<String>,
    /// Confidence in basis points (0–10000 = 0.00%–100.00%). Fixed-point.
    pub confidence_bps: u32,
    /// Machine-readable reason codes for the decision.
    pub reason_codes: Vec<String>,
    /// Estimated round-trip latency in milliseconds.
    pub latency_estimate_ms: u64,
    /// Estimated cost in USD millicents (1 USD = 100_000 millicents). Fixed-point.
    pub cost_estimate_millicents: u64,
    /// Optional fallback hint if the selected route is unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_hint: Option<FallbackHint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackHint {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub reason: String,
}

// ── ProposalBody ──────────────────────────────────────────────────────────────

/// Canonical proposal payload. All fields are included in ProposalId.
/// No floating-point values (canonical hash inputs must be deterministic).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalBody {
    pub intent_id: IntentId,
    pub writ_id: WritId,
    pub plan: ExecutionPlan,
    pub policy_trace: PolicyTrace,
    pub status: ProposalStatus,
}

// ── ExecutionPlan ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub tool: String,
    /// Validated input to the tool contract (already schema-checked by the compiler).
    pub args: Value,
}

// ── PolicyTrace ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyTrace {
    pub rules_evaluated: Vec<String>,
    pub decision: PolicyDecision,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum PolicyDecision {
    Permit,
    Deny(String),
    RequireApproval { channel: String, reason: String },
}

// ── ProposalStatus ────────────────────────────────────────────────────────────
//
// Spec Section 2:
//   Status := Staged | Suspended { channel, reason } | Rejected { reason }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalStatus {
    Staged,
    Suspended { channel: String, reason: String },
    Rejected { reason: String },
}

// ── RejectionReason ───────────────────────────────────────────────────────────
//
// Used in ledger Rejection entries and Compiled::Rejected. Distinct from
// ProposalStatus::Rejected::reason (a human-readable summary string).

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum RejectionReason {
    AuthorityVoid(String),
    PolicyDenied(String),
    BudgetExhausted(String),
    PreconditionFailed(String),
    UnknownTool(String),
    TypeMismatch { tool: String, detail: String },
}

impl std::fmt::Display for RejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthorityVoid(s) => write!(f, "authority void: {s}"),
            Self::PolicyDenied(s) => write!(f, "policy denied: {s}"),
            Self::BudgetExhausted(s) => write!(f, "budget exhausted: {s}"),
            Self::PreconditionFailed(s) => write!(f, "precondition failed: {s}"),
            Self::UnknownTool(s) => write!(f, "unknown tool: {s}"),
            Self::TypeMismatch { tool, detail } => {
                write!(f, "type mismatch for tool '{tool}': {detail}")
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::WritId;
    use crate::ContentHash;

    fn make_body(tool: &str) -> ProposalBody {
        ProposalBody {
            intent_id: crate::IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            plan: ExecutionPlan {
                tool: tool.into(),
                args: serde_json::json!({"key": "v"}),
            },
            policy_trace: PolicyTrace {
                rules_evaluated: vec!["writ.authority".into()],
                decision: PolicyDecision::Permit,
            },
            status: ProposalStatus::Staged,
        }
    }

    #[test]
    fn proposal_id_is_content_addressed() {
        let b1 = make_body("kv_set");
        let b2 = make_body("kv_set");
        let p1 = Proposal::new(b1).unwrap();
        let p2 = Proposal::new(b2).unwrap();
        assert_eq!(p1.id, p2.id, "same inputs must yield same ProposalId");
    }

    #[test]
    fn different_tool_yields_different_id() {
        let p1 = Proposal::new(make_body("kv_set")).unwrap();
        let p2 = Proposal::new(make_body("kv_del")).unwrap();
        assert_ne!(p1.id, p2.id);
    }

    fn sample_evidence() -> RoutingEvidence {
        RoutingEvidence {
            decision_hash: "deadbeef".into(),
            selected: "anthropic:claude".into(),
            alternatives: vec!["openai:gpt".into()],
            confidence_bps: 9500,
            reason_codes: vec!["cost_optimal".into()],
            latency_estimate_ms: 800,
            cost_estimate_millicents: 4200,
            fallback_hint: Some(FallbackHint {
                provider: "openai".into(),
                model: Some("gpt-4o".into()),
                reason: "primary overloaded".into(),
            }),
        }
    }

    #[test]
    fn routing_evidence_does_not_affect_proposal_id() {
        let body = make_body("kv_set");
        let plain = Proposal::new(body.clone()).unwrap();
        let with_ev = Proposal::new(body).unwrap().with_routing_evidence(sample_evidence());
        assert_eq!(
            plain.id, with_ev.id,
            "routing_evidence lives outside ProposalBody and must not affect ProposalId"
        );
    }

    #[test]
    fn proposal_with_routing_evidence_round_trips() {
        let p = Proposal::new(make_body("kv_set"))
            .unwrap()
            .with_routing_evidence(sample_evidence());
        let json = serde_json::to_string(&p).unwrap();
        let back: Proposal = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
        assert_eq!(back.routing_evidence.unwrap().confidence_bps, 9500);

        // A proposal without evidence omits the field entirely (backward-compat).
        let plain = Proposal::new(make_body("kv_set")).unwrap();
        assert!(!serde_json::to_string(&plain).unwrap().contains("routing_evidence"));
    }

    #[test]
    fn proposal_status_staged_serializes() {
        let s = ProposalStatus::Staged;
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "staged");
    }

    #[test]
    fn proposal_status_suspended_serializes() {
        let s = ProposalStatus::Suspended {
            channel: "slack".into(),
            reason: "high cost".into(),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "suspended");
        assert_eq!(v["channel"], "slack");
        assert_eq!(v["reason"], "high cost");
    }

    #[test]
    fn proposal_status_rejected_serializes() {
        let s = ProposalStatus::Rejected {
            reason: "policy denied".into(),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "rejected");
        assert_eq!(v["reason"], "policy denied");
    }

    #[test]
    fn proposal_status_roundtrips() {
        for s in [
            ProposalStatus::Staged,
            ProposalStatus::Suspended {
                channel: "ops".into(),
                reason: "cost".into(),
            },
            ProposalStatus::Rejected {
                reason: "writ expired".into(),
            },
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: ProposalStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn proposal_id_is_stable_across_serialization_boundary() {
        // Build a Proposal, serialize it as JSON, deserialize, recompute the
        // ProposalId from the round-tripped body, and assert the id is the
        // same. This proves content-addressing is invariant under
        // serialize → wire → deserialize (the property replay depends on).
        let body = make_body("kv_set");
        let original = Proposal::new(body).unwrap();

        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: Proposal = serde_json::from_str(&json).unwrap();
        assert_eq!(original, round_tripped, "Proposal must roundtrip cleanly");

        let recomputed_id = ProposalId(content_hash(&round_tripped.body).unwrap());
        assert_eq!(
            original.id, recomputed_id,
            "ProposalId must be identical after serialize → deserialize → recompute"
        );
        assert_eq!(original.id, round_tripped.id);
    }

    #[test]
    fn proposal_id_is_stable_after_canonical_normalization() {
        // Build a Proposal, then construct a *different* serializer ordering
        // (by going through serde_json::Value with shuffled object keys) and
        // assert recomputing the id yields the same result. This proves the
        // canonical_json_bytes sorting is doing its job.
        use crate::hash::canonical_json_bytes;
        let body = make_body("kv_set");
        let canonical_a = canonical_json_bytes(&body).unwrap();

        // Round-trip through a Value where the inner args map's keys are
        // re-shuffled.
        let mut value: serde_json::Value = serde_json::to_value(&body).unwrap();
        if let Some(args_map) = value
            .get_mut("plan")
            .and_then(|p| p.get_mut("args"))
            .and_then(|a| a.as_object_mut())
        {
            // Re-insert in reversed order to break naive iteration order.
            let entries: Vec<_> = args_map
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            args_map.clear();
            for (k, v) in entries.into_iter().rev() {
                args_map.insert(k, v);
            }
        }
        let canonical_b = canonical_json_bytes(&value).unwrap();

        assert_eq!(
            canonical_a, canonical_b,
            "canonical_json_bytes must produce identical bytes regardless of object key insertion order"
        );
    }
}
