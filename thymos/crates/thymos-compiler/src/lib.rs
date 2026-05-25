//! Intent → Proposal compiler.
//!
//! The compiler is pure and deterministic given `(Intent, Writ, World,
//! ToolRegistry, PolicyEngine)`. It produces either a `Proposal` (staged) or a
//! typed `RejectionReason` that the runtime will append to the ledger.
//!
//! Stages (order matters — authority precedes surface visibility):
//!   1. Kind gate         — Phase 1 only supports `Act` intents.
//!   2. Signature check   — writ signature must verify.
//!   3. Time-window check — now must lie within [not_before, expires_at].
//!   4. Writ binding      — tool scope check (before tool surface is consulted).
//!   5. Tool resolution   — lookup in the ToolRegistry; unknown -> UnknownTool.
//!   6. Budget check      — estimated cost vs remaining writ budget.
//!   7. Type check        — delegate to ToolContract::validate_args.
//!   8. Precondition      — contract-declared; evaluated against World.
//!   9. Policy eval       — run the PolicyEngine over (Intent, Writ, World).
//!  10. Emit Proposal     — with full PolicyTrace, or a typed rejection.

use thymos_core::{
    error::{Error, Result},
    intent::{Intent, IntentKind},
    proposal::{
        ExecutionPlan, PolicyDecision, PolicyTrace, Proposal, ProposalBody, ProposalStatus,
        RejectionReason,
    },
    world::World,
    writ::BudgetCost,
    writ::Writ,
};
use thymos_policy::PolicyEngine;
use thymos_tools::{ToolInvocation, ToolRegistry};

pub enum Compiled {
    Staged(Proposal),
    Suspended {
        proposal: Proposal,
        channel: String,
        reason: String,
    },
    Rejected(RejectionReason),
}

/// Additional context the compiler needs beyond (Intent, Writ, World).
pub struct CompileContext {
    /// Current unix timestamp in seconds (for time-window validation).
    pub now_unix: u64,
    /// Accumulated budget usage so far in this trajectory. The compiler
    /// checks that `accumulated + estimate <= writ.budget`.
    pub budget_used: BudgetCost,
}

impl Default for CompileContext {
    fn default() -> Self {
        CompileContext {
            now_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            budget_used: BudgetCost::default(),
        }
    }
}

pub fn compile(
    intent: &Intent,
    writ: &Writ,
    world: &World,
    tools: &ToolRegistry,
    policy: &PolicyEngine,
) -> Result<Compiled> {
    compile_with_context(
        intent,
        writ,
        world,
        tools,
        policy,
        &CompileContext::default(),
    )
}

pub fn compile_with_context(
    intent: &Intent,
    writ: &Writ,
    world: &World,
    tools: &ToolRegistry,
    policy: &PolicyEngine,
    ctx: &CompileContext,
) -> Result<Compiled> {
    // 1. Kind gate.
    match intent.body.kind {
        IntentKind::Act | IntentKind::MemoryPromote | IntentKind::Delegate => {}
        _ => {
            return Ok(Compiled::Rejected(RejectionReason::TypeMismatch {
                tool: intent.body.target.clone(),
                detail: format!("intent kind {:?} is not yet supported", intent.body.kind),
            }));
        }
    }

    // For MemoryPromote intents, the target is the memory key and the tool
    // is always `memory_store`. For Delegate, it's `delegate` (a synthetic
    // tool the runtime intercepts). Translate so the rest of the pipeline
    // sees a tool name.
    let effective_tool = match intent.body.kind {
        IntentKind::MemoryPromote => "memory_store".to_string(),
        IntentKind::Delegate => "delegate".to_string(),
        _ => intent.body.target.clone(),
    };

    // 2. Signature check.
    if let Err(e) = writ.verify_signature() {
        return Ok(Compiled::Rejected(RejectionReason::AuthorityVoid(format!(
            "writ signature invalid: {e}"
        ))));
    }

    // 3. Time-window check.
    if !writ.body.time_window.contains(ctx.now_unix) {
        return Ok(Compiled::Rejected(RejectionReason::AuthorityVoid(format!(
            "writ time window [{}, {}] does not contain now={}",
            writ.body.time_window.not_before, writ.body.time_window.expires_at, ctx.now_unix
        ))));
    }

    // 4. Writ binding (tool scope).
    if !writ.authorizes_tool(&effective_tool) {
        return Ok(Compiled::Rejected(RejectionReason::AuthorityVoid(format!(
            "writ does not authorize tool '{}'",
            effective_tool
        ))));
    }

    // 5. Tool resolution.
    let tool = match tools.get(&effective_tool) {
        Ok(t) => t,
        Err(Error::UnknownTool(name)) => {
            return Ok(Compiled::Rejected(RejectionReason::UnknownTool(name)));
        }
        Err(e) => return Err(e),
    };

    // 6. Budget check — estimated cost for this call + accumulated usage.
    let estimate = tool.estimate_cost(&intent.body.args);
    let projected = ctx.budget_used.saturating_add(&estimate);
    if writ.check_budget(&projected).is_err() {
        return Ok(Compiled::Rejected(RejectionReason::BudgetExhausted(
            format!(
                "projected cost ({} tool_calls, {} tokens) exceeds writ budget",
                projected.tool_calls, projected.tokens
            ),
        )));
    }

    // 7. Type check.
    if let Err(Error::ToolTypeMismatch { tool: t, detail }) = tool.validate_args(&intent.body.args)
    {
        return Ok(Compiled::Rejected(RejectionReason::TypeMismatch {
            tool: t,
            detail,
        }));
    }

    // 8. Preconditions.
    if let Err(e) = tool.check_preconditions(&ToolInvocation {
        args: &intent.body.args,
        world,
    }) {
        return Ok(Compiled::Rejected(RejectionReason::PreconditionFailed(
            e.to_string(),
        )));
    }

    // 9. Policy.
    let policy_trace: PolicyTrace = policy.evaluate(intent, writ, world);

    let (status, suspension) = match &policy_trace.decision {
        PolicyDecision::Permit => (ProposalStatus::Staged, None),
        PolicyDecision::Deny(reason) => {
            return Ok(Compiled::Rejected(RejectionReason::PolicyDenied(
                reason.clone(),
            )));
        }
        PolicyDecision::RequireApproval { channel, reason } => (
            ProposalStatus::SuspendedForApproval,
            Some((channel.clone(), reason.clone())),
        ),
    };

    // 10. Emit Proposal.
    let body = ProposalBody {
        intent_id: intent.id,
        writ_id: writ.id,
        plan: ExecutionPlan {
            tool: effective_tool,
            args: intent.body.args.clone(),
        },
        policy_trace,
        status,
    };
    let proposal = Proposal::new(body)?;

    Ok(match suspension {
        None => Compiled::Staged(proposal),
        Some((channel, reason)) => Compiled::Suspended {
            proposal,
            channel,
            reason,
        },
    })
}
