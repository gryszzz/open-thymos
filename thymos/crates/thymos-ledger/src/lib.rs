//! Thymos Ledger: append-only, content-addressed, parent-chained storage.
//!
//! Supports two backends:
//!   - **SQLite** (default, feature `sqlite`) — single-process, zero-config
//!   - **Postgres** (feature `postgres`) — multi-node, production-grade
//!
//! Both backends share the same entry/payload types and integrity guarantees:
//!   * Append-only — rows are never updated
//!   * Content-addressed — `id = blake3(canonical_json(payload))`
//!   * Parent-chained — every non-root entry references its parent
//!   * Typed kinds: Root, Commit, Rejection, PendingApproval, Delegation, Branch

use serde::{Deserialize, Serialize};

use thymos_core::{
    commit::Commit,
    content_hash,
    ids::IntentId,
    proposal::{Proposal, RejectionReason},
    CommitId, ContentHash, Error, Result, TrajectoryId,
};

// Backend modules.
pub mod anchor;
#[cfg(feature = "postgres")]
pub mod postgres;
pub mod replay;
#[cfg(feature = "sqlite")]
pub mod sqlite;

pub use anchor::{compute_anchor, merkle_root, verify_anchor, MerkleAnchor};
pub use replay::{replay, replay_and_match, ReplayConfig, ReplayReport};

// Re-export the default backend as `Ledger`.
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteLedger as Ledger;

/// A typed ledger entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: ContentHash,
    pub trajectory_id: TrajectoryId,
    pub parent: Option<ContentHash>,
    pub seq: u64,
    pub kind: EntryKind,
    pub payload: EntryPayload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Root,
    Commit,
    Rejection,
    PendingApproval,
    Delegation,
    Branch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntryPayload {
    Root {
        /// The trajectory this genesis entry begins. Binding it into the
        /// payload makes the root entry id (a content hash of the payload)
        /// unique per trajectory, so two runs created with the same `note`
        /// against one shared ledger don't collide on `entries.id`.
        trajectory_id: TrajectoryId,
        note: String,
    },
    Commit(Commit),
    Rejection {
        intent_id: IntentId,
        reason: RejectionReason,
    },
    PendingApproval {
        proposal: Proposal,
        channel: String,
        reason: String,
    },
    Delegation {
        child_trajectory_id: TrajectoryId,
        task: String,
        final_answer: Option<String>,
    },
    Branch {
        source_trajectory_id: TrajectoryId,
        source_commit_id: CommitId,
        note: String,
    },
}

// ---- Shared helpers used by both backends ----

pub(crate) fn build_entry(
    trajectory_id: TrajectoryId,
    parent: Option<ContentHash>,
    seq: u64,
    kind: EntryKind,
    payload: EntryPayload,
) -> Result<Entry> {
    let id = content_hash(&payload)?;
    Ok(Entry {
        id,
        trajectory_id,
        parent,
        seq,
        kind,
        payload,
    })
}

pub(crate) fn kind_to_str(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Root => "root",
        EntryKind::Commit => "commit",
        EntryKind::Rejection => "rejection",
        EntryKind::PendingApproval => "pending_approval",
        EntryKind::Delegation => "delegation",
        EntryKind::Branch => "branch",
    }
}

pub(crate) fn str_to_kind(s: &str) -> Result<EntryKind> {
    match s {
        "root" => Ok(EntryKind::Root),
        "commit" => Ok(EntryKind::Commit),
        "rejection" => Ok(EntryKind::Rejection),
        "pending_approval" => Ok(EntryKind::PendingApproval),
        "delegation" => Ok(EntryKind::Delegation),
        "branch" => Ok(EntryKind::Branch),
        other => Err(Error::Ledger(format!("unknown entry kind: {other}"))),
    }
}

