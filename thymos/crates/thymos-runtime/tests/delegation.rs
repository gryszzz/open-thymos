//! Phase II — multi-agent delegation, proven end to end.
//!
//! A parent agent mints a child writ that is a *strict subset* of its own
//! authority, the child executes on its own trajectory, and the ledger records
//! the parent→child lineage. This test asserts the four properties that make
//! delegation safe:
//!
//! 1. **Child writ ⊆ parent writ** — the minted child authority is a verified
//!    strict subset (narrower tool scope, halved budget, decremented depth).
//! 2. **Tenant boundaries cannot be crossed by delegation** — a child claiming a
//!    different tenant is rejected by `verify_subset_of`.
//! 3. **Lineage is recorded** — the parent trajectory carries a `Delegation`
//!    edge to the child trajectory, and the child's root notes the task.
//! 4. **Parent state isn't mutated by the child** — the child runs on a separate
//!    trajectory with its own world; the parent's committed state is neither
//!    visible to nor changed by the child. The only thing that mutates a
//!    trajectory's world is a commit *on that trajectory*.
//!
//! Finally, replay reconstructs *both* trajectories deterministically.

use serde_json::json;

use thymos_ledger::{replay, EntryPayload, Ledger, ReplayConfig};
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, CoreIntent, DelegationBounds, DelegationKeyring,
    EffectCeiling, IntentBody, IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{DelegateTool, KvGetTool, KvSetTool, ToolRegistry};

const TENANT: &str = "acme";

fn act(target: &str, args: serde_json::Value, nonce: u8) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "cognition".into(),
        kind: IntentKind::Act,
        target: target.into(),
        args,
        rationale: format!("act on {target}"),
        nonce: [nonce; 16],
    })
    .unwrap()
}

