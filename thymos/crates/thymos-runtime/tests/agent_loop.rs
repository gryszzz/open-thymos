//! Integration tests for the agent loop. Uses `MockCognition` so that the
//! Triad is exercised deterministically without a network.

use thymos_cognition::mock::MockCognition;
use thymos_ledger::Ledger;
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, run_agent, AgentRunOptions, Budget, CoreIntent,
    DelegationBounds, EffectCeiling, IntentBody, IntentKind, Runtime, Termination, TimeWindow,
    ToolPattern, Writ, WritBody,
};
use thymos_tools::{KvGetTool, KvSetTool, ToolRegistry};

fn build_runtime() -> Runtime {
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(KvSetTool::default());
    tools.register(KvGetTool::default());
    let policy = PolicyEngine::new().with(WritAuthorityPolicy);
    Runtime::new(ledger, tools, policy)
}

fn root_writ() -> Writ {
    root_writ_with_nonce([0u8; 16])
}

fn root_writ_with_nonce(nonce: [u8; 16]) -> Writ {
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();
    Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "test-agent".into(),
            subject_pubkey: public_key_of(&agent_key),
            nonce,
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_*")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 32,
                wall_clock_ms: 60_000,
                usd_millicents: 0,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow {
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        },
        &root_key,
    )
    .expect("sign writ")
}

fn mk_intent(target: &str, args: serde_json::Value, nonce: u8) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "mock".into(),
        kind: IntentKind::Act,
        target: target.into(),
        args,
        rationale: "test".into(),
        nonce: [nonce; 16],
    })
    .unwrap()
}

#[test]
fn concurrent_runs_same_task_distinct_writs_no_collision() {
    // Load/race harden: fire many runs of the SAME task concurrently against a
    // shared runtime (ledger is Mutex<Connection>). Each mints its own writ, so
    // all must succeed with distinct trajectories — no (trajectory_id, seq)
    // collision and no torn appends under contention.
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::thread;

    let runtime = Arc::new(build_runtime());
    let task = "concurrent identical task";
    let n: u8 = 16;

    let handles: Vec<_> = (0..n)
        .map(|i| {
            let rt = Arc::clone(&runtime);
            thread::spawn(move || {
                let mut nonce = [0u8; 16];
                nonce[0] = i + 1; // distinct per thread
                let writ = root_writ_with_nonce(nonce);
                let set = mk_intent(
                    "kv_set",
                    serde_json::json!({"key": format!("k{i}"), "value": "v"}),
                    1,
                );
                let mut cognition = MockCognition::new(vec![vec![set]], Some("done".into()));
                run_agent(
                    &rt,
                    &mut cognition,
                    task,
                    &writ,
                    AgentRunOptions { max_steps: 4 },
                    None,
                )
                .map(|s| (s.trajectory_id, s.commits))
            })
        })
        .collect();

    let mut trajectories = HashSet::new();
    for h in handles {
        let (traj, commits) = h
            .join()
            .expect("thread panicked")
            .expect("each concurrent run must succeed");
        assert_eq!(commits, 1, "each run commits its kv_set");
        assert!(trajectories.insert(traj), "trajectories must be unique");
    }
    assert_eq!(trajectories.len(), n as usize);
}

#[test]
fn same_task_text_distinct_writs_do_not_collide() {
    // Regression: the trajectory must be seeded from the writ (unique per run
    // via its nonce), NOT the task text. Two runs of the *same task string*
    // (each with its own writ, as the server mints them) must get distinct
    // trajectories and both succeed — otherwise the second collides on the
    // append-only ledger's (trajectory_id, seq) ROOT.
    let runtime = build_runtime();
    let task = "Set greeting to hello, then read it back";

    let go = |writ: &Writ| {
        let set = mk_intent("kv_set", serde_json::json!({"key": "k", "value": "v"}), 1);
        let mut cognition = MockCognition::new(vec![vec![set]], Some("done".into()));
        run_agent(
            &runtime,
            &mut cognition,
            task,
            writ,
            AgentRunOptions { max_steps: 4 },
            None,
        )
    };

    let writ_a = root_writ_with_nonce([1u8; 16]);
    let writ_b = root_writ_with_nonce([2u8; 16]);
    assert_ne!(writ_a.id, writ_b.id, "distinct nonces → distinct writ ids");

    let a = go(&writ_a).expect("first run of the task");
    let b = go(&writ_b).expect("second run of the SAME task must not collide");

    assert_ne!(
        a.trajectory_id, b.trajectory_id,
        "same task + distinct writs must yield distinct trajectories"
    );
    assert_eq!(a.commits, 1);
    assert_eq!(b.commits, 1);
}

