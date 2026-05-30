//! Commit: the only thing that mutates world state. Appended to the ledger.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::crypto::{PublicKey, SignatureBytes, SigningKey};
use crate::error::{Error, Result};
use crate::hash::{canonical_json_bytes, content_hash};
use crate::ids::{CommitId, IntentId, ProposalId, TrajectoryId, WritId};
use crate::proposal::PolicyTrace;

/// An observation captured from a tool execution. Persisted verbatim.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub tool: String,
    pub output: Value,
    pub latency_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Commit {
    pub id: CommitId,
    pub body: CommitBody,
}

impl Commit {
    pub fn new(body: CommitBody) -> Result<Self> {
        let id = CommitId(content_hash(&body)?);
        Ok(Commit { id, body })
    }

    /// Build a commit and sign it with `sk`. The signature covers
    /// `canonical_json(body)` with the `signature` field cleared, so it is
    /// independent of the signature value itself. The `CommitId` is then the
    /// content hash of the full (signed) body — exactly what the ledger's
    /// integrity check recomputes, so signed and unsigned commits both verify
    /// through the same hash-chain path.
    pub fn new_signed(mut body: CommitBody, sk: &SigningKey) -> Result<Self> {
        body.signature = None;
        let msg = canonical_json_bytes(&body)?;
        let sig = crate::crypto::sign(sk, &msg);
        body.signature = Some(hex::encode(sig));
        Self::new(body)
    }

    /// Verify this commit's ed25519 signature against `pk`. Errors if the
    /// commit is unsigned, the stored signature is malformed, or it does not
    /// verify over `canonical_json(body_without_signature)`.
    pub fn verify_signature(&self, pk: &PublicKey) -> Result<()> {
        let sig_hex = self
            .body
            .signature
            .as_ref()
            .ok_or_else(|| Error::AuthorityVoid("commit is unsigned".into()))?;
        let sig_bytes: SignatureBytes = hex::decode(sig_hex)
            .map_err(|e| Error::AuthorityVoid(format!("malformed commit signature hex: {e}")))?
            .try_into()
            .map_err(|_| Error::AuthorityVoid("commit signature must be 64 bytes".into()))?;
        let mut unsigned = self.body.clone();
        unsigned.signature = None;
        let msg = canonical_json_bytes(&unsigned)?;
        crate::crypto::verify(pk, &msg, &sig_bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitBody {
    /// Empty for the root commit of a trajectory; one for linear extension;
    /// multiple for merges.
    pub parent: Vec<CommitId>,
    pub trajectory_id: TrajectoryId,
    pub proposal_id: ProposalId,
    /// The Intent this commit's proposal was compiled from. Persisted so the
    /// permit path is auditable: `proposal_id` alone is a content hash of a
    /// Proposal that is never stored, and the Proposal cannot be recomputed
    /// without its source Intent. Recording `intent_id` (plus `policy_trace`
    /// below) makes a committed action explainable from the ledger alone.
    pub intent_id: IntentId,
    pub writ_id: WritId,
    /// Logical Lamport-style clock, monotonically increasing within a trajectory.
    pub seq: u64,
    pub delta: crate::delta::StructuredDelta,
    pub observations: Vec<Observation>,
    /// The policy decision + rules that authorized this commit, copied from the
    /// proposal. Without this, the *why* of a permitted action is lost the
    /// moment the (unstored) Proposal is dropped.
    pub policy_trace: PolicyTrace,
    pub compiler_version: String,
    /// Fingerprint of the full policy rule set that was in effect when this
    /// commit was produced (see `thymos_policy::PolicyEngine::policy_set_hash`).
    /// Lets replay detect policy drift the same way `compiler_version` detects
    /// compiler drift. Empty string for commits produced without an engine
    /// fingerprint (older fixtures / non-runtime callers).
    #[serde(default)]
    pub policy_set_hash: String,
    /// Budget cost incurred by this commit (tool_calls, tokens, wall_clock_ms, usd).
    pub budget_cost: crate::writ::BudgetCost,
    /// ed25519 signature over `canonical_json(body_without_signature)`,
    /// hex-encoded. `None` for unsigned commits; populated by
    /// [`Commit::new_signed`] and checked by [`Commit::verify_signature`].
    pub signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{generate_signing_key, public_key_of};
    use crate::delta::{DeltaOp, StructuredDelta};
    use crate::ids::WritId;
    use crate::proposal::{PolicyDecision, PolicyTrace};
    use crate::ContentHash;

    fn body() -> CommitBody {
        CommitBody {
            parent: vec![],
            trajectory_id: TrajectoryId::new_from_seed(b"sig-test"),
            proposal_id: ProposalId::ZERO,
            intent_id: IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            seq: 1,
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: "k".into(),
                value: serde_json::json!("v"),
            }),
            observations: vec![],
            policy_trace: PolicyTrace {
                rules_evaluated: vec![],
                decision: PolicyDecision::Permit,
            },
            compiler_version: "test".into(),
            policy_set_hash: String::new(),
            budget_cost: crate::writ::BudgetCost::default(),
            signature: None,
        }
    }

    #[test]
    fn signed_commit_verifies_with_correct_key() {
        let sk = generate_signing_key();
        let pk = public_key_of(&sk);
        let commit = Commit::new_signed(body(), &sk).unwrap();
        assert!(commit.body.signature.is_some(), "signature must be populated");
        commit.verify_signature(&pk).expect("must verify");
    }

    #[test]
    fn signed_commit_id_is_self_consistent() {
        // CommitId must be the content hash of the *signed* body, so the
        // ledger's integrity recomputation (which hashes the full payload)
        // matches what new_signed produced.
        let sk = generate_signing_key();
        let commit = Commit::new_signed(body(), &sk).unwrap();
        let recomputed = CommitId(content_hash(&commit.body).unwrap());
        assert_eq!(commit.id, recomputed);
    }

    #[test]
    fn tampered_body_fails_verification() {
        let sk = generate_signing_key();
        let pk = public_key_of(&sk);
        let mut commit = Commit::new_signed(body(), &sk).unwrap();
        commit.body.seq = 999; // tamper after signing
        assert!(commit.verify_signature(&pk).is_err());
    }

    #[test]
    fn wrong_key_fails_verification() {
        let sk = generate_signing_key();
        let other = public_key_of(&generate_signing_key());
        let commit = Commit::new_signed(body(), &sk).unwrap();
        assert!(commit.verify_signature(&other).is_err());
    }

    #[test]
    fn unsigned_commit_verification_errors() {
        let pk = public_key_of(&generate_signing_key());
        let commit = Commit::new(body()).unwrap();
        let err = commit.verify_signature(&pk).expect_err("unsigned must error");
        assert!(err.to_string().contains("unsigned"));
    }
}
