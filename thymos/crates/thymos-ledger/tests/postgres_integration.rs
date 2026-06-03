//! Postgres ledger backend — gated integration test.
//!
//! The SQLite backend is exercised by the in-tree unit tests on every CI run.
//! The Postgres backend (`feature = "postgres"`) talks to a real database, so it
//! cannot run in the default CI job. This test closes the "we ship a Postgres
//! backend but never prove it works" gap by driving the real append → read-back
//! → hash-chain verification path against a live Postgres.
//!
//! Gated three ways so it never breaks CI:
//!   * `#[cfg(feature = "postgres")]` — only compiles when the backend is built.
//!   * `#[ignore]` — excluded from the default `cargo test`.
//!   * a `THYMOS_TEST_POSTGRES_URL` check — skips cleanly (prints SKIP, passes)
//!     when no database is configured.
//!
//! Run it for real with:
//!
//!     THYMOS_TEST_POSTGRES_URL=postgres://user:pass@localhost/thymos_test \
//!       cargo test -p thymos-ledger --features postgres --test postgres_integration \
//!       -- --ignored --nocapture

#![cfg(feature = "postgres")]

use thymos_core::{
    commit::{Commit, CommitBody, Observation},
    delta::{DeltaOp, StructuredDelta},
    ids::{ProposalId, WritId},
    proposal::{PolicyDecision, PolicyTrace},
    CommitId, ContentHash, IntentId, TrajectoryId, COMPILER_VERSION,
};
use thymos_ledger::postgres::PostgresLedger;

fn trivial_commit(traj: TrajectoryId, parent: CommitId, seq: u64, key: &str, val: &str) -> Commit {
    let body = CommitBody {
        parent: vec![parent],
        trajectory_id: traj,
        proposal_id: ProposalId::ZERO,
        intent_id: IntentId::ZERO,
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

#[tokio::test]
#[ignore = "live Postgres integration — set THYMOS_TEST_POSTGRES_URL and run with --ignored"]
async fn postgres_appends_and_verifies_hash_chain() {
    let url = match std::env::var("THYMOS_TEST_POSTGRES_URL") {
        Ok(u) if !u.trim().is_empty() => u,
        _ => {
            eprintln!(
                "SKIP postgres_appends_and_verifies_hash_chain: THYMOS_TEST_POSTGRES_URL not set."
            );
            return;
        }
    };

    let ledger = PostgresLedger::connect(&url)
        .await
        .expect("connect to test Postgres");

    // Unique per run so repeated runs against the same database don't collide.
    let seed = format!(
        "pg-it-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let traj = TrajectoryId::new_from_seed(seed.as_bytes());

    let root = ledger
        .append_root(traj, "pg integration")
        .await
        .expect("append root");
    assert_eq!(root.seq, 0);

    let c1 = trivial_commit(traj, CommitId(root.id), 1, "alpha", "1");
    let e1 = ledger.append_commit(c1).await.expect("append commit 1");
    assert_eq!(e1.seq, 1);

    let c2 = trivial_commit(traj, CommitId(e1.id), 2, "beta", "2");
    let e2 = ledger.append_commit(c2).await.expect("append commit 2");
    assert_eq!(e2.seq, 2);

    // Read the chain back and prove the hash chain is intact on the real DB.
    let entries = ledger.entries(traj).await.expect("read entries back");
    assert_eq!(entries.len(), 3, "root + two commits");

    ledger
        .verify_integrity(traj)
        .await
        .expect("Postgres hash chain must verify");

    let (_, head_seq) = ledger.head(traj).await.expect("head");
    assert_eq!(head_seq, 2);

    eprintln!("PROOF: Postgres backend append → read-back → verify_integrity holds (traj={traj})");
}
