//! Cross-trajectory compensation: rolling back a parent trajectory also unwinds
//! the committed work of any child trajectory it delegated to (recursively).

use serde_json::{json, Value};

use thymos_core::{
    commit::Observation,
    delta::{DeltaOp, StructuredDelta},
    error::{Error, Result},
    world::ResourceKey,
    CommitId,
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

/// Compensable tool: creates a kv on the forward path, retracts it to compensate.
struct ProvisionTool;
impl ToolContract for ProvisionTool {
    fn meta(&self) -> &ToolContractMeta {
        static M: std::sync::OnceLock<ToolContractMeta> = std::sync::OnceLock::new();
        M.get_or_init(|| ToolContractMeta {
            name: "provision".into(),
            version: "1.0.0".into(),
            effect_class: EffectClass::Write,
            risk_class: RiskClass::Medium,
        })
    }
    fn description(&self) -> &str {
        "provision (compensable)"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let id = inv
            .args
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("r")
            .to_string();
        Ok(ToolOutcome {
            delta: StructuredDelta::single(DeltaOp::Create {
                kind: "kv".into(),
                id: id.clone(),
                value: json!("active"),
            }),
            observation: Observation {
                tool: "provision".into(),
                output: json!({ "key": id }),
                latency_ms: 0,
            },
        })
    }
    fn compensable(&self) -> bool {
        true
    }
    fn compensate(
        &self,
        observation: &Observation,
        world: &thymos_core::world::World,
    ) -> Result<ToolOutcome> {
        let key = observation
            .output
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Other("no key".into()))?;
        let version = world
            .get(&ResourceKey::new("kv", key))
            .map(|s| s.version)
            .unwrap_or(1);
        Ok(ToolOutcome {
            delta: StructuredDelta::single(DeltaOp::Retract {
                kind: "kv".into(),
                id: key.into(),
                expected_version: version,
                reason: "compensated".into(),
            }),
            observation: Observation {
                tool: "provision".into(),
                output: json!({ "compensated": key }),
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
            tool_scopes: vec![ToolPattern::exact("provision")],
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
                max_depth: 2,
                may_subdivide: true,
            },
        },
        &issuer,
    )
    .unwrap()
}

fn act(id: &str, nonce: u8) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: "provision".into(),
        args: json!({ "id": id }),
        rationale: "x".into(),
        nonce: [nonce; 16],
    })
    .unwrap()
}

#[test]
fn parent_rollback_compensates_delegated_child_work() {
    let mut tools = ToolRegistry::new();
    tools.register(ProvisionTool);
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let w = writ();

    // A child trajectory that committed real work.
    let child = runtime.create_run("child", b"child").unwrap();
    let child_traj = child.trajectory_id();
    let child_commit = match child.submit(act("child-res", 1), &w).unwrap() {
        Step::Committed(id) => id,
        o => panic!("child not committed: {o:?}"),
    };

    // A parent trajectory that delegated to that child.
    let parent = runtime.create_run("parent", b"parent").unwrap();
    let parent_traj = parent.trajectory_id();
    runtime
        .ledger
        .append_delegation(parent_traj, child_traj, "do work", None)
        .unwrap();

    // Roll the parent back to its root.
    let parent_root = CommitId(runtime.ledger.entries(parent_traj).unwrap()[0].id);
    let comps = parent.compensate_to(parent_root, &w).unwrap();
    assert_eq!(comps.len(), 1, "the child's one commit was compensated via the parent rollback");

    // The compensation landed in the CHILD trajectory, linked to the child commit.
    let child_links: Vec<Option<CommitId>> = runtime
        .ledger
        .entries(child_traj)
        .unwrap()
        .iter()
        .filter_map(|e| match &e.payload {
            EntryPayload::Commit(c) => Some(c.body.compensates),
            _ => None,
        })
        .collect();
    assert_eq!(child_links, vec![None, Some(child_commit)]);

    runtime.ledger.verify_integrity(child_traj).unwrap();
    runtime.ledger.verify_integrity(parent_traj).unwrap();

    // Idempotent: re-running compensates nothing new.
    assert!(parent.compensate_to(parent_root, &w).unwrap().is_empty());
}
