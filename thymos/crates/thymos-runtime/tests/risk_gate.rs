//! Risk gate (compiler stage 9c): when a threshold is set, a tool whose
//! declared `RiskClass` is at or above it is escalated to require operator
//! approval, even on a bare policy permit. Lower-risk tools and the
//! no-threshold default are unaffected.

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

struct RiskyTool {
    risk: RiskClass,
}
impl ToolContract for RiskyTool {
    fn meta(&self) -> &ToolContractMeta {
        Box::leak(Box::new(ToolContractMeta {
            name: "risky_write".into(),
            version: "1.0.0".into(),
            effect_class: EffectClass::Write,
            risk_class: self.risk,
        }))
    }
    fn description(&self) -> &str {
        "a write whose risk class drives the 9c gate"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        Ok(ToolOutcome {
            delta: StructuredDelta(vec![]),
            observation: Observation {
                tool: "risky_write".into(),
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
            tool_scopes: vec![ToolPattern::exact("risky_write")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 100,
                wall_clock_ms: 600_000,
                usd_millicents: 1_000_000,
            },
            effect_ceiling: EffectCeiling {
                read: true,
                write: true,
                external: false,
                irreversible: false,
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
        target: "risky_write".into(),
        args: json!({}),
        rationale: "gate".into(),
        nonce: [7; 16],
    })
    .unwrap()
}

fn runtime(risk: RiskClass, threshold: Option<RiskClass>) -> Runtime {
    let mut tools = ToolRegistry::new();
    tools.register(RiskyTool { risk });
    Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    )
    .with_approve_risk_at_or_above(threshold)
}

#[test]
fn at_threshold_suspends() {
    let rt = runtime(RiskClass::High, Some(RiskClass::High));
    let run = rt.create_run("r1", b"r1").unwrap();
    match run.submit(act(), &writ()).unwrap() {
        Step::Suspended { channel, reason } => {
            assert_eq!(channel, "high-risk");
            assert!(reason.contains("risky_write"));
        }
        other => panic!("expected suspension, got {other:?}"),
    }
    // The ledger's pending_approval entry must expose the full proposed plan
    // at payload.PendingApproval.proposal.body.plan — the desktop's approval
    // card reads exactly this path to render the concrete action (diff/cmd).
    let entries = rt
        .ledger
        .query_entries(None, Some("pending_approval"), None, None, Some(10))
        .unwrap();
    assert_eq!(entries.len(), 1);
    let v = serde_json::to_value(&entries[0].payload).unwrap();
    assert_eq!(v["type"].as_str(), Some("pending_approval"));
    assert_eq!(
        v.pointer("/proposal/body/plan/tool").and_then(|t| t.as_str()),
        Some("risky_write")
    );
    assert!(v.pointer("/proposal/body/plan/args").is_some());
}

#[test]
fn above_threshold_suspends() {
    let rt = runtime(RiskClass::Critical, Some(RiskClass::High));
    let run = rt.create_run("r2", b"r2").unwrap();
    assert!(matches!(
        run.submit(act(), &writ()).unwrap(),
        Step::Suspended { .. }
    ));
}

#[test]
fn below_threshold_commits() {
    let rt = runtime(RiskClass::Medium, Some(RiskClass::High));
    let run = rt.create_run("r3", b"r3").unwrap();
    assert!(matches!(
        run.submit(act(), &writ()).unwrap(),
        Step::Committed(_)
    ));
}

#[test]
fn no_threshold_is_default_behavior() {
    let rt = runtime(RiskClass::Critical, None);
    let run = rt.create_run("r4", b"r4").unwrap();
    assert!(matches!(
        run.submit(act(), &writ()).unwrap(),
        Step::Committed(_)
    ));
}
