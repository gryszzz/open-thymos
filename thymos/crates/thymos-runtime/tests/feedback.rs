//! Routing-feedback export is safe: it carries only the routing decision id,
//! the chosen route, a coarse status, and latency — never intent args, tool
//! output, tenant, writ, or any free text — and is derived purely from the
//! ledger (a pull, no egress).

use serde_json::{json, Value};

use thymos_core::{
    commit::Observation, delta::StructuredDelta, error::Result, proposal::FallbackHint,
    RoutingEvidence,
};
use thymos_ledger::Ledger;
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, CoreIntent, DelegationBounds, EffectCeiling,
    IntentBody, IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

struct SecretTool;
impl ToolContract for SecretTool {
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
                // A field a naive exporter might leak — must NOT appear in feedback.
                output: json!({ "secret": "do-not-export" }),
                latency_ms: 42,
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
            tenant_id: "secret-tenant".into(),
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
        // Sensitive args a naive exporter might leak — must NOT appear.
        args: json!({ "password": "hunter2" }),
        rationale: "secret rationale".into(),
        nonce: [7; 16],
    })
    .unwrap()
}

#[test]
fn routing_outcomes_export_only_safe_fields() {
    let mut tools = ToolRegistry::new();
    tools.register(SecretTool);
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let run = runtime.create_run("fb", b"fb").unwrap();
    assert!(matches!(
        run.submit_with_routing_evidence(intent(), &writ(), evidence()).unwrap(),
        Step::Committed(_)
    ));

    let outcomes = run.routing_outcomes().unwrap();
    assert_eq!(outcomes.len(), 1);
    let o = &outcomes[0];
    assert_eq!(o.decision_hash, "abc123");
    assert_eq!(o.selected, "anthropic:claude");
    assert_eq!(o.status, "committed");
    assert_eq!(o.latency_ms, 42);

    // Hard guarantee: the serialized export has EXACTLY the four safe keys, and
    // none of the sensitive material from intent/observation/writ.
    let serialized = serde_json::to_string(o).unwrap();
    let v: Value = serde_json::from_str(&serialized).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
    assert_eq!(keys.len(), 4, "exactly four fields exported: {keys:?}");
    for leak in [
        "password", "hunter2", "secret", "do-not-export", "secret-tenant",
        "secret rationale", "writ", "tenant", "args", "output",
    ] {
        assert!(
            !serialized.contains(leak),
            "feedback must not leak '{leak}': {serialized}"
        );
    }
}

#[test]
fn no_routing_evidence_means_no_outcomes() {
    let mut tools = ToolRegistry::new();
    tools.register(SecretTool);
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let run = runtime.create_run("fb2", b"fb2").unwrap();
    run.submit(intent(), &writ()).unwrap();
    assert!(run.routing_outcomes().unwrap().is_empty());
}
