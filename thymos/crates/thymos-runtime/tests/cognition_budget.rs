//! W5: cumulative cognition (model) token/USD usage must be debited against the
//! writ budget. A mock that reports usage per step is driven until the budget is
//! exhausted, and the run must terminate with `Termination::BudgetExhausted`.

use serde_json::{json, Value};

use thymos_cognition::mock::MockCognition;
use thymos_cognition::CognitionUsage;
use thymos_core::{
    commit::Observation,
    delta::{DeltaOp, StructuredDelta},
    error::Result,
};
use thymos_ledger::Ledger;
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, run_agent, AgentRunOptions, Budget, CoreIntent,
    DelegationBounds, EffectCeiling, IntentBody, IntentKind, Runtime, Termination, TimeWindow,
    ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

struct NoopTool;
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
        "writes a kv each call"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        Ok(ToolOutcome {
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: "n".into(),
                value: json!(1),
            }),
            observation: Observation {
                tool: "noop".into(),
                output: json!(null),
                latency_ms: 0,
            },
        })
    }
}

fn writ_with_token_budget(tokens: u64) -> Writ {
    let issuer = generate_signing_key();
    let subject = generate_signing_key();
    Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&issuer),
            subject: "agent".into(),
            subject_pubkey: public_key_of(&subject),
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("noop")],
            budget: Budget {
                tokens,
                tool_calls: 100,
                wall_clock_ms: 600_000,
                usd_millicents: 1_000_000,
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

fn act(nonce: u8) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "mock".into(),
        kind: IntentKind::Act,
        target: "noop".into(),
        args: json!({}),
        rationale: "budget test".into(),
        nonce: [nonce; 16],
    })
    .unwrap()
}

#[test]
fn run_halts_when_cognition_token_budget_is_exhausted() {
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool);
    let runtime = Runtime::new(ledger, tools, PolicyEngine::new().with(WritAuthorityPolicy));

    // 60 tokens per step, budget of 100: step 1 (60) proceeds, step 2 (120) trips.
    let writ = writ_with_token_budget(100);
    let mut cog = MockCognition::new(
        vec![vec![act(1)], vec![act(2)], vec![act(3)]],
        Some("done".into()),
    )
    .with_usage_per_step(CognitionUsage {
        input_tokens: 50,
        output_tokens: 10,
        usd_millicents: 0,
    });

    let summary = run_agent(
        &runtime,
        &mut cog,
        "budget",
        &writ,
        AgentRunOptions { max_steps: 8 },
        None,
    )
    .unwrap();

    assert!(
        matches!(summary.terminated_by, Termination::BudgetExhausted),
        "expected BudgetExhausted, got {:?}",
        summary.terminated_by
    );
    assert_eq!(summary.commits, 1, "only the first step's intent should commit");
}

#[test]
fn run_completes_when_usage_stays_within_budget() {
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool);
    let runtime = Runtime::new(ledger, tools, PolicyEngine::new().with(WritAuthorityPolicy));

    let writ = writ_with_token_budget(10_000);
    let mut cog = MockCognition::new(vec![vec![act(1)]], Some("done".into()))
        .with_usage_per_step(CognitionUsage {
            input_tokens: 50,
            output_tokens: 10,
            usd_millicents: 0,
        });

    let summary = run_agent(
        &runtime,
        &mut cog,
        "ok",
        &writ,
        AgentRunOptions { max_steps: 8 },
        None,
    )
    .unwrap();

    assert!(matches!(summary.terminated_by, Termination::CognitionDone));
    assert_eq!(summary.commits, 1);
}
