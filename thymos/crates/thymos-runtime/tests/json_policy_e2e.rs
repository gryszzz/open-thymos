//! End-to-end: a declarative JSON policy bundle governs a real run — a tool the
//! writ authorizes by name is still denied by a loaded policy rule, and a
//! threshold rule suspends for approval. Proves the policy language plugs into
//! the live Intent → Proposal → Commit pipeline.

use serde_json::{json, Value};

use thymos_core::{commit::Observation, delta::StructuredDelta, error::Result};
use thymos_ledger::Ledger;
use thymos_policy::{JsonPolicySet, PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, CoreIntent, DelegationBounds, EffectCeiling,
    IntentBody, IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

struct NoopTool {
    name: &'static str,
}
impl ToolContract for NoopTool {
    fn meta(&self) -> &ToolContractMeta {
        Box::leak(Box::new(ToolContractMeta {
            name: self.name.into(),
            version: "1.0.0".into(),
            effect_class: EffectClass::Write,
            risk_class: RiskClass::Low,
        }))
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
                tool: self.name.into(),
                output: json!(null),
                latency_ms: 0,
            },
        })
    }
}

const BUNDLE: &str = r#"{
  "name": "ops.policy",
  "version": "1",
  "rules": [
    { "name": "no-danger",
      "when": { "field": "intent.target", "op": "eq", "value": "danger" },
      "decision": { "kind": "deny", "reason": "danger tool is forbidden by policy bundle" } },
    { "name": "big-spend",
      "when": { "field": "intent.args.amount", "op": "gt", "value": 1000 },
      "decision": { "kind": "require_approval", "channel": "ops", "reason": "amount over 1000" } }
  ]
}"#;

fn runtime() -> Runtime {
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool { name: "danger" });
    tools.register(NoopTool { name: "pay" });
    let policy = PolicyEngine::new()
        .with(WritAuthorityPolicy)
        .with(JsonPolicySet::from_json(BUNDLE).unwrap());
    Runtime::new(Ledger::open_in_memory().unwrap(), tools, policy)
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
            // Writ authorizes BOTH tools by name; the JSON bundle is what governs.
            tool_scopes: vec![ToolPattern::exact("danger"), ToolPattern::exact("pay")],
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

fn intent(target: &str, args: Value) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: target.into(),
        args,
        rationale: "json policy".into(),
        nonce: [1; 16],
    })
    .unwrap()
}

#[test]
fn json_bundle_denies_an_in_scope_tool() {
    let rt = runtime();
    let w = writ();
    let run = rt.create_run("jp", b"jp").unwrap();
    // 'danger' is in the writ's scope but the bundle denies it.
    match run.submit(intent("danger", json!({})), &w).unwrap() {
        Step::Rejected(reason) => assert!(reason.to_string().contains("forbidden by policy bundle")),
        other => panic!("expected policy denial, got {other:?}"),
    }
}

#[test]
fn json_bundle_threshold_suspends_for_approval() {
    let rt = runtime();
    let w = writ();
    let run = rt.create_run("jp2", b"jp2").unwrap();
    match run.submit(intent("pay", json!({ "amount": 5000 })), &w).unwrap() {
        Step::Suspended { channel, .. } => assert_eq!(channel, "ops"),
        other => panic!("expected suspension, got {other:?}"),
    }
    // Under the threshold → permitted (commits).
    match run.submit(intent("pay", json!({ "amount": 10 })), &w).unwrap() {
        Step::Committed(_) => {}
        other => panic!("expected commit, got {other:?}"),
    }
}
