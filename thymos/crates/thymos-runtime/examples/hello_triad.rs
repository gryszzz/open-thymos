//! `hello_triad` — demonstrates the Thymos Intent → Proposal → Commit cycle
//! end-to-end, against an in-memory SQLite ledger, with a trivial `kv_*`
//! toolset and one real policy (threshold approval on `kv_set` when the
//! payload string is "danger").
//!
//! Run:
//!     cargo run --example hello_triad -p thymos-runtime

use thymos_ledger::{EntryPayload, Ledger};
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, DelegationBounds, EffectCeiling, IntentBody,
    IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{KvGetTool, KvSetTool, ToolRegistry};

fn main() -> anyhow::Result<()> {
    // 1. Assemble the runtime: ledger, tool registry, policy engine.
    let ledger = Ledger::open_in_memory()?;

    let mut tools = ToolRegistry::new();
    tools.register(KvSetTool::default());
    tools.register(KvGetTool::default());

    let policy = PolicyEngine::new().with(WritAuthorityPolicy);

    let runtime = Runtime::new(ledger, tools, policy);

    // 2. Generate a root keypair and mint a signed Writ authorizing only `kv_*`.
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();

    let writ = Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "hello-agent".into(),
            subject_pubkey: public_key_of(&agent_key),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_*")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 10,
                wall_clock_ms: 60_000,
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
        &root_key,
    )?;
    writ.verify_signature()?;
    println!("== writ signed and verified: {}", writ.id);

    // 3. Start a trajectory.
    let run = runtime.create_run("hello_triad demo", b"hello-triad-v1")?;
    println!("== trajectory: {}", run.trajectory_id());

    // 4. Cognition emits Intent 1: set kv:foo = "bar".
    let intent1 = thymos_runtime::CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "cognition".into(),
        kind: IntentKind::Act,
        target: "kv_set".into(),
        args: serde_json::json!({ "key": "foo", "value": "bar" }),
        rationale: "initialize the canonical greeting".into(),
        nonce: [0; 16],
    })?;
    println!("\n-> submit Intent 1: kv_set(foo=bar)");
    let step1 = run.submit(intent1, &writ)?;
    println!("   result: {:?}", step1);
    assert!(matches!(step1, Step::Committed(_)));

    // 5. Cognition emits Intent 2: set kv:foo = "baz" (CAS Replace).
    let intent2 = thymos_runtime::CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "cognition".into(),
        kind: IntentKind::Act,
        target: "kv_set".into(),
        args: serde_json::json!({ "key": "foo", "value": "baz" }),
        rationale: "update greeting".into(),
        nonce: [1; 16],
    })?;
    println!("\n-> submit Intent 2: kv_set(foo=baz)");
    let step2 = run.submit(intent2, &writ)?;
    println!("   result: {:?}", step2);
    assert!(matches!(step2, Step::Committed(_)));

    // 6. Cognition emits Intent 3: call a tool the Writ does NOT authorize.
    let intent3 = thymos_runtime::CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "cognition".into(),
        kind: IntentKind::Act,
        target: "refund_order".into(),
        args: serde_json::json!({ "order_id": "42" }),
        rationale: "try to refund -- should be rejected at the writ boundary".into(),
        nonce: [2; 16],
    })?;
    println!("\n-> submit Intent 3: refund_order (UNAUTHORIZED)");
    let step3 = run.submit(intent3, &writ)?;
    println!("   result: {:?}", step3);
    assert!(matches!(step3, Step::Rejected(_)));

    // 7. Cognition emits Intent 4: kv_get(foo).
    let intent4 = thymos_runtime::CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "cognition".into(),
        kind: IntentKind::Act,
        target: "kv_get".into(),
        args: serde_json::json!({ "key": "foo" }),
        rationale: "observe current greeting".into(),
        nonce: [3; 16],
    })?;
    println!("\n-> submit Intent 4: kv_get(foo)");
    let step4 = run.submit(intent4, &writ)?;
    println!("   result: {:?}", step4);

    // 8. Dump trajectory summary.
    let summary = run.summary()?;
    println!("\n== trajectory summary");
    println!(
        "   total entries: {}  roots: {}  commits: {}  rejections: {}",
        summary.entries_total, summary.roots, summary.commits, summary.rejections
    );
    for (i, e) in summary.entries.iter().enumerate() {
        let label = match &e.payload {
            EntryPayload::Root { note, .. } => format!("Root({note})"),
            EntryPayload::Commit(c) => {
                let op = c
                    .body
                    .delta
                    .0
                    .first()
                    .map(|op| format!("{:?}", op))
                    .unwrap_or_else(|| "(empty)".into());
                format!("Commit seq={} delta={}", c.body.seq, op)
            }
            EntryPayload::Rejection { reason, .. } => format!("Rejection({:?})", reason),
            EntryPayload::PendingApproval {
                channel, reason, ..
            } => {
                format!("PendingApproval({channel}: {reason})")
            }
            EntryPayload::Delegation {
                child_trajectory_id,
                task,
                ..
            } => {
                format!("Delegation(child={child_trajectory_id}, task={task})")
            }
            EntryPayload::Branch {
                source_trajectory_id,
                source_commit_id,
                note,
            } => {
                format!("Branch(from={source_trajectory_id}@{source_commit_id}, {note})")
            }
        };
        println!("   [{i}] seq={} {} id={}", e.seq, label, e.id.short());
    }

    // 9. Final world projection.
    let world = run.project_world()?;
    println!("\n== world projection");
    for (k, v) in &world.resources {
        println!(
            "   {}:{} v{} = {}",
            k.kind,
            k.id,
            v.version,
            serde_json::to_string(&v.value)?
        );
    }

    println!("\nthymos: intent -> proposal -> commit. ledger remembers.");
    Ok(())
}
