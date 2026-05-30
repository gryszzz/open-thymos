//! Writ revocation: a validly-signed, unexpired writ can be pulled at runtime.
//! Subsequent submissions under it — or under any child whose immediate parent
//! is the revoked writ — are rejected as AuthorityVoid before the tool runs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use thymos_core::{crypto::SigningKey, ContentHash, WritId};
use thymos_core::{commit::Observation, delta::StructuredDelta, error::Result};
use thymos_ledger::Ledger;
use thymos_policy::{PolicyEngine, WritAuthorityPolicy};
use thymos_runtime::{
    generate_signing_key, public_key_of, Budget, CoreIntent, DelegationBounds, EffectCeiling,
    IntentBody, IntentKind, Runtime, Step, TimeWindow, ToolPattern, Writ, WritBody,
};
use thymos_tools::{
    EffectClass, RiskClass, ToolContract, ToolContractMeta, ToolInvocation, ToolOutcome,
    ToolRegistry,
};

struct NoopTool {
    counter: Arc<AtomicUsize>,
}
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
        self.counter.fetch_add(1, Ordering::SeqCst);
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

fn writ_with_parent(parent: Option<WritId>) -> Writ {
    let issuer = generate_signing_key();
    let subject = generate_signing_key();
    sign_writ(&issuer, &subject, parent)
}

fn sign_writ(issuer: &SigningKey, subject: &SigningKey, parent: Option<WritId>) -> Writ {
    Writ::sign(
        WritBody {
            issuer: "root".into(),
            issuer_pubkey: public_key_of(issuer),
            subject: "agent".into(),
            subject_pubkey: public_key_of(subject),
            parent,
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
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 1,
                may_subdivide: false,
            },
        },
        issuer,
    )
    .unwrap()
}

fn act(nonce: u8) -> CoreIntent {
    CoreIntent::new(IntentBody {
        parent_commit: None,
        author: "test".into(),
        kind: IntentKind::Act,
        target: "noop".into(),
        args: json!({}),
        rationale: "revocation".into(),
        nonce: [nonce; 16],
    })
    .unwrap()
}

fn runtime_with_noop() -> (Runtime, Arc<AtomicUsize>) {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(NoopTool {
        counter: Arc::clone(&counter),
    });
    let runtime = Runtime::new(
        Ledger::open_in_memory().unwrap(),
        tools,
        PolicyEngine::new().with(WritAuthorityPolicy),
    );
    (runtime, counter)
}

#[test]
fn revoked_writ_is_rejected_before_execution() {
    let (runtime, counter) = runtime_with_noop();
    let writ = writ_with_parent(None);
    let run = runtime.create_run("revoke", b"revoke").unwrap();

    // Works before revocation.
    assert!(matches!(
        run.submit(act(1), &writ).unwrap(),
        Step::Committed(_)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Pull the capability.
    runtime.revoke_writ(writ.id);

    // Now rejected — and the tool must not run again.
    match run.submit(act(2), &writ).unwrap() {
        Step::Rejected(reason) => assert!(
            reason.to_string().contains("revoked"),
            "expected a revocation reason, got: {reason}"
        ),
        other => panic!("expected rejection after revocation, got {other:?}"),
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1, "no execution under revoked writ");
}

#[test]
fn revoking_parent_voids_child_writ() {
    let (runtime, counter) = runtime_with_noop();
    let parent_id = WritId(ContentHash::ZERO);
    // A child writ that names `parent_id` as its parent.
    let child = writ_with_parent(Some(parent_id));
    let run = runtime.create_run("revoke-parent", b"revoke-parent").unwrap();

    assert!(matches!(
        run.submit(act(1), &child).unwrap(),
        Step::Committed(_)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Revoke the PARENT; the child must become void too.
    runtime.revoke_writ(parent_id);

    match run.submit(act(2), &child).unwrap() {
        Step::Rejected(reason) => assert!(reason.to_string().contains("revoked")),
        other => panic!("expected child rejection after parent revocation, got {other:?}"),
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn restore_reinstates_a_writ() {
    let (runtime, counter) = runtime_with_noop();
    let writ = writ_with_parent(None);
    let run = runtime.create_run("restore", b"restore").unwrap();

    runtime.revoke_writ(writ.id);
    assert!(matches!(
        run.submit(act(1), &writ).unwrap(),
        Step::Rejected(_)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 0);

    runtime.revocations.restore(&writ.id);
    assert!(matches!(
        run.submit(act(2), &writ).unwrap(),
        Step::Committed(_)
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}