#[test]
fn delegation_mints_subset_child_records_lineage_and_isolates_state() {
    // ── Runtime with a delegation keyring (so child writs get signed). ──
    let mut tools = ToolRegistry::new();
    tools.register(KvSetTool::default());
    tools.register(KvGetTool::default());
    tools.register(DelegateTool::default());

    let keyring = DelegationKeyring::new();
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    )
    .with_delegation_keyring(keyring.clone());

    // ── Parent writ: tenant `acme`, may use kv_* and delegate, depth 2. ──
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();
    let agent_pubkey = public_key_of(&agent_key);
    let parent_writ = Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "ops-agent".into(),
            subject_pubkey: agent_pubkey,
            nonce: [0u8; 16],
            parent: None,
            tenant_id: TENANT.into(),
            tool_scopes: vec![ToolPattern::exact("kv_*"), ToolPattern::exact("delegate")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 100,
                wall_clock_ms: 600_000,
                usd_millicents: 0,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow { not_before: 0, expires_at: u64::MAX },
            delegation: DelegationBounds { max_depth: 2, may_subdivide: true },
        },
        &root_key,
    )
    .unwrap();
    // The runtime needs the parent subject's signing key to mint signed children.
    keyring.register(agent_key);

    // ── Parent commits a write on its own trajectory. ──
    let parent = runtime.create_run("delegation demo", b"parent-v1").unwrap();
    let s = parent
        .submit(act("kv_set", json!({"key":"order","value":"received"}), 1), &parent_writ)
        .unwrap();
    assert!(matches!(s, Step::Committed(_)), "parent write should commit");

    // ── Parent delegates an audit sub-task, restricting the child to kv_get. ──
    let delegate_intent = CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "cognition".into(),
        kind: IntentKind::Delegate,
        target: "auditor".into(),
        args: json!({ "task": "audit the order", "tool_scopes": ["kv_get"] }),
        rationale: "hand off a read-only audit under a narrower writ".into(),
        nonce: [2u8; 16],
    })
    .unwrap();
    let child_traj = match parent.submit(delegate_intent, &parent_writ).unwrap() {
        Step::Delegated { child_trajectory_id, .. } => child_trajectory_id,
        other => panic!("expected delegation, got {other:?}"),
    };

    // ── (1) The minted child writ is a strict subset of the parent. ──
    let child_writ = keyring
        .take_pending_child_writ(child_traj)
        .expect("a signed child writ should have been minted and stashed");
    assert!(
        child_writ.body.verify_subset_of(&parent_writ.body).is_ok(),
        "child writ must be a strict subset of the parent"
    );
    child_writ.verify_signature().expect("child writ is validly signed");
    assert_eq!(child_writ.body.tenant_id, TENANT, "child inherits parent tenant");
    assert_eq!(child_writ.body.parent, Some(parent_writ.id), "child points at parent writ");
    assert_eq!(child_writ.body.delegation.max_depth, 1, "delegation depth decremented");
    assert!(child_writ.body.budget.tool_calls <= parent_writ.body.budget.tool_calls / 2 + 1);
    // Narrower authority: child may read but not write.
    assert!(child_writ.authorizes_tool("kv_get"), "child keeps the granted kv_get scope");
    assert!(!child_writ.authorizes_tool("kv_set"), "child does NOT inherit kv_set");

    // ── (2) Tenant boundaries cannot be crossed by delegation. ──
    let mut cross_tenant = child_writ.body.clone();
    cross_tenant.tenant_id = "evil-corp".into();
    assert!(
        cross_tenant.verify_subset_of(&parent_writ.body).is_err(),
        "a child claiming a different tenant must be rejected"
    );

    // ── Drive the child on its own trajectory. ──
    let child = runtime.resume_run(child_traj).unwrap();
    // The child reads — allowed by its writ. Its world is its own: the parent's
    // `order=received` is NOT visible here (trajectory-isolated state).
    let s = child.submit(act("kv_get", json!({"key":"order"}), 3), &child_writ).unwrap();
    assert!(matches!(s, Step::Committed(_)), "child read should commit");
    // The child attempts a write it was not granted — rejected at the writ boundary.
    let s = child.submit(act("kv_set", json!({"key":"order","value":"shipped"}), 4), &child_writ).unwrap();
    assert!(matches!(s, Step::Rejected(_)), "child write must be rejected (kv_set not in child scope)");

    // ── (3) Lineage is recorded on the ledger. ──
    let parent_entries = runtime.ledger.entries(parent.trajectory_id()).unwrap();
    let edge = parent_entries.iter().find_map(|e| match &e.payload {
        EntryPayload::Delegation { child_trajectory_id, task, .. } => Some((*child_trajectory_id, task.clone())),
        _ => None,
    });
    let (edge_child, edge_task) = edge.expect("parent trajectory records a Delegation edge");
    assert_eq!(edge_child, child_traj, "edge points at the child trajectory");
    assert_eq!(edge_task, "audit the order");

    let child_entries = runtime.ledger.entries(child_traj).unwrap();
    match &child_entries[0].payload {
        EntryPayload::Root { note, trajectory_id } => {
            assert!(note.contains("delegated"), "child root notes the delegation: {note}");
            assert_eq!(*trajectory_id, child_traj);
        }
        other => panic!("child trajectory must start with a Root, got {other:?}"),
    }

    // ── (4) Parent state isn't mutated by the child. ──
    let parent_world = parent.project_world().unwrap();
    let order = parent_world
        .resources
        .iter()
        .find(|(k, _)| k.id == "order")
        .map(|(_, v)| v.value.clone());
    assert_eq!(order, Some(json!("received")), "parent world unchanged by the child");
    // The child's world never saw the parent's `order` resource.
    let child_world = child.project_world().unwrap();
    assert!(
        !child_world.resources.iter().any(|(k, _)| k.id == "order"),
        "child trajectory is state-isolated from the parent"
    );
    // No child commit leaked onto the parent trajectory.
    assert!(
        !parent_entries.iter().any(|e| e.trajectory_id == child_traj),
        "child commits stay on the child trajectory"
    );

    // ── Replay reconstructs BOTH trajectories deterministically. ──
    let (pworld, preport) = replay(&parent_entries, &ReplayConfig::default()).unwrap();
    assert!(preport.commits_replayed >= 1);
    assert_eq!(
        pworld.resources.iter().find(|(k, _)| k.id == "order").map(|(_, v)| v.value.clone()),
        Some(json!("received")),
        "parent replay folds back to the same world"
    );
    let (_cworld, creport) = replay(&child_entries, &ReplayConfig::default()).unwrap();
    assert_eq!(creport.commits_replayed, 1, "child replay verifies its single committed read");
}
