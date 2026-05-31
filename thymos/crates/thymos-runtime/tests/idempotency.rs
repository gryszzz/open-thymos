//! Idempotency: an External/Irreversible tool must execute at most once per
//! (content-addressed) proposal. Re-submitting or re-approving the same proposal
//! returns the prior commit instead of repeating the side effect. Write/Read
//! effects are intentionally not guarded (they are replayable deltas).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use thymos_core::{
    commit::Observation,
    delta::StructuredDelta,
    error::Result,
};
use thymos_ledger::{EntryPayload, Ledger};
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, CoreIntent, DelegationBounds, EffectCeiling,
    IntentBody, IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

/// Counts how many times `execute` actually runs.
struct CountingTool {
    name: &'static str,
    effect: EffectClass,
    counter: Arc<AtomicUsize>,
}

impl ToolContract for CountingTool {
    fn meta(&self) -> &ToolContractMeta {
        // Leak a per-instance meta so `&ToolContractMeta` can be returned.
        Box::leak(Box::new(ToolContractMeta {
            name: self.name.into(),
            version: "1.0.0".into(),
            effect_class: self.effect,
            risk_class: RiskClass::High,
        }))
    }
    fn description(&self) -> &str {
        "counts executions"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutcome {
            // Empty delta so re-execution applies cleanly (the side effect we
            // care about here is the `counter`, standing in for a real external
            // action like a payment or deploy).
            delta: StructuredDelta(vec![]),
            observation: Observation {
                tool: self.name.into(),
                output: json!(null),
                latency_ms: 0,
            },
        })
    }
}

fn all_effects_writ(tool: &str) -> Writ {
    let issuer = generate_signing_key();
    let subject = generate_signing_key();
    Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&issuer),
            subject: "agent".into(),
            subject_pubkey: public_key_of(&subject),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact(tool)],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 100,
                wall_clock_ms: 600_000,
                usd_millicents: 1_000_000,
            },
            // Grant every effect so the W1 effect gate admits the tool; the
            // idempotency guard is what this test exercises.
            effect_ceiling: EffectCeiling {
                read: true,
                write: true,
                external: true,
                irreversible: true,
            },
            time_window: TimeWindow {
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        },
        &issuer,
    )
    .unwrap()
}

fn act(tool: &str) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: tool.into(),
        args: json!({}),
        rationale: "idempotency".into(),
        // Fixed nonce → identical intent → identical, content-addressed proposal.
        nonce: [7; 16],
    })
    .unwrap()
}

fn commit_count(runtime: &Runtime, traj: thymos_core::TrajectoryId) -> usize {
    runtime
        .ledger
        .entries(traj)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.payload, EntryPayload::Commit(_)))
        .count()
}

#[test]
fn irreversible_tool_executes_at_most_once_per_proposal() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(CountingTool {
        name: "transfer",
        effect: EffectClass::Irreversible,
        counter: Arc::clone(&counter),
    });
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let writ = all_effects_writ("transfer");
    let run = runtime.create_run("idem", b"idem").unwrap();

    // Same intent submitted twice → same proposal id.
    let first = run.submit(act("transfer"), &writ).unwrap();
    let second = run.submit(act("transfer"), &writ).unwrap();

    let id1 = match first {
        Step::Committed(id) => id,
        other => panic!("first submit not committed: {other:?}"),
    };
    let id2 = match second {
        Step::Committed(id) => id,
        other => panic!("second submit not committed: {other:?}"),
    };

    assert_eq!(id1, id2, "second submit must return the original commit");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "irreversible side effect must run exactly once"
    );
    assert_eq!(
        commit_count(&runtime, run.trajectory_id()),
        1,
        "no duplicate commit for the same proposal"
    );
}

#[test]
fn write_effects_are_not_idempotency_guarded() {
    // Scope check: replayable Write effects are intentionally NOT guarded.
    let counter = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(CountingTool {
        name: "kv_put",
        effect: EffectClass::Write,
        counter: Arc::clone(&counter),
    });
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let writ = all_effects_writ("kv_put");
    let run = runtime.create_run("idem2", b"idem2").unwrap();

    run.submit(act("kv_put"), &writ).unwrap();
    run.submit(act("kv_put"), &writ).unwrap();

    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "Write effects run each time (idempotency is scoped to External/Irreversible)"
    );
}
