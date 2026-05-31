//! Clock attestation: the runtime's writ time-window check uses the injected
//! clock, not the host wall clock — so an attested/pinned time source decides
//! whether a writ is in-window.

use serde_json::{json, Value};

use thymos_core::{commit::Observation, delta::StructuredDelta, error::Result};
use thymos_ledger::Ledger;
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, Clock, CoreIntent, DelegationBounds, EffectCeiling,
    FixedClock, IntentBody, IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
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
        "noop"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
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

/// Writ valid only in [100, 1000].
fn windowed_writ() -> Writ {
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
                not_before: 100,
                expires_at: 1000,
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
        rationale: "clock".into(),
        nonce: [3; 16],
    })
    .unwrap()
}

fn runtime_with_clock(clock: std::sync::Arc<dyn Clock>) -> Runtime {
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool);
    Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    )
    .with_clock(clock)
}

#[test]
fn injected_clock_inside_window_permits() {
    let runtime = runtime_with_clock(std::sync::Arc::new(FixedClock(500)));
    let run = runtime.create_run("clock-ok", b"clock-ok").unwrap();
    assert!(matches!(
        run.submit(act(), &windowed_writ()).unwrap(),
        Step::Committed(_)
    ));
}

#[test]
fn injected_clock_after_window_rejects() {
    // Host wall clock (now ~2025+) would also be outside [100,1000], but the
    // point is the *injected* clock decides — here a pinned 5000.
    let runtime = runtime_with_clock(std::sync::Arc::new(FixedClock(5000)));
    let run = runtime.create_run("clock-late", b"clock-late").unwrap();
    match run.submit(act(), &windowed_writ()).unwrap() {
        Step::Rejected(reason) => assert!(
            reason.to_string().contains("time window"),
            "expected time-window rejection, got: {reason}"
        ),
        other => panic!("expected rejection, got {other:?}"),
    }
}
