//! Deterministic replay verifier.
//!
//! Walks every entry of a trajectory and proves three things:
//!   1. The hash chain is intact — every entry's `id` is `blake3(payload)` and
//!      every entry's `parent` is the previous entry's `id`.
//!   2. The commit sequence is contiguous starting from seq 0.
//!   3. Re-applying the deltas in order produces a `World` that matches the
//!      one observed at replay time.
//!
//! Optionally, `compiler_version_pinning` rejects any commit whose
//! `compiler_version` field disagrees with the version the verifier was built
//! against — useful for catching a downgrade or a drift in the compiler crate
//! after the fact.

use serde::{Deserialize, Serialize};

use thymos_core::{
    commit::Commit, content_hash, error::Result, world::World, CommitId, COMPILER_VERSION,
};

use crate::{Entry, EntryPayload};

/// Result of a successful replay.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayReport {
    pub trajectory_id: String,
    pub entries_seen: usize,
    pub commits_replayed: usize,
    pub head_commit: Option<String>,
    pub head_seq: u64,
    pub compiler_versions_seen: Vec<String>,
}

/// Configuration knobs for the verifier.
#[derive(Clone, Debug, Default)]
pub struct ReplayConfig {
    /// If `Some`, every commit must declare exactly this compiler version.
    /// Use [`ReplayConfig::pinned_to_current`] to pin against the version
    /// linked into the verifier binary.
    pub require_compiler_version: Option<String>,
    /// If `Some`, every commit must carry an ed25519 signature that verifies
    /// against this public key — i.e. the trajectory must have been produced
    /// by a runtime holding the corresponding signing key
    /// (`Runtime::with_commit_signer`). Unsigned commits are rejected.
    pub require_commit_signatures: Option<thymos_core::crypto::PublicKey>,
    /// If `Some`, every commit must declare exactly this policy-set hash
    /// (`PolicyEngine::policy_set_hash`). Detects that the policy rule set
    /// drifted since the trajectory was produced.
    pub require_policy_set_hash: Option<String>,
}

impl ReplayConfig {
    pub fn pinned_to_current() -> Self {
        ReplayConfig {
            require_compiler_version: Some(COMPILER_VERSION.into()),
            require_commit_signatures: None,
            require_policy_set_hash: None,
        }
    }

    /// Require every commit to be signed by `pubkey`.
    pub fn require_signed_by(mut self, pubkey: thymos_core::crypto::PublicKey) -> Self {
        self.require_commit_signatures = Some(pubkey);
        self
    }

    /// Require every commit to declare exactly `hash` as its policy-set hash.
    pub fn require_policy_set(mut self, hash: impl Into<String>) -> Self {
        self.require_policy_set_hash = Some(hash.into());
        self
    }
}

/// Replay the trajectory described by `entries` and return the rebuilt world.
///
/// `entries` must be ordered by `seq` ascending — both backends already return
/// them that way.
pub fn replay(entries: &[Entry], cfg: &ReplayConfig) -> Result<(World, ReplayReport)> {
    crate::verify_integrity_entries(entries)?;

    let mut world = World::default();
    let mut commits_replayed = 0usize;
    let mut head_commit: Option<CommitId> = None;
    let mut head_seq: u64 = 0;
    let mut compiler_versions_seen: Vec<String> = Vec::new();
    let mut trajectory_id_hex = String::new();

    for entry in entries {
        head_seq = entry.seq;
        if trajectory_id_hex.is_empty() {
            trajectory_id_hex = entry.trajectory_id.to_string();
        }

        if let EntryPayload::Commit(commit) = &entry.payload {
            apply_commit(&mut world, commit, cfg)?;
            commits_replayed += 1;
            head_commit = Some(commit.id);
            if !compiler_versions_seen.contains(&commit.body.compiler_version) {
                compiler_versions_seen.push(commit.body.compiler_version.clone());
            }
        }
    }

    Ok((
        world,
        ReplayReport {
            trajectory_id: trajectory_id_hex,
            entries_seen: entries.len(),
            commits_replayed,
            head_commit: head_commit.map(|c| c.to_string()),
            head_seq,
            compiler_versions_seen,
        },
    ))
}

