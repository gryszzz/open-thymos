//! Compensation gate (compiler stage 9b): when enabled, an irreversible tool
//! that is not compensable is escalated to require approval, even on a bare
//! policy permit. A compensable irreversible tool proceeds. Off by default.

use serde_json::{json, Value};

use thymos_core::{commit::Observation, delta::StructuredDelta, error::Result};
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

struct IrreversibleTool {
    compensable: bool,
}
impl ToolContract for IrreversibleTool {
    fn meta(&self) -> &ToolContractMeta {
        Box::leak(Box::new(ToolContractMeta {
            name: "wire_transfer".into(),
            version: "1.0.0".into(),
            effect_class: EffectClass::Irreversible,
            risk_class: RiskClass::Critical,
        }))
    }
    fn description(&self) -> &str {
        "irreversible action"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        Ok(ToolOutcome {
            delta: StructuredDelta(vec![]),
            observation: Observation {
                tool: "wire_transfer".into(),
                output: json!(null),
                latency_ms: 0,
            },
        })
    }
    fn compensable(&self) -> bool {
        self.compensable
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
            tool_scopes: vec![ToolPattern::exact("wire_transfer")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 100,
                wall_clock_ms: 600_000,
                usd_millicents: 1_000_000,
            },
            // Grant irreversible so the effect-ceiling gate (5b) admits it; 9b is
            // what this test exercises.
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

fn act() -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: "wire_transfer".into(),
        args: json!({}),
        rationale: "gate".into(),
        nonce: [5; 16],
    })
    .unwrap()
}

fn runtime(compensable: bool, gate_on: bool) -> Runtime {
    let mut tools = ToolRegistry::new();
    tools.register(IrreversibleTool { compensable });
    let rt = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    if gate_on {
        rt.with_require_compensation_for_irreversible(true)
    } else {
        rt
    }
}

#[test]
fn gate_on_suspends_irreversible_uncompensable() {
    let rt = runtime(false, true);
    let run = rt.create_run("g1", b"g1").unwrap();
    match run.submit(act(), &writ()).unwrap() {
        Step::Suspended { channel, .. } => assert_eq!(channel, "irreversible-uncompensable"),
        other => panic!("expected suspension, got {other:?}"),
    }
}

#[test]
fn gate_on_allows_compensable_irreversible() {
    let rt = runtime(true, true);
    let run = rt.create_run("g2", b"g2").unwrap();
    assert!(matches!(
        run.submit(act(), &writ()).unwrap(),
        Step::Committed(_)
    ));
}

#[test]
fn gate_off_is_default_behavior() {
    let rt = runtime(false, false);
    let run = rt.create_run("g3", b"g3").unwrap();
    assert!(matches!(
        run.submit(act(), &writ()).unwrap(),
        Step::Committed(_)
    ));
}
