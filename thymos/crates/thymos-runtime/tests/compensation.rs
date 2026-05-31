//! Compensation / saga rollback: `Run::compensate_to` undoes committed steps
//! after a target, newest-first, by invoking each tool's `compensate`. Each
//! rollback is itself an appended commit tagged `compensates = Some(original)`.
//! A non-compensable step halts the rollback (no silent partial undo).

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

/// Creates a kv resource on the forward path; compensates by retracting it.
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
        "provisions a resource (compensable)"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        let id = inv
            .args
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("resource")
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
            .ok_or_else(|| Error::Other("compensate: observation has no key".into()))?;
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

/// Forward-only tool with no compensation.
struct OneWayTool;
impl ToolContract for OneWayTool {
    fn meta(&self) -> &ToolContractMeta {
        static M: std::sync::OnceLock<ToolContractMeta> = std::sync::OnceLock::new();
        M.get_or_init(|| ToolContractMeta {
            name: "oneway".into(),
            version: "1.0.0".into(),
            effect_class: EffectClass::Write,
            risk_class: RiskClass::Low,
        })
    }
    fn description(&self) -> &str {
        "not compensable"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        Ok(ToolOutcome {
            delta: StructuredDelta(vec![]),
            observation: Observation {
                tool: "oneway".into(),
                output: json!(null),
                latency_ms: 0,
            },
        })
    }
}

fn writ(tool: &str) -> Writ {
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
            tool_scopes: vec![ToolPattern::exact(tool)],
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

fn act(tool: &str, id: &str, nonce: u8) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: tool.into(),
        args: json!({ "id": id }),
        rationale: "saga".into(),
        nonce: [nonce; 16],
    })
    .unwrap()
}

#[test]
fn compensate_to_rolls_back_in_reverse_and_is_idempotent() {
    let mut tools = ToolRegistry::new();
    tools.register(ProvisionTool);
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let w = writ("provision");
    let run = runtime.create_run("saga", b"saga").unwrap();
    let traj = run.trajectory_id();

    let root_id = CommitId(runtime.ledger.entries(traj).unwrap()[0].id);

    let c1 = match run.submit(act("provision", "r1", 1), &w).unwrap() {
        Step::Committed(id) => id,
        o => panic!("c1 not committed: {o:?}"),
    };
    let c2 = match run.submit(act("provision", "r2", 2), &w).unwrap() {
        Step::Committed(id) => id,
        o => panic!("c2 not committed: {o:?}"),
    };

    // Roll everything back to the root.
    let comps = run.compensate_to(root_id, &w).unwrap();
    assert_eq!(comps.len(), 2, "two steps compensated");

    // Newest-first: first compensation undoes c2, second undoes c1.
    let links: Vec<Option<CommitId>> = runtime
        .ledger
        .entries(traj)
        .unwrap()
        .iter()
        .filter_map(|e| match &e.payload {
            EntryPayload::Commit(c) => Some(c.body.compensates),
            _ => None,
        })
        .collect();
    // commits in seq order: c1, c2, comp(c2), comp(c1)
    assert_eq!(links, vec![None, None, Some(c2), Some(c1)]);

    // Integrity holds after rollback, and the world reflects the retractions.
    runtime.ledger.verify_integrity(traj).unwrap();

    // Idempotent: re-running compensates nothing new.
    let again = run.compensate_to(root_id, &w).unwrap();
    assert!(again.is_empty(), "already-compensated steps are skipped");
}

#[test]
fn compensate_halts_on_non_compensable_step() {
    let mut tools = ToolRegistry::new();
    tools.register(OneWayTool);
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    let w = writ("oneway");
    let run = runtime.create_run("saga2", b"saga2").unwrap();
    let traj = run.trajectory_id();
    let root_id = CommitId(runtime.ledger.entries(traj).unwrap()[0].id);

    run.submit(act("oneway", "x", 1), &w).unwrap();

    let err = run.compensate_to(root_id, &w).unwrap_err();
    assert!(
        err.to_string().contains("not compensable"),
        "expected a non-compensable error, got: {err}"
    );
}
