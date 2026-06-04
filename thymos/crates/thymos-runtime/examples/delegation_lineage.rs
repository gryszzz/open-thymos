//! `delegation_lineage` — multi-agent delegation you can watch.
//!
//! A parent agent (tenant `acme`) commits a write, then **delegates** a
//! read-only audit sub-task to a child running under a *strict subset* of its
//! authority. The child executes on its own trajectory; the ledger records the
//! parent→child lineage; the child cannot exceed the authority it was granted,
//! cannot cross the tenant boundary, and cannot mutate the parent's state.
//!
//! Run:
//!     cargo run --example delegation_lineage -p thymos-runtime
//!
//! The same flow is asserted property-by-property in
//! `tests/delegation.rs`.

use serde_json::json;

use thymos_ledger::{replay, EntryPayload, Ledger, ReplayConfig};
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, CoreIntent, DelegationBounds, DelegationKeyring,
    EffectCeiling, IntentBody, IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{DelegateTool, KvGetTool, KvSetTool, ToolRegistry};

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

fn main() -> anyhow::Result<()> {
    let mut tools = ToolRegistry::new();
    tools.register(KvSetTool::default());
    tools.register(KvGetTool::default());
    tools.register(DelegateTool::default());

    // A keyring lets the runtime sign minted child writs. Keep a handle so we
    // can pick up the child writ after delegating.
    let keyring = DelegationKeyring::new();
    let runtime = Runtime::new(
        Ledger::open_in_memory()?,
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    )
    .with_delegation_keyring(keyring.clone());

    // Parent writ: tenant `acme`, may use kv_* and delegate, delegation depth 2.
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
            tenant_id: "acme".into(),
            tool_scopes: vec![ToolPattern::exact("kv_*"), ToolPattern::exact("delegate")],
            budget: Budget { tokens: 10_000, tool_calls: 100, wall_clock_ms: 600_000, usd_millicents: 0 },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow { not_before: 0, expires_at: u64::MAX },
            delegation: DelegationBounds { max_depth: 2, may_subdivide: true },
        },
        &root_key,
    )?;
    keyring.register(agent_key);
    println!("== parent writ: subject=ops-agent tenant=acme scopes=[kv_*, delegate] depth=2");

    // 1. Parent commits a write on its own trajectory.
    let parent = runtime.create_run("delegation demo", b"parent-v1")?;
    println!("\n-> parent trajectory: {}", parent.trajectory_id());
    let s = parent.submit(act("kv_set", json!({"key":"order","value":"received"}), 1), &parent_writ)?;
    println!("   parent kv_set(order=received): {s:?}");

    // 2. Parent delegates a read-only audit, restricting the child to kv_get.
    let delegate_intent = CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "cognition".into(),
        kind: IntentKind::Delegate,
        target: "auditor".into(),
        args: json!({ "task": "audit the order", "tool_scopes": ["kv_get"] }),
        rationale: "hand off a read-only audit under a narrower writ".into(),
        nonce: [2u8; 16],
    })?;
    let child_traj = match parent.submit(delegate_intent, &parent_writ)? {
        Step::Delegated { child_trajectory_id, .. } => child_trajectory_id,
        other => anyhow::bail!("expected delegation, got {other:?}"),
    };
    println!("\n-> delegated 'audit the order' → child trajectory: {child_traj}");

    // 3. Inspect the minted child writ: a strict subset of the parent.
    let child_writ = keyring
        .take_pending_child_writ(child_traj)
        .expect("a signed child writ was minted");
    println!("   child writ ⊆ parent? {}", child_writ.body.verify_subset_of(&parent_writ.body).is_ok());
    println!(
        "   child: tenant={} scopes=[kv_get] depth={} can_write={}",
        child_writ.body.tenant_id,
        child_writ.body.delegation.max_depth,
        child_writ.authorizes_tool("kv_set"),
    );

    // Tenant isolation is structural: a child claiming a different tenant is void.
    let mut cross = child_writ.body.clone();
    cross.tenant_id = "evil-corp".into();
    println!(
        "   cross-tenant child rejected? {}",
        cross.verify_subset_of(&parent_writ.body).is_err()
    );

    // 4. Drive the child on its own trajectory.
    let child = runtime.resume_run(child_traj)?;
    let s = child.submit(act("kv_get", json!({"key":"order"}), 3), &child_writ)?;
    println!("\n-> child kv_get(order): {s:?}  (reads its OWN world — parent state not visible)");
    let s = child.submit(act("kv_set", json!({"key":"order","value":"shipped"}), 4), &child_writ)?;
    println!("   child kv_set(order=shipped): {s:?}  (rejected — kv_set not in child scope)");

    // 5. Lineage on the ledger.
    let parent_entries = runtime.ledger.entries(parent.trajectory_id())?;
    println!("\n== parent trajectory entries");
    for e in &parent_entries {
        let label = match &e.payload {
            EntryPayload::Root { note, .. } => format!("Root({note})"),
            EntryPayload::Commit(c) => format!("Commit seq={}", c.body.seq),
            EntryPayload::Delegation { child_trajectory_id, task, .. } => {
                format!("Delegation(task={task:?} → {child_trajectory_id})")
            }
            other => format!("{other:?}"),
        };
        println!("   seq={} {label}", e.seq);
    }

    // 6. Parent state is unchanged by the child; replay reconstructs both.
    let parent_world = parent.project_world()?;
    let order = parent_world.resources.iter().find(|(k, _)| k.id == "order").map(|(_, v)| &v.value);
    println!("\n== parent world: order = {}", serde_json::to_string(&order)?);

    let (_pw, pr) = replay(&parent_entries, &ReplayConfig::default())?;
    let child_entries = runtime.ledger.entries(child_traj)?;
    let (_cw, cr) = replay(&child_entries, &ReplayConfig::default())?;
    println!(
        "== replay: parent {} commits verified, child {} commits verified",
        pr.commits_replayed, cr.commits_replayed
    );

    println!("\nthymos: a parent grants less than it holds; the ledger remembers who did what.");
    Ok(())
}
