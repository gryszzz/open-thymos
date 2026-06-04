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
use thymos_ledger::postgres::{BlockingPostgresLedger, PostgresLedger};
use thymos_ledger::LedgerStore;

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

fn skip_if_unset(test: &str) -> Option<String> {
    match std::env::var("THYMOS_TEST_POSTGRES_URL") {
        Ok(u) if !u.trim().is_empty() => Some(u),
        _ => {
            eprintln!("SKIP {test}: THYMOS_TEST_POSTGRES_URL not set.");
            None
        }
    }
}

fn unique_traj(tag: &str) -> TrajectoryId {
    let seed = format!(
        "{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    TrajectoryId::new_from_seed(seed.as_bytes())
}

/// Drive an identical scripted trajectory through any `LedgerStore` and return
/// the content-addressed entry id chain.
fn drive_script<L: LedgerStore>(l: &L, traj: TrajectoryId) -> Vec<ContentHash> {
    let root = l.append_root(traj, "backend-indep").expect("append root");
    let e1 = l
        .append_commit(trivial_commit(traj, CommitId(root.id), 1, "alpha", "1"))
        .expect("commit 1");
    let _e2 = l
        .append_commit(trivial_commit(traj, CommitId(e1.id), 2, "beta", "2"))
        .expect("commit 2");
    l.verify_integrity(traj).expect("verify integrity");
    l.entries(traj)
        .expect("entries")
        .into_iter()
        .map(|e| e.id)
        .collect()
}

/// RFC `runtime-ledger-trait-v1` step 5: the Postgres blocking facade and the
/// SQLite backend, driven through the *same* synchronous `LedgerStore` surface
/// with identical inputs, must produce a byte-identical (content-addressed)
/// chain and the same head. This is the guard against silent semantic drift
/// between backends.
#[test]
#[cfg(feature = "sqlite")]
#[ignore = "live Postgres integration — set THYMOS_TEST_POSTGRES_URL and run with --ignored"]
fn blocking_facade_matches_sqlite() {
    let Some(url) = skip_if_unset("blocking_facade_matches_sqlite") else {
        return;
    };

    let pg = BlockingPostgresLedger::connect(&url).expect("connect blocking facade");
    let sqlite = thymos_ledger::Ledger::open_in_memory().expect("open sqlite");

    // Distinct trajectory ids so the run is repeatable against a shared DB, but
    // the *same* logical script, so the content-addressed payloads (and thus the
    // ids) line up position-for-position across backends.
    let traj_pg = unique_traj("indep-pg");
    let traj_sq = unique_traj("indep-sq");

    let pg_ids = drive_script(&pg, traj_pg);
    let sq_ids = drive_script(&sqlite, traj_sq);

    // The root payload binds the trajectory id, so the genesis ids differ by
    // construction; every *commit* id is trajectory-independent and must match.
    assert_eq!(pg_ids.len(), sq_ids.len(), "same number of entries");
    assert_eq!(
        &pg_ids[1..],
        &sq_ids[1..],
        "commit chain must be byte-identical across backends"
    );

    // Heads agree on seq, and each head id equals the backend's own last entry.
    assert_eq!(pg.head(traj_pg).unwrap().1, 2);
    assert_eq!(sqlite.head(traj_sq).unwrap().1, 2);
    assert_eq!(pg.head(traj_pg).unwrap().0, *pg_ids.last().unwrap());

    eprintln!("PROOF: Postgres blocking facade yields a chain identical to SQLite");
}

/// The critical safety property of the blocking facade: its *synchronous*
/// `LedgerStore` methods may be called from inside a tokio runtime (the server
/// invokes them from async handlers and `run_agent_streaming`) without panicking
/// — `block_on` would panic there, so the facade must not use it. Here we call
/// the sync methods directly on a tokio worker thread.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live Postgres integration — set THYMOS_TEST_POSTGRES_URL and run with --ignored"]
async fn blocking_facade_is_safe_from_async_context() {
    let Some(url) = skip_if_unset("blocking_facade_is_safe_from_async_context") else {
        return;
    };

    // Connect off the async worker to avoid blocking it during setup.
    let pg = tokio::task::spawn_blocking(move || BlockingPostgresLedger::connect(&url))
        .await
        .expect("join")
        .expect("connect blocking facade");

    // Call the SYNC trait methods directly on this tokio worker thread. The
    // facade routes them to its own runtime thread, so this blocks the worker
    // briefly but must NOT panic with "cannot block from within a runtime".
    let traj = unique_traj("from-async");
    let root = pg.append_root(traj, "from async").expect("append_root on worker");
    assert_eq!(root.seq, 0);
    let _c = pg
        .append_commit(trivial_commit(traj, CommitId(root.id), 1, "k", "v"))
        .expect("append_commit on worker");
    assert_eq!(pg.entries(traj).expect("entries on worker").len(), 2);
    pg.verify_integrity(traj).expect("verify on worker");

    eprintln!("PROOF: blocking facade sync methods are safe to call from a tokio worker thread");
}
