//! Typed, newtype-wrapped identifiers.
//!
//! Content-addressed IDs (spec Section 7):
//!   CommitId, IntentId, ProposalId — derived from blake3(canonical_json(body)).
//!   WritId — derived from blake3(canonical_json(writ_body)) at signing time.
//!   LedgerEntryId — alias for ContentHash; entries are hashed from their payload.
//!
//! TrajectoryId — seeded from a caller-supplied seed; not content-addressed.
//!
//! Same canonical payload inputs MUST always produce the same ID, across any
//! serialization boundary. See hash::canonical_json_bytes for encoding rules.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::hash::ContentHash;

macro_rules! content_id {
    ($name:ident, $tag:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub ContentHash);

        impl $name {
            pub const ZERO: Self = $name(ContentHash::ZERO);
            pub fn inner(&self) -> &ContentHash {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", $tag, self.0.short())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}:{}", $tag, self.0)
            }
        }
    };
}

content_id!(CommitId, "commit");
content_id!(IntentId, "intent");
content_id!(ProposalId, "proposal");

/// Random 32-byte opaque id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrajectoryId(pub ContentHash);

impl TrajectoryId {
    pub fn new_from_seed(seed: &[u8]) -> Self {
        TrajectoryId(ContentHash(*blake3::hash(seed).as_bytes()))
    }
}

impl fmt::Debug for TrajectoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "traj({})", self.0.short())
    }
}
impl fmt::Display for TrajectoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "traj:{}", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WritId(pub ContentHash);

impl WritId {
    pub fn new_from_seed(seed: &[u8]) -> Self {
        WritId(ContentHash(*blake3::hash(seed).as_bytes()))
    }
}

impl fmt::Debug for WritId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "writ({})", self.0.short())
    }
}
impl fmt::Display for WritId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "writ:{}", self.0)
    }
}

/// Ledger entry identities are content-addressed from their canonical payload
/// (spec Section 7). The underlying type is `ContentHash`; a distinct newtype
/// is not introduced here because Entry IDs flow through SQLite blob columns
/// and the ledger parent-chain as raw 32-byte values — a full newtype refactor
/// is tracked as a future protocol change requiring an RFC.
pub type LedgerEntryId = ContentHash;

// ── Determinism tests ─────────────────────────────────────────────────────────
//
// Spec Section 7: "Same inputs must always produce same ID across serialization
// boundaries."

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::content_hash;
    use crate::intent::IntentBody;
    use crate::intent::IntentKind;
    use crate::proposal::{ExecutionPlan, PolicyDecision, PolicyTrace, ProposalBody, ProposalStatus};
    use crate::commit::{CommitBody, Observation};
    use crate::delta::{DeltaOp, StructuredDelta};
    use crate::writ::BudgetCost;
    use crate::COMPILER_VERSION;

    fn intent_body_fixture() -> IntentBody {
        IntentBody {
            parent_commit: None,
            author: "agent".into(),
            kind: IntentKind::Act,
            target: "kv_set".into(),
            args: serde_json::json!({"key": "hello", "value": "world"}),
            rationale: "set greeting".into(),
            nonce: [1u8; 16],
        }
    }

    fn proposal_body_fixture() -> ProposalBody {
        ProposalBody {
            intent_id: IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            plan: ExecutionPlan {
                tool: "kv_set".into(),
                args: serde_json::json!({"key": "k"}),
            },
            policy_trace: PolicyTrace {
                rules_evaluated: vec!["writ.authority".into()],
                decision: PolicyDecision::Permit,
            },
            status: ProposalStatus::Staged,
        }
    }

    fn commit_body_fixture(traj: TrajectoryId) -> CommitBody {
        CommitBody {
            parent: vec![CommitId::ZERO],
            trajectory_id: traj,
            proposal_id: ProposalId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            seq: 1,
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: "x".into(),
                value: serde_json::json!("y"),
            }),
            observations: vec![Observation {
                tool: "kv_set".into(),
                output: serde_json::json!(null),
                latency_ms: 10,
            }],
            compiler_version: COMPILER_VERSION.into(),
            budget_cost: BudgetCost::default(),
            signature: None,
        }
    }

    #[test]
    fn intent_id_is_deterministic() {
        let b1 = intent_body_fixture();
        let b2 = intent_body_fixture();
        let h1 = content_hash(&b1).unwrap();
        let h2 = content_hash(&b2).unwrap();
        assert_eq!(h1, h2, "same IntentBody inputs must yield same hash");
    }

    #[test]
    fn intent_id_differs_on_changed_field() {
        let b1 = intent_body_fixture();
        let mut b2 = intent_body_fixture();
        b2.rationale = "different".into();
        assert_ne!(content_hash(&b1).unwrap(), content_hash(&b2).unwrap());
    }

    #[test]
    fn proposal_id_is_deterministic() {
        let b1 = proposal_body_fixture();
        let b2 = proposal_body_fixture();
        assert_eq!(content_hash(&b1).unwrap(), content_hash(&b2).unwrap());
    }

    #[test]
    fn proposal_id_differs_on_changed_tool() {
        let b1 = proposal_body_fixture();
        let mut b2 = proposal_body_fixture();
        b2.plan.tool = "other_tool".into();
        assert_ne!(content_hash(&b1).unwrap(), content_hash(&b2).unwrap());
    }

    #[test]
    fn commit_id_is_deterministic() {
        let traj = TrajectoryId::new_from_seed(b"det-test");
        let b1 = commit_body_fixture(traj);
        let b2 = commit_body_fixture(traj);
        assert_eq!(content_hash(&b1).unwrap(), content_hash(&b2).unwrap());
    }

    #[test]
    fn commit_id_differs_on_changed_seq() {
        let traj = TrajectoryId::new_from_seed(b"det-test2");
        let b1 = commit_body_fixture(traj);
        let mut b2 = commit_body_fixture(traj);
        b2.seq = 99;
        assert_ne!(content_hash(&b1).unwrap(), content_hash(&b2).unwrap());
    }

    #[test]
    fn nonce_makes_identical_intents_distinct() {
        let mut b1 = intent_body_fixture();
        let mut b2 = intent_body_fixture();
        b1.nonce = [0u8; 16];
        b2.nonce = [1u8; 16];
        assert_ne!(content_hash(&b1).unwrap(), content_hash(&b2).unwrap());
    }
}
