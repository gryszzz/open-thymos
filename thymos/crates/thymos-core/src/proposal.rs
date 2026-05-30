//! Proposal: the compiler's output. The only thing the scheduler executes.
//!
//! Spec reference: Section 2 (Execution Grammar), Section 3 (Compilation).
//!
//! ProposalId is content-addressed from the canonical hash of ProposalBody.
//! routing_evidence lives on Proposal, NOT inside ProposalBody, so it does
//! not affect ProposalId. It is supplementary data that influences only
//! step 5 (capability registry resolution) per Section 3.

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
    /// Supplementary routing metadata — influences step 5 capability registry
    /// resolution only. Never affects ProposalId, authority, policy, or replay
    /// semantics. See Section 9 for provider authority boundary.
    ///
    /// NOTE (Phase I): the runtime does not currently read this field. It is
    /// reserved per RFC `proposal-contract-v1.md` for provider routing layers
    /// that wish to surface their decision without crossing the authority
    /// boundary. See open question Q1 in the Phase I audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_evidence: Option<RoutingEvidence>,
}

impl Proposal {
    /// Construct a Proposal from a body. The id is the content-hash of body;
    /// routing_evidence is not hashed.
    pub fn new(body: ProposalBody) -> Result<Self> {
        let id = ProposalId(content_hash(&body)?);
        Ok(Proposal {
            id,
            body,
            routing_evidence: None,
        })
    }

    pub fn with_routing_evidence(mut self, evidence: RoutingEvidence) -> Self {
        self.routing_evidence = Some(evidence);
        self
    }
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

// ── RoutingEvidence ───────────────────────────────────────────────────────────
//
// Supplementary metadata from the provider routing layer. Lives on Proposal
// (not ProposalBody) and MUST NOT affect authority, policy, or ledger hashes.
// Influences only compiler step 5 (capability registry resolution).

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingEvidence {
    /// Hex-encoded content digest of the full routing decision payload, for
    /// audit. The runtime treats this as opaque; the project's canonical hash
    /// is BLAKE3 (see `crate::hash`), but providers MAY use any hex digest as
    /// long as it is stable across their routing decisions.
    pub decision_hash: String,
    /// The provider:model string the router selected.
    pub selected: String,
    /// Other provider:model strings considered but not selected.
    pub alternatives: Vec<String>,
    /// Confidence in the selection, in basis points (0–10000 = 0.00%–100.00%).
    /// Fixed-point to avoid floating-point in canonical data paths.
    pub confidence: u32,
    /// Machine-readable reason codes explaining the routing decision.
    pub reason_codes: Vec<String>,
    /// Estimated provider round-trip latency in milliseconds.
    pub latency_estimate_ms: u64,
    /// Estimated cost in USD millicents (1 USD = 100_000 millicents).
    pub cost_estimate_usd: u64,
    /// Fallback provider hint if the selected provider is unavailable.
    pub fallback_hint: Option<FallbackHint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackHint {
    pub provider: String,
    pub model: Option<String>,
    pub reason: String,
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

    #[test]
    fn routing_evidence_does_not_affect_id() {
        let body = make_body("kv_set");
        let p_no_evidence = Proposal::new(body.clone()).unwrap();
        let p_with_evidence = Proposal::new(body)
            .unwrap()
            .with_routing_evidence(RoutingEvidence {
                decision_hash: "abc123".into(),
                selected: "anthropic:claude-opus-4-7".into(),
                alternatives: vec!["openai:gpt-4o".into()],
                confidence: 9500,
                reason_codes: vec!["cost_optimal".into()],
                latency_estimate_ms: 800,
                cost_estimate_usd: 42,
                fallback_hint: Some(FallbackHint {
                    provider: "openai".into(),
                    model: Some("gpt-4o".into()),
                    reason: "primary overloaded".into(),
                }),
            });
        assert_eq!(
            p_no_evidence.id, p_with_evidence.id,
            "routing_evidence must not affect ProposalId"
        );
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
    fn proposal_id_is_stable_with_routing_evidence_round_trip() {
        // The same property must hold even when routing_evidence is attached
        // — it is serialized along with the Proposal but excluded from the id.
        let body = make_body("kv_set");
        let original = Proposal::new(body)
            .unwrap()
            .with_routing_evidence(RoutingEvidence {
                decision_hash: "deadbeef".into(),
                selected: "anthropic:opus".into(),
                alternatives: vec![],
                confidence: 8800,
                reason_codes: vec!["primary".into()],
                latency_estimate_ms: 200,
                cost_estimate_usd: 17,
                fallback_hint: None,
            });

        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: Proposal = serde_json::from_str(&json).unwrap();

        let recomputed_id = ProposalId(content_hash(&round_tripped.body).unwrap());
        assert_eq!(
            original.id, recomputed_id,
            "routing_evidence must not affect ProposalId after roundtrip"
        );
        assert_eq!(original.routing_evidence, round_tripped.routing_evidence);
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
