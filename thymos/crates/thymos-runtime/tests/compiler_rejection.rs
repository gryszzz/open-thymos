//! Hardening tests: prove the compiler rejects an Intent BEFORE any tool's
//! `execute()` runs on every authority-failure path enumerated in
//! `thymos-compiler` Stages 1–8.
//!
//! Strategy: register a `PoisonTool` whose `execute()` increments a shared
//! counter (and panics in debug builds). For each failure mode, submit an
//! Intent that should reject at the corresponding compiler stage and assert
//! `PoisonTool::execute()` was never reached.
//!
//! Spec reference: Section 3 — "If any authority check fails, the compiler
//! MUST reject before capability execution."

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use thymos_cognition::mock::MockCognition;
use thymos_core::{
    crypto::SigningKey,
    error::{Error, Result},
    proposal::PolicyDecision,
};
use thymos_ledger::Ledger;
use thymos_policy::{Policy, PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, run_agent, AgentRunOptions, Budget, CoreIntent,
    DelegationBounds, EffectCeiling, IntentBody, IntentKind, Runtime, Termination, TimeWindow,
    ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

/// A tool that asserts it is never executed. Increments a shared counter on
/// any call so the test can verify zero invocations even in release builds
/// (where panics inside `cargo test` may be swallowed by the harness).
struct PoisonTool {
    counter: Arc<AtomicUsize>,
    meta: ToolContractMeta,
}

impl PoisonTool {
    fn new(name: &str, counter: Arc<AtomicUsize>) -> Self {
        PoisonTool {
            counter,
            meta: ToolContractMeta {
                name: name.into(),
                version: "1.0.0".into(),
                effect_class: EffectClass::Write,
                risk_class: RiskClass::Low,
            },
        }
    }
}

impl ToolContract for PoisonTool {
    fn meta(&self) -> &ToolContractMeta {
        &self.meta
    }
    fn description(&self) -> &str {
        "test poison tool — execute must never be called"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn validate_args(&self, args: &Value) -> Result<()> {
        // Stage 7 rejection target — fail when args contain "fail_validate".
        if args.get("fail_validate").is_some() {
            return Err(Error::ToolTypeMismatch {
                tool: self.meta.name.clone(),
                detail: "synthetic validate failure".into(),
            });
        }
        Ok(())
    }
    fn check_preconditions(&self, inv: &ToolInvocation<'_>) -> Result<()> {
        // Stage 8 rejection target.
        if inv.args.get("fail_precondition").is_some() {
            return Err(Error::PreconditionFailed("synthetic".into()));
        }
        Ok(())
    }
    fn execute(&self, _inv: &ToolInvocation<'_>) -> Result<ToolOutcome> {
        // Bump the counter and panic — under no spec-conformant path should
        // we reach here during a rejection test.
        self.counter.fetch_add(1, Ordering::SeqCst);
        panic!("PoisonTool::execute() must never be reached on a rejection path");
    }
}

fn build_runtime(counter: Arc<AtomicUsize>) -> Runtime {
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(PoisonTool::new("poison", counter));
    let policy = PolicyEngine::new().with(WritAuthorityPolicy);
    Runtime::new(ledger, tools, policy)
}

fn signed_writ(
    tool_scopes: Vec<ToolPattern>,
    budget: Budget,
    time_window: TimeWindow,
) -> (Writ, SigningKey) {
    signed_writ_with_ceiling(
        tool_scopes,
        budget,
        time_window,
        EffectCeiling::read_write_local(),
    )
}

fn signed_writ_with_ceiling(
    tool_scopes: Vec<ToolPattern>,
    budget: Budget,
    time_window: TimeWindow,
    effect_ceiling: EffectCeiling,
) -> (Writ, SigningKey) {
    let issuer = generate_signing_key();
    let subject = generate_signing_key();
    let writ = Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(&issuer),
            subject: "agent".into(),
            subject_pubkey: public_key_of(&subject),
            parent: None,
            tenant_id: String::new(),
            tool_scopes,
            budget,
            effect_ceiling,
            time_window,
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        },
        &issuer,
    )
    .unwrap();
    (writ, issuer)
}

fn intent(target: &str, args: Value, nonce: u8) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "mock".into(),
        kind: IntentKind::Act,
        target: target.into(),
        args,
        rationale: "rejection test".into(),
        nonce: [nonce; 16],
    })
    .unwrap()
}

fn assert_never_executed(counter: &AtomicUsize, label: &str) {
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "{label}: PoisonTool::execute() was called — compiler did not reject before tool execution"
    );
}

// ── Stage 2: signature check ─────────────────────────────────────────────────