/// Verify integrity of a sequence of entries (used by both backends).
///
/// Enforces spec Section 7 invariants:
///   * every entry id == blake3(canonical_json(payload))
///   * the first entry is a `Root` or `Branch`, has `seq == 0` and `parent == None`
///   * all entries share the same `trajectory_id`
///   * `seq` is contiguous (prev + 1)
///   * each non-root `parent` equals the previous entry's `id`
pub(crate) fn verify_integrity_entries(entries: &[Entry]) -> Result<()> {
    let mut prev_seq: Option<u64> = None;
    let mut prev_id: Option<ContentHash> = None;
    let mut expected_trajectory: Option<TrajectoryId> = None;
    for (idx, e) in entries.iter().enumerate() {
        // Hash chain: claimed id must equal recomputed payload hash.
        let recomputed = content_hash(&e.payload)?;
        if e.id != recomputed {
            return Err(Error::Invariant(format!(
                "hash mismatch at seq {}: claimed {} vs recomputed {}",
                e.seq, e.id, recomputed
            )));
        }
        // Trajectory cohesion.
        match expected_trajectory {
            None => expected_trajectory = Some(e.trajectory_id),
            Some(t) if t == e.trajectory_id => {}
            Some(t) => {
                return Err(Error::Invariant(format!(
                    "entry at seq {} belongs to trajectory {} but earlier entries belonged to {}",
                    e.seq, e.trajectory_id, t
                )));
            }
        }
        // The genesis payload commits to the trajectory it begins. Enforce that
        // it equals the entry's own trajectory column — otherwise a root row
        // relabeled/restored under a different trajectory id would still verify
        // (the payload hash matches the unchanged payload, and cohesion only
        // checks the column), defeating the binding this field exists to give.
        if let EntryPayload::Root {
            trajectory_id: claimed,
            ..
        } = &e.payload
        {
            if *claimed != e.trajectory_id {
                return Err(Error::Invariant(format!(
                    "root payload trajectory {} does not match entry trajectory {}",
                    claimed, e.trajectory_id
                )));
            }
        }
        // Root invariants: the first entry of any verified trajectory MUST
        // start the chain. Allowed kinds are Root (fresh trajectory) and
        // Branch (forked from a source trajectory).
        if idx == 0 {
            match e.kind {
                EntryKind::Root | EntryKind::Branch => {}
                other => {
                    return Err(Error::Invariant(format!(
                        "first entry must be Root or Branch, found {:?}",
                        other
                    )));
                }
            }
            if e.seq != 0 {
                return Err(Error::Invariant(format!(
                    "first entry must have seq 0, found {}",
                    e.seq
                )));
            }
            if e.parent.is_some() {
                return Err(Error::Invariant(
                    "first entry must have parent=None".into(),
                ));
            }
        } else if let (Some(ps), Some(pid)) = (prev_seq, prev_id) {
            if e.seq != ps + 1 {
                return Err(Error::Invariant(format!(
                    "non-contiguous seq: {} after {}",
                    e.seq, ps
                )));
            }
            if e.parent != Some(pid) {
                return Err(Error::Invariant("parent mismatch".into()));
            }
        }
        prev_seq = Some(e.seq);
        prev_id = Some(e.id);
    }
    Ok(())
}

