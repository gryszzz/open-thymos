//! Live-provider integration test.
//!
//! Everything else in the runtime suite drives the Triad with `MockCognition`
//! so it is deterministic and network-free. That proves the *wiring* is sound,
//! but a skeptic can fairly ask: does the full loop actually close against a
//! real model? This test answers that — it drives
//!
//!     real Anthropic model -> submit -> compile -> govern -> execute -> ledger
//!
//! end to end and asserts that a real commit landed *and* mutated the world.
//!
//! It is gated two ways so it never breaks CI:
//!   * `#[ignore]` — excluded from the default `cargo test`.
//!   * an explicit `ANTHROPIC_API_KEY` check — if the key is absent (e.g. someone
//!     runs `--ignored` on a machine without one) the test prints a SKIP notice
//!     and returns rather than failing.
//!
//! Run it for real with:
//!
//!     ANTHROPIC_API_KEY=sk-... cargo test -p thymos-runtime --test live_provider -- --ignored --nocapture

use thymos_cognition::anthropic::AnthropicCognition;
use thymos_ledger::Ledger;
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, run_agent, AgentRunOptions, Budget, DelegationBounds,
    EffectCeiling, ResourceKey, Runtime, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{KvGetTool, KvSetTool, ToolRegistry};

fn build_runtime() -> Runtime {
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(KvSetTool::default());
    tools.register(KvGetTool::default());
    let policy = PolicyEngine::new().with(WritAuthorityPolicy);
    Runtime::new(ledger, tools, policy)
}

/// A root writ scoped to the kv tools, with budgets generous enough that a real
/// model round-trip (which spends real tokens / USD) does not trip the
/// capability bound before the intent is submitted.
fn root_writ() -> Writ {
    let root_key = generate_signing_key();
    let agent_key = generate_signing_key();
    Writ::sign(
        WritBody {
            issuer: "audit-root".into(),
            issuer_pubkey: public_key_of(&root_key),
            subject: "live-agent".into(),
            subject_pubkey: public_key_of(&agent_key),
            nonce: [7u8; 16],
            parent: None,
            tenant_id: String::new(),
            tool_scopes: vec![ToolPattern::exact("kv_*")],
            budget: Budget {
                tokens: 1_000_000,
                tool_calls: 16,
                wall_clock_ms: 180_000,
                // USD ceiling well above a single small completion. Without a
                // non-zero ceiling the loop would stop the instant the model
                // reports any real spend.
                usd_millicents: 100_000_000,
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
    )
    .expect("sign writ")
}

#[test]
#[ignore = "live LLM integration — set ANTHROPIC_API_KEY and run with --ignored"]
fn live_anthropic_closes_the_loop_and_commits() {
    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        eprintln!(
            "SKIP live_anthropic_closes_the_loop_and_commits: ANTHROPIC_API_KEY not set. \
             Export a key to run this test for real."
        );
        return;
    }

    let runtime = build_runtime();
    let writ = root_writ();
    let mut cognition = AnthropicCognition::from_env().expect("build live Anthropic cognition");

    // Deterministic target so the assertion is exact. The model is told the
    // precise key and value to write, then to finish.
    let task = "Use the kv_set tool to set the key `audit_key` to the string value \
                `live-proof` (exactly, no quotes in the stored value). Make exactly one \
                kv_set call, then you are done. Do not call any other tool.";

    let summary = run_agent(
        &runtime,
        &mut cognition,
        task,
        &writ,
        AgentRunOptions { max_steps: 6 },
        None,
    )
    .expect("live agent run");

    eprintln!(
        "live run: trajectory={} steps={} intents={} commits={} rejections={} failures={} terminated_by={:?} final_answer={:?}",
        summary.trajectory_id,
        summary.steps_executed,
        summary.intents_submitted,
        summary.commits,
        summary.rejections,
        summary.failures,
        summary.terminated_by,
        summary.final_answer,
    );

    // 1. The model produced at least one governed action that the runtime
    //    actually committed (not rejected, not failed).
    assert!(
        summary.commits >= 1,
        "expected at least one real commit from the live model; summary={summary:?}"
    );

    // 2. Re-fold the committed ledger for that trajectory and prove the world
    //    actually carries the mutation. resume_run + project_world re-reads and
    //    re-validates the hash-chained entries, so this also exercises replay.
    let run = runtime
        .resume_run(summary.trajectory_id)
        .expect("resume committed trajectory");
    let world = run.project_world().expect("project world from ledger");

    let state = world
        .get(&ResourceKey::new("kv", "audit_key"))
        .expect("kv:audit_key must exist in the projected world after a live commit");

    assert_eq!(
        state.value,
        serde_json::json!("live-proof"),
        "the committed value must match what the live model was instructed to write"
    );

    eprintln!(
        "PROOF: live model -> governed commit -> ledger -> world projection holds \
         (kv:audit_key = {})",
        state.value
    );
}
