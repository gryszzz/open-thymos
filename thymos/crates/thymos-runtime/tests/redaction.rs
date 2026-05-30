//! W9: secrets in a tool observation must be redacted before they land in the
//! append-only ledger. Drives a tool whose observation carries a credential and
//! asserts the committed observation has it replaced with the redaction marker.

use serde_json::{json, Value};

use thymos_cognition::mock::MockCognition;
use thymos_core::{
    commit::Observation,
    crypto::SigningKey,
    delta::{DeltaOp, StructuredDelta},
    error::Result,
    redact::REDACTED,
    Redactor,
};
use thymos_ledger::{EntryPayload, Ledger};
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, run_agent, AgentRunOptions, Budget, CoreIntent,
    DelegationBounds, EffectCeiling, IntentBody, IntentKind, Runtime, ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

/// A tool that returns a secret-bearing observation, to prove the runtime
/// redacts it before persisting.
struct LeakyTool;

impl ToolContract for LeakyTool {
    fn meta(&self) -> &ToolContractMeta {
        // `Box::leak` keeps a 'static meta without per-call allocation churn.
        static META: std::sync::OnceLock<ToolContractMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ToolContractMeta {
            name: "leaky".into(),
            version: "1.0.0".into(),
            effect_class: EffectClass::Write,
            risk_class: RiskClass::Low,
        })
    }
    fn description(&self) -> &str {
        "returns a secret to test redaction"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        Ok(ToolOutcome {
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: "result".into(),
                value: json!("done"),
            }),
            observation: Observation {
                tool: "leaky".into(),
                output: json!({
                    "api_key": "sk-super-secret",
                    "nested": {"authorization": "Bearer abc"},
                    "ok": true
                }),
                latency_ms: 1,
            },
        })
    }
}

fn signed_writ() -> (Writ, SigningKey) {
    let issuer = generate_signing_key();
    let subject = generate_signing_key();
    let writ = Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&issuer),
            subject: "agent".into(),
            subject_pubkey: public_key_of(&subject),
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("leaky")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 32,
                wall_clock_ms: 60_000,
                usd_millicents: 0,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: thymos_runtime::TimeWindow {
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
    .unwrap();
    (writ, issuer)
}

fn intent() -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "mock".into(),
        kind: IntentKind::Act,
        target: "leaky".into(),
        args: json!({}),
        rationale: "redaction test".into(),
        nonce: [1; 16],
    })
    .unwrap()
}

fn committed_output(runtime: &Runtime, traj: thymos_core::TrajectoryId) -> Value {
    let entries = runtime.ledger.entries(traj).unwrap();
    for e in entries {
        if let EntryPayload::Commit(c) = e.payload {
            return c.body.observations[0].output.clone();
        }
    }
    panic!("no commit found");
}

#[test]
fn secrets_are_redacted_in_the_ledger_by_default() {
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(LeakyTool);
    let runtime = Runtime::new(ledger, tools, PolicyEngine::new().with(WritAuthorityPolicy));

    let (writ, _) = signed_writ();
    let mut cog = MockCognition::new(vec![vec![intent()]], Some("done".into()));
    let summary = run_agent(
        &runtime,
        &mut cog,
        "redact",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();
    assert_eq!(summary.commits, 1);

    let out = committed_output(&runtime, summary.trajectory_id);
    assert_eq!(out["api_key"], REDACTED, "top-level secret must be redacted");
    assert_eq!(
        out["nested"]["authorization"], REDACTED,
        "nested secret must be redacted"
    );
    assert_eq!(out["ok"], true, "non-secret fields must survive");
}

#[test]
fn redactor_none_preserves_values() {
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(LeakyTool);
    let runtime = Runtime::new(ledger, tools, PolicyEngine::new().with(WritAuthorityPolicy))
        .with_redactor(Redactor::none());

    let (writ, _) = signed_writ();
    let mut cog = MockCognition::new(vec![vec![intent()]], Some("done".into()));
    let summary = run_agent(
        &runtime,
        &mut cog,
        "noredact",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    let out = committed_output(&runtime, summary.trajectory_id);
    assert_eq!(out["api_key"], "sk-super-secret", "none() must pass through");
}
