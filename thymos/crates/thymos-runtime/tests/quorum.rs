//! Quorum / multi-party approval: a suspended proposal requires N distinct
//! approvers before it executes. One approval is not enough when quorum > 1;
//! a single explicit denial still vetoes immediately.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use thymos_cognition::mock::MockCognition;
use thymos_core::{
    commit::Observation, delta::StructuredDelta, error::Result, intent::Intent,
    proposal::PolicyDecision, world::World,
};
use thymos_ledger::Ledger;
use thymos_policy::{Policy, PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, run_agent, AgentRunOptions, Budget, CoreIntent,
    DelegationBounds, EffectCeiling, IntentBody, IntentKind, Runtime, Step, Termination, TimeWindow,
    ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

struct NoopTool {
    counter: Arc<AtomicUsize>,
}
impl ToolContract for NoopTool {
    fn meta(&self) -> &ToolContractMeta {
        static M: std::sync::OnceLock<ToolContractMeta> = std::sync::OnceLock::new();
        M.get_or_init(|| ToolContractMeta {
            name: "noop".into(),
            version: "1.0.0".into(),
            effect_class: EffectClass::Write,
            risk_class: RiskClass::Low,
        })
    }
    fn description(&self) -> &str {
        "noop"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutcome {
            delta: StructuredDelta(vec![]),
            observation: Observation {
                tool: "noop".into(),
                output: json!(null),
                latency_ms: 0,
            },
        })
    }
}

/// Always suspends for approval, so we can drive the quorum path.
struct AlwaysApproval;
impl Policy for AlwaysApproval {
    fn name(&self) -> &'static str {
        "test.always_approval"
    }
    fn evaluate(&self, _i: &Intent, _w: &Writ, _world: &World) -> PolicyDecision {
        PolicyDecision::RequireApproval {
            channel: "ops".into(),
            reason: "needs N approvals".into(),
        }
    }
}

fn writ() -> Writ {
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
            tool_scopes: vec![ToolPattern::exact("noop")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 100,
                wall_clock_ms: 600_000,
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
        &issuer,
    )
    .unwrap()
}

fn act() -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: "noop".into(),
        args: json!({}),
        rationale: "quorum".into(),
        nonce: [9; 16],
    })
    .unwrap()
}

#[test]
fn requires_two_distinct_approvers() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool {
        counter: Arc::clone(&counter),
    });
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy).with(AlwaysApproval),
    )
    .with_approval_quorum(2);
    let w = writ();

    // Drive one step to produce a suspended proposal.
    let mut cog = MockCognition::new(vec![vec![act()]], Some("done".into()));
    let summary = run_agent(
        &runtime,
        &mut cog,
        "q",
        &w,
        AgentRunOptions { max_steps: 2 },
        None,
    )
    .unwrap();
    assert!(matches!(summary.terminated_by, Termination::Suspended));

    // Find the suspended proposal id from the ledger.
    let entries = runtime.ledger.entries(summary.trajectory_id).unwrap();
    let proposal_id = entries
        .iter()
        .find_map(|e| match &e.payload {
            thymos_ledger::EntryPayload::PendingApproval { proposal, .. } => Some(proposal.id),
            _ => None,
        })
        .expect("a pending approval entry");

    let run = runtime.resume_run(summary.trajectory_id).unwrap();

    // One approver → quorum not met.
    let p1 = run.approve(proposal_id, "alice");
    assert_eq!((p1.received, p1.required, p1.satisfied), (1, 2, false));
    assert!(
        run.resume_with_quorum(proposal_id, &w).is_err(),
        "must refuse to execute below quorum"
    );
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    // Same approver again → still 1 distinct.
    let p_dup = run.approve(proposal_id, "alice");
    assert_eq!(p_dup.received, 1, "duplicate approver does not count twice");

    // Second distinct approver → quorum met → executes once.
    let p2 = run.approve(proposal_id, "bob");
    assert!(p2.satisfied);
    assert!(matches!(
        run.resume_with_quorum(proposal_id, &w).unwrap(),
        Step::Committed(_)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn single_denial_vetoes_regardless_of_quorum() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool {
        counter: Arc::clone(&counter),
    });
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy).with(AlwaysApproval),
    )
    .with_approval_quorum(3);
    let w = writ();

    let mut cog = MockCognition::new(vec![vec![act()]], Some("done".into()));
    let summary = run_agent(&runtime, &mut cog, "v", &w, AgentRunOptions { max_steps: 2 }, None)
        .unwrap();
    let entries = runtime.ledger.entries(summary.trajectory_id).unwrap();
    let proposal_id = entries
        .iter()
        .find_map(|e| match &e.payload {
            thymos_ledger::EntryPayload::PendingApproval { proposal, .. } => Some(proposal.id),
            _ => None,
        })
        .unwrap();
    let run = runtime.resume_run(summary.trajectory_id).unwrap();

    // A single operator veto rejects immediately, even with quorum=3.
    assert!(matches!(
        run.resume_with_approval(proposal_id, false, &w).unwrap(),
        Step::Rejected(_)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}
