//! Routing evidence (WisePick / Option 2): evidence attached at submit time is
//! recorded into the committed (ledgered) record — durable and replay-safe — and
//! does not affect the proposal id.

use serde_json::{json, Value};

use thymos_core::{
    commit::Observation, delta::StructuredDelta, error::Result, proposal::FallbackHint,
    RoutingEvidence,
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

struct NoopTool;
impl ToolContract for NoopTool {
    fn meta(&self) -> &ToolContractMeta {
        static M: std::sync::OnceLock<ToolContractMeta> = std::sync::OnceLock::new();
        M.get_or_init(|| ToolContractMeta {
            name: "act".into(),
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
        Ok(ToolOutcome {
            delta: StructuredDelta(vec![]),
            observation: Observation {
                tool: "act".into(),
                output: json!(null),
                latency_ms: 0,
            },
        })
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
            tool_scopes: vec![ToolPattern::exact("act")],
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

fn evidence() -> RoutingEvidence {
    RoutingEvidence {
        decision_hash: "abc123".into(),
        selected: "anthropic:claude".into(),
        alternatives: vec!["openai:gpt".into()],
        confidence_bps: 9000,
        reason_codes: vec!["cost_optimal".into()],
        latency_estimate_ms: 700,
        cost_estimate_millicents: 1234,
        fallback_hint: Some(FallbackHint {
            provider: "openai".into(),
            model: Some("gpt-4o".into()),
            reason: "primary overloaded".into(),
        }),
    }
}

fn intent() -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: "act".into(),
        args: json!({}),
        rationale: "routing".into(),
        nonce: [7; 16],
    })
    .unwrap()
}

#[test]
fn routing_evidence_is_recorded_in_the_commit_and_replay_safe() {
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool);
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let w = writ();
    let run = runtime.create_run("re", b"re").unwrap();

    let committed = run
        .submit_with_routing_evidence(intent(), &w, evidence())
        .unwrap();
    assert!(matches!(committed, Step::Committed(_)));

    // The committed ledger entry carries the routing evidence verbatim.
    let entries = runtime.ledger.entries(run.trajectory_id()).unwrap();
    let recorded = entries
        .iter()
        .find_map(|e| match &e.payload {
            EntryPayload::Commit(c) => c.body.routing_evidence.clone(),
            _ => None,
        })
        .expect("commit should carry routing_evidence");
    assert_eq!(recorded, evidence());
    assert_eq!(recorded.confidence_bps, 9000);
    assert_eq!(recorded.cost_estimate_millicents, 1234);

    // It is bound into the hash chain (integrity holds with it present).
    runtime.ledger.verify_integrity(run.trajectory_id()).unwrap();
}

#[test]
fn submit_without_evidence_omits_it() {
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool);
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let w = writ();
    let run = runtime.create_run("re2", b"re2").unwrap();
    run.submit(intent(), &w).unwrap();

    let entries = runtime.ledger.entries(run.trajectory_id()).unwrap();
    let has_evidence = entries.iter().any(|e| match &e.payload {
        EntryPayload::Commit(c) => c.body.routing_evidence.is_some(),
        _ => false,
    });
    assert!(!has_evidence, "no evidence attached → field stays None");
}