#[test]
fn run_agent_executes_scripted_intents_and_terminates() {
    let runtime = build_runtime();
    let writ = root_writ();

    let set = mk_intent("kv_set", serde_json::json!({"key": "k", "value": "v"}), 1);
    let get = mk_intent("kv_get", serde_json::json!({"key": "k"}), 2);

    let mut cognition = MockCognition::new(vec![vec![set], vec![get]], Some("done".into()));

    let summary = run_agent(
        &runtime,
        &mut cognition,
        "exercise the triad",
        &writ,
        AgentRunOptions { max_steps: 8 },
        None,
    )
    .expect("agent run");

    assert_eq!(summary.intents_submitted, 2);
    assert_eq!(summary.commits, 2);
    assert_eq!(summary.rejections, 0);
    assert!(matches!(summary.terminated_by, Termination::CognitionDone));
    assert_eq!(summary.final_answer.as_deref(), Some("done"));
}

#[test]
fn run_agent_records_rejection_and_keeps_going() {
    let runtime = build_runtime();
    let writ = root_writ();

    let bad = mk_intent("refund_order", serde_json::json!({"order_id": 1}), 9);
    let good = mk_intent("kv_set", serde_json::json!({"key": "k", "value": "v"}), 1);

    let mut cognition = MockCognition::new(vec![vec![bad], vec![good]], Some("recovered".into()));

    let summary = run_agent(
        &runtime,
        &mut cognition,
        "adapt to rejection",
        &writ,
        AgentRunOptions { max_steps: 8 },
        None,
    )
    .expect("agent run");

    assert_eq!(summary.rejections, 1);
    assert_eq!(summary.commits, 1);
    assert!(matches!(summary.terminated_by, Termination::CognitionDone));
}

#[test]
fn run_agent_stops_at_max_steps() {
    let runtime = build_runtime();
    let writ = root_writ();

    let batches: Vec<Vec<CoreIntent>> = (0..4)
        .map(|i| {
            vec![mk_intent(
                "kv_set",
                serde_json::json!({"key": "k", "value": format!("v{i}")}),
                i as u8 + 10,
            )]
        })
        .collect();

    let mut cognition = MockCognition::new(batches, Some("unreached".into()));

    let summary = run_agent(
        &runtime,
        &mut cognition,
        "step-bounded run",
        &writ,
        AgentRunOptions { max_steps: 2 },
        None,
    )
    .expect("agent run");

    assert_eq!(summary.steps_executed, 2);
    assert!(matches!(
        summary.terminated_by,
        Termination::MaxStepsReached
    ));
    assert!(summary.final_answer.is_none());
}

#[test]
fn run_agent_rejects_on_budget_exhaustion() {
    let runtime = build_runtime();
    // Writ with only 2 tool_calls budget.
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();
    let tight_writ = Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "test-agent".into(),
            subject_pubkey: public_key_of(&agent_key),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_*")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 2,
                wall_clock_ms: 60_000,
                usd_millicents: 0,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow {
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        },
        &root_key,
    )
    .expect("sign writ");

    // 3 tool calls — the 3rd should be rejected as BudgetExhausted.
    let batches: Vec<Vec<CoreIntent>> = (0..3)
        .map(|i| {
            vec![mk_intent(
                "kv_set",
                serde_json::json!({"key": format!("k{i}"), "value": "v"}),
                i as u8 + 20,
            )]
        })
        .collect();

    let mut cognition = MockCognition::new(batches, Some("budget test".into()));

    let summary = run_agent(
        &runtime,
        &mut cognition,
        "budget-bounded run",
        &tight_writ,
        AgentRunOptions { max_steps: 8 },
        None,
    )
    .expect("agent run");

    assert_eq!(summary.commits, 2);
    assert_eq!(summary.rejections, 1);
}

/// Step 2 proof: the runtime is generic over *any* `LedgerStore`, not only the
/// default SQLite `Ledger`. `WrapLedger` is a distinct concrete type that
/// implements the trait by delegating to an inner `Ledger`; building a
/// `Runtime<WrapLedger>` and driving a full agent loop through it proves the
/// generic refactor works end-to-end with an arbitrary backend — the exact
/// shape a Phase III Postgres facade will take.
mod backend_agnostic {
    use super::*;
    use thymos_core::{
        commit::Commit,
        ids::IntentId,
        proposal::{Proposal, RejectionReason},
        CommitId, ContentHash, Result, TrajectoryId,
    };
    use thymos_ledger::{AuditEntry, Entry, LedgerStore};