fn apply_commit(world: &mut World, commit: &Commit, cfg: &ReplayConfig) -> Result<()> {
    // Spec Section 8 requires "report compiler versions seen". An empty
    // `compiler_version` is unreportable — reject regardless of pinning.
    if commit.body.compiler_version.is_empty() {
        return Err(thymos_core::error::Error::Invariant(format!(
            "commit {} has empty compiler_version (spec Section 8 requires a recorded version)",
            commit.id
        )));
    }
    if let Some(required) = &cfg.require_compiler_version {
        if commit.body.compiler_version != *required {
            return Err(thymos_core::error::Error::Invariant(format!(
                "compiler version drift at commit {}: pinned {} got {}",
                commit.id, required, commit.body.compiler_version
            )));
        }
    }
    if let Some(pk) = &cfg.require_commit_signatures {
        commit.verify_signature(pk).map_err(|e| {
            thymos_core::error::Error::Invariant(format!(
                "commit {} failed signature verification: {e}",
                commit.id
            ))
        })?;
    }
    if let Some(required) = &cfg.require_policy_set_hash {
        if commit.body.policy_set_hash != *required {
            return Err(thymos_core::error::Error::Invariant(format!(
                "policy-set drift at commit {}: pinned {} got {}",
                commit.id, required, commit.body.policy_set_hash
            )));
        }
    }
    world.apply(&commit.body.delta, commit.id)
}

