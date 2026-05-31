//! External anchoring: a Merkle commitment over a trajectory's ledger entries.
//!
//! The hash chain proves a ledger is *internally* consistent, but a party with
//! write access could rewrite the whole chain. An anchor is a single small
//! value — the Merkle root over the ordered entry ids — that an operator can
//! publish somewhere outside the runtime's control (a transparency log, a git
//! tag, a notary, another chain). Later, anyone holding the ledger can recompute
//! the root and prove it was not rewritten since the anchor was published.

use serde::{Deserialize, Serialize};

use thymos_core::{ContentHash, Result, TrajectoryId};

use crate::Entry;

/// A publishable commitment to a trajectory's history at a point in time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleAnchor {
    pub trajectory_id: TrajectoryId,
    /// Number of entries covered by `root`.
    pub entry_count: usize,
    /// Sequence number of the last covered entry.
    pub head_seq: u64,
    /// Merkle root over the ordered entry ids.
    pub root: ContentHash,
}

/// Compute the Merkle root over `entries` (ordered by seq). Leaves are the
/// entry ids (already BLAKE3 content hashes); internal nodes hash the
/// concatenation of their children, duplicating the last node when a level has
/// an odd count. Empty input yields `ContentHash::ZERO`.
pub fn merkle_root(entries: &[Entry]) -> ContentHash {
    if entries.is_empty() {
        return ContentHash::ZERO;
    }
    let mut level: Vec<[u8; 32]> = entries.iter().map(|e| *e.id.as_bytes()).collect();
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            let left = &pair[0];
            // Duplicate the last node when the level is odd (RFC 6962-style
            // would carry it up; duplication is simpler and sufficient here as
            // long as verification uses the same rule).
            let right = if pair.len() == 2 { &pair[1] } else { &pair[0] };
            let mut hasher = blake3::Hasher::new();
            hasher.update(left);
            hasher.update(right);
            next.push(*hasher.finalize().as_bytes());
        }
        level = next;
    }
    ContentHash(level[0])
}

/// Build an anchor over `entries`. Callers should pass the full, integrity-
/// verified entry list for a trajectory (see `Ledger::anchor`).
pub fn compute_anchor(trajectory_id: TrajectoryId, entries: &[Entry]) -> MerkleAnchor {
    MerkleAnchor {
        trajectory_id,
        entry_count: entries.len(),
        head_seq: entries.last().map(|e| e.seq).unwrap_or(0),
        root: merkle_root(entries),
    }
}

/// Verify that `entries` still match a previously published `anchor`. Fails if
/// the root, the entry count, or the head sequence diverged — i.e. the ledger
/// was rewritten or truncated since the anchor was taken.
pub fn verify_anchor(entries: &[Entry], anchor: &MerkleAnchor) -> Result<()> {
    let recomputed = compute_anchor(anchor.trajectory_id, entries);
    if recomputed.root != anchor.root
        || recomputed.entry_count != anchor.entry_count
        || recomputed.head_seq != anchor.head_seq
    {
        return Err(thymos_core::error::Error::Invariant(format!(
            "anchor mismatch: published root {} ({} entries, head {}), recomputed {} ({} entries, head {})",
            anchor.root, anchor.entry_count, anchor.head_seq,
            recomputed.root, recomputed.entry_count, recomputed.head_seq
        )));
    }
    Ok(())
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use thymos_core::{
        commit::{Commit, CommitBody, Observation},
        delta::{DeltaOp, StructuredDelta},
        ids::{ProposalId, WritId},
        proposal::{PolicyDecision, PolicyTrace},
        ContentHash, IntentId, COMPILER_VERSION,
    };

    use crate::Ledger;

    fn append_kv(ledger: &Ledger, traj: TrajectoryId, key: &str, seq: u64) {
        let body = CommitBody {
            parent: vec![],
            trajectory_id: traj,
            proposal_id: ProposalId::ZERO,
            intent_id: IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            seq,
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: key.into(),
                value: serde_json::json!("v"),
            }),
            observations: vec![Observation {
                tool: "kv_set".into(),
                output: serde_json::json!(null),
                latency_ms: 0,
            }],
            policy_trace: PolicyTrace {
                rules_evaluated: vec![],
                decision: PolicyDecision::Permit,
            },
            compiler_version: COMPILER_VERSION.into(),
            policy_set_hash: String::new(),
            budget_cost: thymos_core::writ::BudgetCost::default(),
            compensates: None,
            routing_evidence: None,
            signature: None,
        };
        ledger.append_commit(Commit::new(body).unwrap()).unwrap();
    }

    #[test]
    fn anchor_round_trips_and_detects_divergence() {
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"anchor-1");
        ledger.append_root(traj, "test").unwrap();
        append_kv(&ledger, traj, "a", 1);
        append_kv(&ledger, traj, "b", 2);

        let anchor = ledger.anchor(traj).unwrap();
        assert_eq!(anchor.entry_count, 3, "root + 2 commits");
        assert_eq!(anchor.head_seq, 2);
        assert_ne!(anchor.root, ContentHash::ZERO);

        // Re-deriving from the same entries verifies.
        let entries = ledger.entries(traj).unwrap();
        verify_anchor(&entries, &anchor).expect("anchor must verify against its own entries");

        // A later commit moves the chain forward → the old anchor no longer
        // matches (count/head/root diverge).
        append_kv(&ledger, traj, "c", 3);
        let grown = ledger.entries(traj).unwrap();
        assert!(
            verify_anchor(&grown, &anchor).is_err(),
            "anchor must reject a grown/rewritten ledger"
        );

        // A fresh anchor over the grown ledger verifies again.
        let anchor2 = ledger.anchor(traj).unwrap();
        verify_anchor(&grown, &anchor2).unwrap();
        assert_ne!(anchor.root, anchor2.root);
    }

    #[test]
    fn empty_ledger_root_is_zero() {
        assert_eq!(merkle_root(&[]), ContentHash::ZERO);
    }
}
