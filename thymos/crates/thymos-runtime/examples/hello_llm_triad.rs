//! `hello_llm_triad` — same IPC Triad, now driven by a real LLM.
//!
//! The Thymos invariant is that the runtime is the sole source of truth. The
//! model is confined to producing `tool_use` blocks, which the
//! `AnthropicCognition` adapter translates into Intents. Every Intent still
//! goes through compile → policy → tool → commit. The ledger still remembers
//! every proposal, every commit, and every typed rejection.
//!
//! If `ANTHROPIC_API_KEY` is not set, the example falls back to a scripted
//! `MockCognition` so the demo remains runnable offline.
//!
//! Run:
//!     ANTHROPIC_API_KEY=sk-ant-... cargo run --example hello_llm_triad -p thymos-runtime
//! or (offline mock):
//!     cargo run --example hello_llm_triad -p thymos-runtime

use thymos_cognition::{anthropic::AnthropicCognition, mock::MockCognition, Cognition};
use thymos_ledger::{EntryPayload, Ledger};
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, run_agent, AgentRunOptions, Budget, DelegationBounds,
    EffectCeiling, IntentBody, IntentKind, Runtime, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{KvGetTool, KvSetTool, ToolRegistry};

fn main() -> anyhow::Result<()> {
    // 1. Assemble the runtime identically to hello_triad.
    let ledger = Ledger::open_in_memory()?;

    let mut tools = ToolRegistry::new();
    tools.register(KvSetTool::default());
    tools.register(KvGetTool::default());

    let policy = PolicyEngine::new().with(WritAuthorityPolicy);

    let runtime = Runtime::new(ledger, tools, policy);

    // 2. Generate keypair and mint a signed Writ authorizing only kv_*.
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();

    let writ = Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "llm-agent".into(),
            subject_pubkey: public_key_of(&agent_key),
            nonce: [0u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_*")],
            budget: Budget {
                tokens: 50_000,
                tool_calls: 16,
                wall_clock_ms: 120_000,
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

    // 3. Pick a cognition. Real LLM if the key is present; mock otherwise.
    let task = "Record the greeting 'hello thymos' under the key 'greeting', \
                then read it back to confirm. Report the final observed value.";

    let mut cognition: Box<dyn Cognition> = match AnthropicCognition::from_env() {
        Ok(c) => {
            eprintln!("== cognition: AnthropicCognition (live)");
            Box::new(c)
        }
        Err(_) => {
            eprintln!("== cognition: MockCognition (ANTHROPIC_API_KEY not set; running offline)");
            Box::new(scripted_mock())
        }
    };

    // 4. Run the agent loop. Runtime remains authoritative.
    let summary = run_agent(
        &runtime,
        cognition.as_mut(),
        task,
        &writ,
        AgentRunOptions { max_steps: 8 },
        None,
    )?;

    // 5. Print the run summary.
    println!("\n== agent run summary");
    println!("   trajectory    : {}", summary.trajectory_id);
    println!("   steps         : {}", summary.steps_executed);
    println!("   intents       : {}", summary.intents_submitted);
    println!("   commits       : {}", summary.commits);
    println!("   rejections    : {}", summary.rejections);
    println!("   terminated_by : {:?}", summary.terminated_by);
    if let Some(ans) = &summary.final_answer {
        println!("   final_answer  : {}", ans);
    }

    // 6. Dump the ledger.
    let run_again = scan_trajectory(&runtime, summary.trajectory_id)?;
    println!("\n== ledger ({} entries)", run_again.len());
    for (i, e) in run_again.iter().enumerate() {
        let label = match &e.payload {
            EntryPayload::Root { note } => format!("Root({note})"),
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

    println!("\nthymos: cognition proposes. runtime decides. ledger remembers.");
    Ok(())
}

/// Scripted cognition used when running without an API key.
fn scripted_mock() -> MockCognition {
    let set_intent = thymos_runtime::CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "mock".into(),
        kind: IntentKind::Act,
        target: "kv_set".into(),
        args: serde_json::json!({ "key": "greeting", "value": "hello thymos" }),
        rationale: "record the greeting".into(),
        nonce: [0xA1; 16],
    })
    .expect("build set intent");

    let get_intent = thymos_runtime::CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "mock".into(),
        kind: IntentKind::Act,
        target: "kv_get".into(),
        args: serde_json::json!({ "key": "greeting" }),
        rationale: "read it back to confirm".into(),
        nonce: [0xA2; 16],
    })
    .expect("build get intent");

    MockCognition::new(
        vec![vec![set_intent], vec![get_intent]],
        Some("observed value: \"hello thymos\"".into()),
    )
}

fn scan_trajectory(
    runtime: &Runtime,
    trajectory_id: thymos_core::TrajectoryId,
) -> anyhow::Result<Vec<thymos_ledger::Entry>> {
    Ok(runtime.ledger.entries(trajectory_id)?)
}