/// Convenience: replay and assert the rebuilt world matches `observed`.
/// Returns the report on success; an [`Error::Invariant`] on mismatch.
pub fn replay_and_match(
    entries: &[Entry],
    observed: &World,
    cfg: &ReplayConfig,
) -> Result<ReplayReport> {
    let (rebuilt, report) = replay(entries, cfg)?;
    let rebuilt_hash = content_hash(&rebuilt)?;
    let observed_hash = content_hash(observed)?;
    if rebuilt_hash != observed_hash {
        return Err(thymos_core::error::Error::Invariant(format!(
            "world divergence after replay: rebuilt {} resources ({}) vs observed {} resources ({})",
            rebuilt.resources.len(),
            rebuilt_hash,
            observed.resources.len(),
            observed_hash
        )));
    }
    Ok(report)
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use thymos_core::{
        commit::{Commit, CommitBody, Observation},
        delta::{DeltaOp, StructuredDelta},
        ids::{ProposalId, WritId},
        world::ResourceKey,
        ContentHash, TrajectoryId,
    };

    use crate::Ledger;

    fn append_kv(ledger: &Ledger, traj: TrajectoryId, key: &str, value: &str, seq: u64) -> Commit {
        let body = CommitBody {
            parent: vec![],
            trajectory_id: traj,
            proposal_id: ProposalId::ZERO,
            intent_id: thymos_core::ids::IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            seq,
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: key.into(),
                value: serde_json::json!(value),
            }),
            observations: vec![Observation {
                tool: "kv_set".into(),
                output: serde_json::json!(null),
                latency_ms: 0,
            }],
            policy_trace: thymos_core::proposal::PolicyTrace {
                rules_evaluated: vec![],
                decision: thymos_core::proposal::PolicyDecision::Permit,
            },
            compiler_version: COMPILER_VERSION.into(),
            policy_set_hash: String::new(),
            budget_cost: thymos_core::writ::BudgetCost::default(),
            compensates: None,
            routing_evidence: None,
            signature: None,
        };
        let commit = Commit::new(body).unwrap();
        ledger.append_commit(commit.clone()).unwrap();
        commit
    }

    #[test]
    fn replay_rebuilds_world() {
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"replay-1");
        ledger.append_root(traj, "test").unwrap();
        append_kv(&ledger, traj, "alpha", "1", 1);
        append_kv(&ledger, traj, "beta", "2", 2);

        let entries = ledger.entries(traj).unwrap();
        let (world, report) = replay(&entries, &ReplayConfig::default()).unwrap();
        assert_eq!(report.commits_replayed, 2);
        assert_eq!(report.entries_seen, 3);
        assert_eq!(
            world.get(&ResourceKey::new("kv", "alpha")).unwrap().value,
            serde_json::json!("1")
        );
    }

    #[test]
    fn replay_rejects_compiler_version_drift() {
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"replay-pin");
        ledger.append_root(traj, "test").unwrap();
        append_kv(&ledger, traj, "x", "y", 1);

        let entries = ledger.entries(traj).unwrap();
        let cfg = ReplayConfig {
            require_compiler_version: Some("thymos-compiler/9.9.9".into()),
            ..Default::default()
        };
        let err = replay(&entries, &cfg).unwrap_err();
        assert!(err.to_string().contains("compiler version drift"));
    }

    #[test]
    fn replay_pinned_to_current_passes() {
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"replay-pin-ok");
        ledger.append_root(traj, "test").unwrap();
        append_kv(&ledger, traj, "x", "y", 1);

        let entries = ledger.entries(traj).unwrap();
        let report = replay(&entries, &ReplayConfig::pinned_to_current())
            .unwrap()
            .1;
        assert_eq!(report.commits_replayed, 1);
        assert_eq!(
            report.compiler_versions_seen,
            vec![COMPILER_VERSION.to_string()]
        );
    }

    fn append_signed_kv(
        ledger: &Ledger,
        traj: TrajectoryId,
        key: &str,
        seq: u64,
        sk: &thymos_core::crypto::SigningKey,
    ) {
        let body = CommitBody {
            parent: vec![],
            trajectory_id: traj,
            proposal_id: ProposalId::ZERO,
            intent_id: thymos_core::ids::IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            seq,
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: key.into(),
                value: serde_json::json!("v"),
            }),
            observations: vec![],
            policy_trace: thymos_core::proposal::PolicyTrace {
                rules_evaluated: vec![],
                decision: thymos_core::proposal::PolicyDecision::Permit,
            },
            compiler_version: COMPILER_VERSION.into(),
            policy_set_hash: String::new(),
            budget_cost: thymos_core::writ::BudgetCost::default(),
            compensates: None,
            routing_evidence: None,
            signature: None,
        };
        let commit = Commit::new_signed(body, sk).unwrap();
        ledger.append_commit(commit).unwrap();
    }

    #[test]
    fn replay_requires_commit_signatures() {
        use thymos_core::crypto::{generate_signing_key, public_key_of};
        let sk = generate_signing_key();
        let pk = public_key_of(&sk);

        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"replay-sig");
        ledger.append_root(traj, "test").unwrap();
        append_signed_kv(&ledger, traj, "k", 1, &sk);

        let entries = ledger.entries(traj).unwrap();
        // Correct key verifies.
        replay(&entries, &ReplayConfig::default().require_signed_by(pk))
            .expect("signed replay must pass with the right key");
        // Wrong key is rejected.
        let wrong = public_key_of(&generate_signing_key());
        let err = replay(&entries, &ReplayConfig::default().require_signed_by(wrong))
            .unwrap_err();
        assert!(err.to_string().contains("signature verification"));
    }

    #[test]
    fn replay_detects_policy_set_drift() {
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"replay-policy");
        ledger.append_root(traj, "test").unwrap();
        append_kv(&ledger, traj, "x", "y", 1); // fixture commit: policy_set_hash == ""

        let entries = ledger.entries(traj).unwrap();
        // Matching hash passes.
        replay(&entries, &ReplayConfig::default().require_policy_set(""))
            .expect("matching policy-set hash must pass");
        // A different expected hash is flagged as drift.
        let err = replay(
            &entries,
            &ReplayConfig::default().require_policy_set("deadbeef"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("policy-set drift"));
    }

    #[test]
    fn replay_rejects_unsigned_when_signatures_required() {
        use thymos_core::crypto::{generate_signing_key, public_key_of};
        let pk = public_key_of(&generate_signing_key());
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"replay-unsigned");
        ledger.append_root(traj, "test").unwrap();
        append_kv(&ledger, traj, "x", "y", 1); // unsigned

        let entries = ledger.entries(traj).unwrap();
        let err = replay(&entries, &ReplayConfig::default().require_signed_by(pk))
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("signature verification") || msg.contains("unsigned"),
            "expected signature failure, got: {msg}"
        );
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod bench_tests {
    use super::*;
    use thymos_core::{
        commit::{Commit, CommitBody, Observation},
        delta::{DeltaOp, StructuredDelta},
        ids::{ProposalId, WritId},
        ContentHash, TrajectoryId,
    };
    use crate::Ledger;
    use std::time::Instant;

    fn build_commit(traj: TrajectoryId, key: &str, val: &str, seq: u64) -> Commit {
        let body = CommitBody {
            parent: vec![],
            trajectory_id: traj,
            proposal_id: ProposalId::ZERO,
            intent_id: thymos_core::ids::IntentId::ZERO,
            writ_id: WritId(ContentHash::ZERO),
            seq,
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: key.into(),
                value: serde_json::json!(val),
            }),
            observations: vec![Observation {
                tool: "kv_set".into(),
                output: serde_json::json!(null),
                latency_ms: 0,
            }],
            policy_trace: thymos_core::proposal::PolicyTrace {
                rules_evaluated: vec![],
                decision: thymos_core::proposal::PolicyDecision::Permit,
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
    #[ignore = "timing benchmark — run with --include-ignored --nocapture"]
    fn bench_replay_speed() {
        let n_commits: u64 = 1000;
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"bench-replay");
        ledger.append_root(traj, "bench").unwrap();
        for i in 1..=n_commits {
            let c = build_commit(traj, &format!("key{i}"), "val", i);
            ledger.append_commit(c).unwrap();
        }
        let entries = ledger.entries(traj).unwrap();
        let iters = 5u32;
        let mut total_ns = 0u128;
        for _ in 0..iters {
            let t = Instant::now();
            let _ = replay(&entries, &ReplayConfig::default()).unwrap();
            total_ns += t.elapsed().as_nanos();
        }
        let avg_us = total_ns / iters as u128 / 1000;
        let entries_per_sec = (entries.len() as u128 * 1_000_000) / avg_us.max(1);
        println!(
            "\nbench_replay_speed: n_entries={} avg_latency={}µs entries/sec={}",
            entries.len(), avg_us, entries_per_sec
        );
    }

    #[test]
    #[ignore = "timing benchmark — run with --include-ignored --nocapture"]
    fn bench_folding_speed() {
        let n_commits: u64 = 1000;
        let ledger = Ledger::open_in_memory().unwrap();
        let traj = TrajectoryId::new_from_seed(b"bench-fold");
        ledger.append_root(traj, "bench").unwrap();
        for i in 1..=n_commits {
            let c = build_commit(traj, &format!("k{i}"), "v", i);
            ledger.append_commit(c).unwrap();
        }
        let entries = ledger.entries(traj).unwrap();
        let iters = 5u32;
        let mut total_ns = 0u128;
        for _ in 0..iters {
            let mut world = thymos_core::world::World::default();
            let t = Instant::now();
            for e in &entries {
                if let crate::EntryPayload::Commit(c) = &e.payload {
                    world.apply(&c.body.delta, c.id).unwrap();
                }
            }
            total_ns += t.elapsed().as_nanos();
        }
        let avg_us = total_ns / iters as u128 / 1000;
        let commits_per_sec = (n_commits as u128 * 1_000_000) / avg_us.max(1);
        println!(
            "\nbench_folding_speed: n_commits={} avg_latency={}µs commits/sec={}",
            n_commits, avg_us, commits_per_sec
        );
    }
}