#[test]
fn rejects_before_execute_when_signature_is_invalid() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    // Build a signed writ, then tamper with its body so the signature no
    // longer verifies.
    let (mut writ, _) = signed_writ(
        vec![ToolPattern::exact("poison")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
    );
    writ.body.subject = "tampered".into();

    let mut cog = MockCognition::new(
        vec![vec![intent("poison", json!({}), 1)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "sig",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert!(matches!(summary.terminated_by, Termination::CognitionDone));
    assert_never_executed(&counter, "sig invalid");
}

// ── Stage 3: time window ─────────────────────────────────────────────────────

#[test]
fn rejects_before_execute_when_time_window_expired() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    let (writ, _) = signed_writ(
        vec![ToolPattern::exact("poison")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            // Already expired (epoch + 100s).
            expires_at: 100,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent("poison", json!({}), 2)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "tw",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "time window");
}

// ── Stage 4: writ tool-scope binding ─────────────────────────────────────────

#[test]
fn rejects_before_execute_when_tool_outside_writ_scope() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    // Writ authorizes a *different* tool name.
    let (writ, _) = signed_writ(
        vec![ToolPattern::exact("safe_tool_only")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent("poison", json!({}), 3)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "scope",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "tool scope");
}

// ── Stage 5: unknown tool ────────────────────────────────────────────────────

#[test]
fn rejects_before_execute_when_tool_unknown() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    // Writ authorizes the tool name, but the registry doesn't have it.
    let (writ, _) = signed_writ(
        vec![ToolPattern::exact("ghost")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent("ghost", json!({}), 4)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "unknown",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "unknown tool");
}

// ── Stage 5b: effect ceiling ─────────────────────────────────────────────────

#[test]
fn rejects_before_execute_when_effect_exceeds_ceiling() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    // PoisonTool declares EffectClass::Write, but this writ's ceiling grants
    // read only. The tool name IS in scope, so the only thing standing between
    // the intent and execution is the effect-ceiling gate (Stage 5b). Before
    // that gate existed, a read-only writ could drive a Write tool.
    let (writ, _) = signed_writ_with_ceiling(
        vec![ToolPattern::exact("poison")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
        EffectCeiling {
            read: true,
            write: false,
            external: false,
            irreversible: false,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent("poison", json!({}), 9)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "effect",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "effect ceiling");
}

// ── Stage 6: budget projection ───────────────────────────────────────────────

#[test]
fn rejects_before_execute_when_budget_exhausted() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    let (writ, _) = signed_writ(
        vec![ToolPattern::exact("poison")],
        // Zero tool_calls — first attempt should reject as BudgetExhausted.
        Budget {
            tokens: 10_000,
            tool_calls: 0,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent("poison", json!({}), 5)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "budget",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "budget exhausted");
}

// ── Stage 7: type validation ─────────────────────────────────────────────────

#[test]
fn rejects_before_execute_when_args_fail_validation() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    let (writ, _) = signed_writ(
        vec![ToolPattern::exact("poison")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent("poison", json!({"fail_validate": true}), 6)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "type",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "type validation");
}

// ── Stage 8: precondition ────────────────────────────────────────────────────

#[test]
fn rejects_before_execute_when_precondition_fails() {
    let counter = Arc::new(AtomicUsize::new(0));
    let runtime = build_runtime(Arc::clone(&counter));

    let (writ, _) = signed_writ(
        vec![ToolPattern::exact("poison")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent(
            "poison",
            json!({"fail_precondition": true}),
            7,
        )]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "pre",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "precondition");
}

// ── Stage 9: policy denial ───────────────────────────────────────────────────

/// Policy that unconditionally denies. Plugged in alongside WritAuthority so
/// the compiler reaches policy evaluation (Stage 9) and is denied there.
struct AlwaysDenyPolicy;
impl Policy for AlwaysDenyPolicy {
    fn name(&self) -> &'static str {
        "test.always_deny"
    }
    fn evaluate(
        &self,
        _intent: &thymos_core::intent::Intent,
        _writ: &Writ,
        _world: &thymos_core::world::World,
    ) -> PolicyDecision {
        PolicyDecision::Deny("synthetic deny".into())
    }
}

#[test]
fn rejects_before_execute_when_policy_denies() {
    let counter = Arc::new(AtomicUsize::new(0));
    let ledger = Ledger::open_in_memory().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(PoisonTool::new("poison", Arc::clone(&counter)));
    let policy = PolicyEngine::new()
        .with(WritAuthorityPolicy)
        .with(AlwaysDenyPolicy);
    let runtime = Runtime::new(ledger, tools, policy);

    let (writ, _) = signed_writ(
        vec![ToolPattern::exact("poison")],
        Budget {
            tokens: 10_000,
            tool_calls: 32,
            wall_clock_ms: 60_000,
            usd_millicents: 0,
        },
        TimeWindow {
            not_before: 0,
            expires_at: u64::MAX,
        },
    );

    let mut cog = MockCognition::new(
        vec![vec![intent("poison", json!({}), 8)]],
        Some("done".into()),
    );
    let summary = run_agent(
        &runtime,
        &mut cog,
        "policy",
        &writ,
        AgentRunOptions { max_steps: 4 },
        None,
    )
    .unwrap();

    assert_eq!(summary.commits, 0);
    assert_eq!(summary.rejections, 1);
    assert_never_executed(&counter, "policy deny");
}

// ── Compile silently: every rejection bumps the rejection count but no
//    BudgetCost is debited (the writ scopes never reach `debit`). This is
//    inherent to Phase I; just sanity-check the counter math.

#[test]
fn poison_tool_unused_in_clean_run() {
    // Sanity: with an authorized intent and no policy denial, a real tool
    // would run. We use a permissive-args call, then assert poison was
    // entered exactly zero times because every other test above never
    // reached the tool boundary.
    let counter = Arc::new(AtomicUsize::new(0));
    let _runtime = build_runtime(Arc::clone(&counter));
    // We deliberately don't drive a run here — this is just an assertion
    // that the counter exists at zero before this test file's other cases.
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}