    /// A `LedgerStore` that is *not* `Ledger`, delegating to one inside.
    struct WrapLedger(Ledger);

    impl LedgerStore for WrapLedger {
        fn append_root(&self, t: TrajectoryId, note: &str) -> Result<Entry> {
            self.0.append_root(t, note)
        }
        fn append_commit(&self, c: Commit) -> Result<Entry> {
            self.0.append_commit(c)
        }
        fn append_rejection(
            &self,
            t: TrajectoryId,
            i: IntentId,
            r: RejectionReason,
        ) -> Result<Entry> {
            self.0.append_rejection(t, i, r)
        }
        fn append_pending_approval(
            &self,
            t: TrajectoryId,
            p: Proposal,
            channel: String,
            reason: String,
        ) -> Result<Entry> {
            self.0.append_pending_approval(t, p, channel, reason)
        }
        fn append_delegation(
            &self,
            t: TrajectoryId,
            child: TrajectoryId,
            task: &str,
            final_answer: Option<String>,
        ) -> Result<Entry> {
            self.0.append_delegation(t, child, task, final_answer)
        }
        fn append_branch_root(
            &self,
            new_t: TrajectoryId,
            src: TrajectoryId,
            src_commit: CommitId,
            note: &str,
        ) -> Result<Entry> {
            self.0.append_branch_root(new_t, src, src_commit, note)
        }
        fn head(&self, t: TrajectoryId) -> Result<(ContentHash, u64)> {
            self.0.head(t)
        }
        fn entries(&self, t: TrajectoryId) -> Result<Vec<Entry>> {
            self.0.entries(t)
        }
        fn query_entries(
            &self,
            t: Option<TrajectoryId>,
            kind: Option<&str>,
            from_ts: Option<u64>,
            to_ts: Option<u64>,
            limit: Option<u32>,
        ) -> Result<Vec<AuditEntry>> {
            self.0.query_entries(t, kind, from_ts, to_ts, limit)
        }
        fn count_entries(
            &self,
            t: Option<TrajectoryId>,
            kind: Option<&str>,
            from_ts: Option<u64>,
            to_ts: Option<u64>,
        ) -> Result<u64> {
            self.0.count_entries(t, kind, from_ts, to_ts)
        }
    }

    #[test]
    fn runtime_drives_a_non_default_ledger_backend() {
        let ledger = WrapLedger(Ledger::open_in_memory().unwrap());
        let mut tools = ToolRegistry::new();
        tools.register(KvSetTool::default());
        tools.register(KvGetTool::default());
        let policy = PolicyEngine::new().with(WritAuthorityPolicy);
        // The whole point: `Runtime` parameterized by a backend that is *not*
        // the default `Ledger`.
        let runtime: Runtime<WrapLedger> = Runtime::new(ledger, tools, policy);

        let writ = root_writ();
        let set = mk_intent("kv_set", serde_json::json!({"key": "k", "value": "v"}), 1);
        let get = mk_intent("kv_get", serde_json::json!({"key": "k"}), 2);
        let mut cognition = MockCognition::new(vec![vec![set], vec![get]], Some("done".into()));

        let summary = run_agent(
            &runtime,
            &mut cognition,
            "drive a non-default backend",
            &writ,
            AgentRunOptions { max_steps: 8 },
            None,
        )
        .expect("agent run on wrapped backend");

        assert_eq!(summary.intents_submitted, 2);
        assert_eq!(summary.commits, 2);
        assert!(matches!(summary.terminated_by, Termination::CognitionDone));
        assert_eq!(summary.final_answer.as_deref(), Some("done"));
    }
}

#[test]
#[ignore = "timing benchmark — run with --include-ignored --nocapture"]
fn bench_execution_overhead_per_proposal() {
    use std::time::Instant;
    let rt = build_runtime();
    let writ = root_writ();
    let n = 50u32;
    let mut total_ns = 0u128;

    for i in 0..n {
        let run = rt.create_run(&format!("bench-{i}"), format!("bench-run-{i}").as_bytes()).unwrap();
        let intent = mk_intent(
            "kv_set",
            serde_json::json!({"key": format!("k{i}"), "value": "v"}),
            i as u8,
        );
        let t = Instant::now();
        run.submit(intent, &writ).unwrap();
        total_ns += t.elapsed().as_nanos();
    }

    let avg_us = total_ns / n as u128 / 1000;
    println!(
        "\nbench_execution_overhead_per_proposal: n={n} avg={}µs ({:.2}ms)",
        avg_us,
        avg_us as f64 / 1000.0
    );
}