/// Extension helper: pull every Commit from a trajectory for projection.
pub fn project_commits(entries: &[Entry]) -> Vec<&Commit> {
    entries
        .iter()
        .filter_map(|e| match &e.payload {
            EntryPayload::Commit(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// A flattened audit-friendly entry with hex IDs and a timestamp.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub trajectory_id: String,
    pub seq: u64,
    pub kind: String,
    pub payload: EntryPayload,
    pub created_at: u64,
}

impl Entry {
    pub fn commit_id(&self) -> Option<CommitId> {
        match &self.payload {
            EntryPayload::Commit(c) => Some(c.id),
            _ => None,
        }
    }
}

/// The storage-and-query surface shared by every ledger backend.
///
/// This is the abstraction the runtime is intended to depend on, so the agent
/// loop can be written once against `LedgerStore` and bound to a concrete
/// backend (SQLite today; a Postgres facade under Phase III) at construction.
///
/// It is **purely additive**: every method mirrors an existing inherent method
/// on the concrete backends, so introducing the trait changes no behavior. The
/// `append_*`, `head`, `entries`, `query_entries`, and `count_entries` methods
/// are the storage primitives each backend must provide. The derived
/// operations — `has_trajectory`, `verify_integrity`, and `anchor` — are given
/// as defaults in terms of those primitives, because every backend computes
/// them identically over the same shared entry types.
///
/// `Send + Sync` is required because the runtime shares one ledger across
/// concurrent tasks; the in-tree backends already satisfy it.
pub trait LedgerStore: Send + Sync {
    /// Append the genesis (`Root`) entry that begins a trajectory.
    fn append_root(&self, trajectory_id: TrajectoryId, note: &str) -> Result<Entry>;

    /// Append a `Commit`, enforcing seq/parent continuity against the head.
    fn append_commit(&self, commit: Commit) -> Result<Entry>;

    /// Append a `Rejection` recording why an intent was refused.
    fn append_rejection(
        &self,
        trajectory_id: TrajectoryId,
        intent_id: IntentId,
        reason: RejectionReason,
    ) -> Result<Entry>;

    /// Append a `PendingApproval` for a suspended proposal awaiting a human.
    fn append_pending_approval(
        &self,
        trajectory_id: TrajectoryId,
        proposal: Proposal,
        channel: String,
        reason: String,
    ) -> Result<Entry>;

    /// Append a `Delegation` linking a parent trajectory to a child one.
    fn append_delegation(
        &self,
        trajectory_id: TrajectoryId,
        child_trajectory_id: TrajectoryId,
        task: &str,
        final_answer: Option<String>,
    ) -> Result<Entry>;

    /// Append the genesis `Branch` entry for a trajectory forked from another.
    fn append_branch_root(
        &self,
        new_trajectory_id: TrajectoryId,
        source_trajectory_id: TrajectoryId,
        source_commit_id: CommitId,
        note: &str,
    ) -> Result<Entry>;

    /// Current head `(id, seq)` for a trajectory, or an error if it has none.
    fn head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)>;

    /// All entries for a trajectory in `seq` order.
    fn entries(&self, trajectory_id: TrajectoryId) -> Result<Vec<Entry>>;

    /// Query entries across trajectories with optional filters (see the
    /// backend method for the precise filter semantics).
    fn query_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>>;

    /// Count entries matching the given filters.
    fn count_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Result<u64>;

    /// Whether a trajectory has been rooted. Derived from [`head`](Self::head).
    fn has_trajectory(&self, trajectory_id: TrajectoryId) -> bool {
        self.head(trajectory_id).is_ok()
    }

    /// Verify the hash-chain / seq / parent integrity of a trajectory.
    /// Derived from [`entries`](Self::entries).
    fn verify_integrity(&self, trajectory_id: TrajectoryId) -> Result<()> {
        verify_integrity_entries(&self.entries(trajectory_id)?)
    }

    /// Produce a publishable [`MerkleAnchor`] over a verified trajectory.
    /// Integrity is checked first, so an anchor is only taken over a valid
    /// chain. Derived from [`entries`](Self::entries).
    fn anchor(&self, trajectory_id: TrajectoryId) -> Result<MerkleAnchor> {
        let entries = self.entries(trajectory_id)?;
        verify_integrity_entries(&entries)?;
        Ok(compute_anchor(trajectory_id, &entries))
    }
}

/// Forwarding impl so a boxed trait object is itself a `LedgerStore`. This lets
/// a caller hold *either* backend behind one concrete type — e.g. the HTTP
/// server selecting SQLite or Postgres at startup and running
/// `Runtime<Box<dyn LedgerStore>>` — without making every signature generic.
/// Every method, including the derived defaults, forwards to the inner value so
/// a backend that overrides a default is still honored.
impl LedgerStore for Box<dyn LedgerStore> {
    fn append_root(&self, trajectory_id: TrajectoryId, note: &str) -> Result<Entry> {
        (**self).append_root(trajectory_id, note)
    }
    fn append_commit(&self, commit: Commit) -> Result<Entry> {
        (**self).append_commit(commit)
    }
    fn append_rejection(
        &self,
        trajectory_id: TrajectoryId,
        intent_id: IntentId,
        reason: RejectionReason,
    ) -> Result<Entry> {
        (**self).append_rejection(trajectory_id, intent_id, reason)
    }
    fn append_pending_approval(
        &self,
        trajectory_id: TrajectoryId,
        proposal: Proposal,
        channel: String,
        reason: String,
    ) -> Result<Entry> {
        (**self).append_pending_approval(trajectory_id, proposal, channel, reason)
    }
    fn append_delegation(
        &self,
        trajectory_id: TrajectoryId,
        child_trajectory_id: TrajectoryId,
        task: &str,
        final_answer: Option<String>,
    ) -> Result<Entry> {
        (**self).append_delegation(trajectory_id, child_trajectory_id, task, final_answer)
    }
    fn append_branch_root(
        &self,
        new_trajectory_id: TrajectoryId,
        source_trajectory_id: TrajectoryId,
        source_commit_id: CommitId,
        note: &str,
    ) -> Result<Entry> {
        (**self).append_branch_root(
            new_trajectory_id,
            source_trajectory_id,
            source_commit_id,
            note,
        )
    }
    fn head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)> {
        (**self).head(trajectory_id)
    }
    fn entries(&self, trajectory_id: TrajectoryId) -> Result<Vec<Entry>> {
        (**self).entries(trajectory_id)
    }
    fn query_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>> {
        (**self).query_entries(trajectory_id, kind, from_ts, to_ts, limit)
    }
    fn count_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Result<u64> {
        (**self).count_entries(trajectory_id, kind, from_ts, to_ts)
    }
    fn has_trajectory(&self, trajectory_id: TrajectoryId) -> bool {
        (**self).has_trajectory(trajectory_id)
    }
    fn verify_integrity(&self, trajectory_id: TrajectoryId) -> Result<()> {
        (**self).verify_integrity(trajectory_id)
    }
    fn anchor(&self, trajectory_id: TrajectoryId) -> Result<MerkleAnchor> {
        (**self).anchor(trajectory_id)
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use thymos_core::{
        commit::{Commit, CommitBody, Observation},
        delta::{DeltaOp, StructuredDelta},
        ids::{ProposalId, WritId},
        proposal::{
            ExecutionPlan, PolicyDecision, PolicyTrace, Proposal, ProposalBody, ProposalStatus,
        },
        IntentId, COMPILER_VERSION,
    };

    fn trivial_commit(traj: TrajectoryId, parent: Option<CommitId>, seq: u64) -> Commit {
        let body = CommitBody {
            parent: parent.into_iter().collect(),
            trajectory_id: traj,
            proposal_id: ProposalId::ZERO,
            intent_id: IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            seq,
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: "foo".into(),
                value: serde_json::json!("bar"),
            }),
            observations: vec![Observation {
                tool: "kv_set".into(),
                output: serde_json::json!(null),
                latency_ms: 1,
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
        Commit::new(body).unwrap()
    }

    #[test]
    fn root_and_commit_append() {
        let l = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"t1");
        let root = l.append_root(traj, "hello").unwrap();
        assert_eq!(root.seq, 0);

        let c1 = trivial_commit(traj, Some(CommitId(root.id)), 1);
        let e = l.append_commit(c1).unwrap();
        assert_eq!(e.seq, 1);

        l.verify_integrity(traj).unwrap();
    }

    /// Phase III prep: the runtime will be generic over `LedgerStore`. This
    /// proves `SqliteLedger` satisfies the trait and that a fully
    /// backend-agnostic function can drive the entire append/read/verify/query
    /// surface through the trait alone — including the derived default methods
    /// (`has_trajectory`, `verify_integrity`, `anchor`).
    #[test]
    fn ledger_store_trait_drives_full_surface() {
        fn exercise<L: LedgerStore>(l: &L) {
            let traj = TrajectoryId::new_from_seed(b"trait-generic");
            let root = l.append_root(traj, "via-trait").unwrap();
            assert_eq!(root.seq, 0);
            assert!(l.has_trajectory(traj));

            let c1 = trivial_commit(traj, Some(CommitId(root.id)), 1);
            l.append_commit(c1).unwrap();

            let (_head_id, head_seq) = l.head(traj).unwrap();
            assert_eq!(head_seq, 1);
            assert_eq!(l.entries(traj).unwrap().len(), 2);

            l.verify_integrity(traj).unwrap();
            let _ = l.anchor(traj).unwrap();

            assert_eq!(l.count_entries(Some(traj), None, None, None).unwrap(), 2);
            assert_eq!(
                l.count_entries(Some(traj), Some("commit"), None, None)
                    .unwrap(),
                1
            );
            assert_eq!(
                l.query_entries(Some(traj), None, None, None, None)
                    .unwrap()
                    .len(),
                2
            );
        }

        let l = Ledger::open_in_memory().unwrap();
        exercise(&l);
    }

    /// `Box<dyn LedgerStore>` is itself a `LedgerStore`, so a single boxed value
    /// can stand in for any backend (the server's runtime-time backend choice).
    #[test]
    fn boxed_dyn_ledger_store_forwards() {
        let boxed: Box<dyn LedgerStore> = Box::new(Ledger::open_in_memory().unwrap());
        let traj = TrajectoryId::new_from_seed(b"boxed-dyn");
        let root = boxed.append_root(traj, "via-box").unwrap();
        assert_eq!(root.seq, 0);
        let c1 = trivial_commit(traj, Some(CommitId(root.id)), 1);
        boxed.append_commit(c1).unwrap();
        assert_eq!(boxed.head(traj).unwrap().1, 1);
        assert_eq!(boxed.entries(traj).unwrap().len(), 2);
        boxed.verify_integrity(traj).unwrap();
        // It can also parameterize the runtime-facing generic surface.
        fn takes_store<L: LedgerStore>(l: &L, t: TrajectoryId) -> usize {
            l.entries(t).unwrap().len()
        }
        assert_eq!(takes_store(&boxed, traj), 2);
    }

    /// Regression: two trajectories created with the *same* note against one
    /// shared ledger must not collide on `entries.id`. The root id is a content
    /// hash that now binds the trajectory id, so distinct trajectories get
    /// distinct roots even when the note is identical. (This is the routed-submit
    /// path: every routed action for the same tool roots with note
    /// `"routed:<tool>"` against the one process-wide ledger.)
    #[test]
    fn same_note_distinct_trajectories_do_not_collide() {
        let l = Ledger::open_in_memory().unwrap();
        let traj_a = TrajectoryId::new_from_seed(b"routed-a");
        let traj_b = TrajectoryId::new_from_seed(b"routed-b");
        let root_a = l.append_root(traj_a, "routed:kv_set").unwrap();
        // Before the fix this second append failed with
        // "UNIQUE constraint failed: entries.id".
        let root_b = l.append_root(traj_b, "routed:kv_set").unwrap();
        assert_ne!(root_a.id, root_b.id, "same-note roots must get distinct ids");
        l.verify_integrity(traj_a).unwrap();
        l.verify_integrity(traj_b).unwrap();
    }

    #[test]
    fn determinism_same_inputs_same_id() {
        let l1 = Ledger::open_in_memory().unwrap();
        let l2 = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"det");
        let r1 = l1.append_root(traj, "x").unwrap();
        let r2 = l2.append_root(traj, "x").unwrap();
        assert_eq!(r1.id, r2.id);
        let c1 = trivial_commit(traj, Some(CommitId(r1.id)), 1);
        let c2 = trivial_commit(traj, Some(CommitId(r2.id)), 1);
        assert_eq!(c1.id, c2.id);
    }

    #[test]
    fn rejects_out_of_order_seq() {
        let l = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"t2");
        let root = l.append_root(traj, "hello").unwrap();
        let c = trivial_commit(traj, Some(CommitId(root.id)), 5);
        assert!(l.append_commit(c).is_err());
    }

    /// W8: two writers (separate connections to the same file) racing to append
    /// at the same seq must resolve to exactly one winner — never a forked chain.
    #[test]
    fn concurrent_append_at_same_seq_admits_exactly_one() {
        use std::sync::{Arc, Barrier};
        use std::thread;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("thymos-w8-{}-{}.db", std::process::id(), nanos));
        let _ = std::fs::remove_file(&path);

        let traj = TrajectoryId::new_from_seed(b"w8-race");
        {
            let l = Ledger::open(&path).unwrap();
            l.append_root(traj, "root").unwrap();
        }
        let root_id = Ledger::open(&path).unwrap().head(traj).unwrap().0;

        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for n in 0..2u8 {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let l = Ledger::open(&path).unwrap();
                // Two *distinct* commits (different delta → different id) both
                // claiming seq 1, so the seq-unique invariant is what decides,
                // not the id primary key.
                let body = CommitBody {
                    parent: vec![CommitId(root_id)],
                    trajectory_id: traj,
                    proposal_id: ProposalId::ZERO,
                    intent_id: IntentId::ZERO,
                    writ_id: WritId(ContentHash::ZERO),
                    seq: 1,
                    delta: StructuredDelta::single(DeltaOp::Create {
                        kind: "kv".into(),
                        id: format!("k{n}"),
                        value: serde_json::json!(n),
                    }),
                    observations: vec![],
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
                let commit = Commit::new(body).unwrap();
                barrier.wait();
                l.append_commit(commit).is_ok()
            }));
        }

        let successes = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();

        let l = Ledger::open(&path).unwrap();
        let entries = l.entries(traj).unwrap();
        l.verify_integrity(traj).expect("chain must stay valid");
        let _ = std::fs::remove_file(&path);

        assert_eq!(successes, 1, "exactly one writer may win the seq-1 slot");
        assert_eq!(entries.len(), 2, "root + exactly one commit (no fork)");
    }

    // ── F2 hardening: verify_integrity_entries new invariants ────────────

    /// Build a synthetic entry whose `id` is the proper content_hash of
    /// `payload` but whose other fields can be tuned for fault injection.
    fn forge_entry(
        trajectory_id: TrajectoryId,
        parent: Option<ContentHash>,
        seq: u64,
        kind: EntryKind,
        payload: EntryPayload,
    ) -> Entry {
        let id = thymos_core::content_hash(&payload).unwrap();
        Entry {
            id,
            trajectory_id,
            parent,
            seq,
            kind,
            payload,
        }
    }

    #[test]
    fn rejects_mixed_trajectory_ids() {
        let traj_a = TrajectoryId::new_from_seed(b"traj-a");
        let traj_b = TrajectoryId::new_from_seed(b"traj-b");
        let root_a = forge_entry(
            traj_a,
            None,
            0,
            EntryKind::Root,
            EntryPayload::Root {
                trajectory_id: traj_a,
                note: "a".into(),
            },
        );
        // Same seq=1 but on the wrong trajectory.
        let bad = forge_entry(
            traj_b,
            Some(root_a.id),
            1,
            EntryKind::Root,
            EntryPayload::Root {
                trajectory_id: traj_b,
                note: "b".into(),
            },
        );
        let err = verify_integrity_entries(&[root_a, bad]).unwrap_err();
        assert!(
            err.to_string().contains("belongs to trajectory"),
            "expected trajectory mismatch, got: {err}"
        );
    }

    #[test]
    fn rejects_root_relabeled_under_wrong_trajectory() {
        // A root whose payload commits to trajectory A but whose entry column
        // was relabeled to trajectory B must not verify. The payload hash still
        // matches (payload is unchanged), so only the new payload-vs-column
        // check catches the relabel.
        let traj_a = TrajectoryId::new_from_seed(b"claimed-a");
        let traj_b = TrajectoryId::new_from_seed(b"relabeled-b");
        let relabeled = forge_entry(
            traj_b,
            None,
            0,
            EntryKind::Root,
            EntryPayload::Root {
                trajectory_id: traj_a,
                note: "x".into(),
            },
        );
        let err = verify_integrity_entries(&[relabeled]).unwrap_err();
        assert!(
            err.to_string().contains("root payload trajectory"),
            "expected payload/column trajectory mismatch, got: {err}"
        );
    }

    #[test]
    fn rejects_non_root_first_entry() {
        let traj = TrajectoryId::new_from_seed(b"traj-no-root");
        // A commit-looking entry sitting at the start of the chain.
        let commit = trivial_commit(traj, None, 0);
        let payload = EntryPayload::Commit(commit);
        let only = forge_entry(traj, None, 0, EntryKind::Commit, payload);
        let err = verify_integrity_entries(&[only]).unwrap_err();
        assert!(
            err.to_string().contains("first entry must be Root or Branch"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_root_with_nonzero_seq() {
        let traj = TrajectoryId::new_from_seed(b"traj-bad-seq");
        let bad_root = forge_entry(
            traj,
            None,
            7,
            EntryKind::Root,
            EntryPayload::Root {
                trajectory_id: traj,
                note: "x".into(),
            },
        );
        let err = verify_integrity_entries(&[bad_root]).unwrap_err();
        assert!(
            err.to_string().contains("must have seq 0"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_root_with_parent_some() {
        let traj = TrajectoryId::new_from_seed(b"traj-bad-parent");
        let bogus_parent = ContentHash([42u8; 32]);
        let bad_root = forge_entry(
            traj,
            Some(bogus_parent),
            0,
            EntryKind::Root,
            EntryPayload::Root {
                trajectory_id: traj,
                note: "x".into(),
            },
        );
        let err = verify_integrity_entries(&[bad_root]).unwrap_err();
        assert!(
            err.to_string().contains("parent=None"),
            "got: {err}"
        );
    }

    #[test]
    fn accepts_branch_as_first_entry() {
        // A trajectory rooted by a Branch (rather than a Root) entry is valid
        // — it represents a fork from another trajectory.
        let traj = TrajectoryId::new_from_seed(b"traj-branch");
        let src = TrajectoryId::new_from_seed(b"traj-src");
        let branch = forge_entry(
            traj,
            None,
            0,
            EntryKind::Branch,
            EntryPayload::Branch {
                source_trajectory_id: src,
                source_commit_id: CommitId(ContentHash([1u8; 32])),
                note: "fork".into(),
            },
        );
        verify_integrity_entries(&[branch]).expect("branch as first entry must verify");
    }

    // ── RFC proposal-contract-v1: PendingApproval compatibility ─────────────

    fn suspended_proposal(channel: &str, reason: &str) -> Proposal {
        let body = ProposalBody {
            intent_id: IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            plan: ExecutionPlan {
                tool: "kv_set".into(),
                args: serde_json::json!({"key": "k", "value": "v"}),
            },
            policy_trace: PolicyTrace {
                rules_evaluated: vec!["writ.authority".into()],
                decision: PolicyDecision::RequireApproval {
                    channel: channel.into(),
                    reason: reason.into(),
                },
            },
            status: ProposalStatus::Suspended {
                channel: channel.into(),
                reason: reason.into(),
            },
        };
        Proposal::new(body).unwrap()
    }

    /// RFC test-plan item: `PendingApproval` ledger entry round-trips with the
    /// new tagged `ProposalStatus` format. Writes via the real append path,
    /// reads via the SQLite-backed `entries()`, and verifies (a) integrity
    /// passes (b) the deserialized status is the same tagged variant with the
    /// same channel/reason.
    #[test]
    fn pending_approval_round_trips_with_new_status_format() {
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"pending-roundtrip");
        ledger.append_root(traj, "test").unwrap();

        let proposal = suspended_proposal("ops", "high cost");
        let proposal_id = proposal.id;

        ledger
            .append_pending_approval(traj, proposal, "ops".into(), "high cost".into())
            .expect("append pending approval");

        // Integrity must pass (hash chain + parent + seq + trajectory cohesion).
        ledger
            .verify_integrity(traj)
            .expect("integrity must hold for PendingApproval entries");

        let entries = ledger.entries(traj).unwrap();
        assert_eq!(entries.len(), 2, "root + pending_approval");
        let pending = entries
            .iter()
            .find(|e| matches!(e.kind, EntryKind::PendingApproval))
            .expect("pending_approval entry present");

        match &pending.payload {
            EntryPayload::PendingApproval {
                proposal,
                channel,
                reason,
            } => {
                assert_eq!(proposal.id, proposal_id, "ProposalId stable across ledger");
                assert_eq!(channel, "ops");
                assert_eq!(reason, "high cost");
                match &proposal.body.status {
                    ProposalStatus::Suspended { channel: c, reason: r } => {
                        // Per RFC invariants: status carries same channel/reason
                        // as the surrounding entry.
                        assert_eq!(c, channel);
                        assert_eq!(r, reason);
                    }
                    other => panic!("expected Suspended status, got {other:?}"),
                }
            }
            other => panic!("expected PendingApproval payload, got {other:?}"),
        }
    }

    /// RFC test-plan item: a pre-RFC `PendingApproval` payload (where
    /// `ProposalStatus` serialized as a plain string like
    /// `"suspended_for_approval"`) must fail to deserialize under the new
    /// runtime — and the error MUST clearly point at the status field, not
    /// silently misfire as a different variant.
    ///
    /// The RFC's compatibility section commits to: "Operators should treat
    /// pre-RFC `PendingApproval` entries as incompatible." This test is the
    /// runtime-side proof of that commitment.
    #[test]
    fn pre_rfc_pending_approval_fails_to_deserialize_cleanly() {
        // Synthetic pre-RFC payload. Mimics what a runtime built before the
        // proposal-contract-v1 RFC would have written to the ledger:
        //   - status is the bare string "suspended_for_approval"
        //     (old unit-variant serialization)
        //   - no routing_evidence field (didn't exist yet)
        let pre_rfc_payload = serde_json::json!({
            "type": "pending_approval",
            "proposal": {
                "id": "0000000000000000000000000000000000000000000000000000000000000000",
                "body": {
                    "intent_id": "0000000000000000000000000000000000000000000000000000000000000000",
                    "writ_id":   "0000000000000000000000000000000000000000000000000000000000000000",
                    "plan": {
                        "tool": "kv_set",
                        "args": {"key": "k"}
                    },
                    "policy_trace": {
                        "rules_evaluated": [],
                        "decision": {"kind": "permit"}
                    },
                    "status": "suspended_for_approval"
                }
            },
            "channel": "ops",
            "reason": "needs review"
        });

        let result: std::result::Result<EntryPayload, _> =
            serde_json::from_value(pre_rfc_payload);
        let err = result.expect_err(
            "pre-RFC PendingApproval must fail to deserialize under new ProposalStatus shape",
        );
        let msg = err.to_string();

        // The error must be diagnostic — it should at minimum mention the
        // status type so an operator can map it back to the breaking change.
        // serde_json's typical error for "expected internally-tagged enum but
        // got plain string" reads:
        //   "invalid type: string \"suspended_for_approval\", expected
        //    internally tagged enum ProposalStatus"
        // We assert the diagnostic is unambiguous without over-fitting to the
        // exact wording (serde may polish messages between versions).
        assert!(
            msg.contains("ProposalStatus")
                || msg.contains("suspended_for_approval")
                || msg.contains("kind"),
            "deserialization error must clearly indicate the ProposalStatus break; got: {msg}"
        );
    }

    /// Negative companion to the above: a post-RFC payload with the tagged
    /// status form deserializes cleanly when fed through the same path. This
    /// guards against the previous test passing for the wrong reason (e.g.
    /// some unrelated schema mismatch).
    #[test]
    fn post_rfc_pending_approval_deserializes_cleanly() {
        let post_rfc_payload = serde_json::json!({
            "type": "pending_approval",
            "proposal": {
                "id": "0000000000000000000000000000000000000000000000000000000000000000",
                "body": {
                    "intent_id": "0000000000000000000000000000000000000000000000000000000000000000",
                    "writ_id":   "0000000000000000000000000000000000000000000000000000000000000000",
                    "plan": {
                        "tool": "kv_set",
                        "args": {"key": "k"}
                    },
                    "policy_trace": {
                        "rules_evaluated": [],
                        "decision": {"kind": "permit"}
                    },
                    "status": {
                        "kind": "suspended",
                        "channel": "ops",
                        "reason": "needs review"
                    }
                }
            },
            "channel": "ops",
            "reason": "needs review"
        });
        let parsed: EntryPayload = serde_json::from_value(post_rfc_payload)
            .expect("post-RFC payload must deserialize");
        match parsed {
            EntryPayload::PendingApproval { proposal, .. } => {
                assert!(matches!(
                    proposal.body.status,
                    ProposalStatus::Suspended { .. }
                ));
            }
            _ => panic!("expected PendingApproval"),
        }
    }
}
