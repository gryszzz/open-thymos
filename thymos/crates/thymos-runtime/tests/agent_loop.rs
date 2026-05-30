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
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();
    Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "test-agent".into(),
            subject_pubkey: public_key_of(&agent_key),
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
