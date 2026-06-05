//! Intent → Proposal compiler.
//!
//! The compiler is pure and deterministic given `(Intent, Writ, World,
//! ToolRegistry, PolicyEngine)`. It produces either a `Proposal` (staged) or a
//! typed `RejectionReason` that the runtime will append to the ledger.
//!
//! Stages (order matters — authority precedes surface visibility):
//!
//! ```text
//! 1.  Kind gate          — Phase 1 only supports `Act` intents.
//! 2.  Signature check    — writ signature must verify.
//! 2b. Revocation check   — writ (or its parent) must not be revoked.
//! 3.  Time-window check  — now must lie within [not_before, expires_at].
//! 4.  Writ binding       — tool scope check (before tool surface is consulted).
//! 5.  Tool resolution    — lookup in the ToolRegistry; unknown -> UnknownTool.
//! 5b. Effect ceiling     — tool effect class must be granted by the writ.
//! 6.  Budget check       — estimated cost vs remaining writ budget.
//! 7.  Type check         — delegate to ToolContract::validate_args.
//! 8.  Precondition       — contract-declared; evaluated against World.
//! 9.  Policy eval        — run the PolicyEngine over (Intent, Writ, World).
//! 9b. Compensation gate  — optionally require approval for an irreversible,
//!                          non-compensable tool.
//! 10. Emit Proposal      — with full PolicyTrace, or a typed rejection.
//! ```

use std::collections::HashSet;

use thymos_core::{
    error::{Error, Result},
    ids::WritId,
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
use thymos_tools::{EffectClass, ToolInvocation, ToolRegistry};

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
///
/// Constructed explicitly by callers — there is intentionally no `Default`
/// impl, because both fields name external state (the wall clock and the
/// trajectory's accumulated budget). Spec Section 3 + the project's purity
/// rule require that the compiler be a pure function of its inputs; reading
/// `SystemTime::now()` inside `Default::default()` would smuggle the clock
/// into the pure path and make the compiler non-deterministic when invoked
/// via `..CompileContext::default()`.
pub struct CompileContext {
    /// Current unix timestamp in seconds (for time-window validation).
    /// Callers are responsible for sourcing this from their own clock.
    pub now_unix: u64,
    /// Accumulated budget usage so far in this trajectory. The compiler
    /// checks that `accumulated + estimate <= writ.budget`.
    pub budget_used: BudgetCost,
    /// Revoked writ ids. A writ whose id (or whose immediate parent id) appears
    /// here is treated as void even if its signature and time window are valid —
    /// the revocation mechanism for capabilities pulled before expiry.
    pub revoked: HashSet<WritId>,
    /// When true, an `Irreversible` tool that is **not** compensable
    /// (`ToolContract::compensable() == false`) is escalated to `Suspended`
    /// (require approval) even if policy permitted — it cannot be undone and the
    /// runtime cannot roll it back, so a human must sign off. Default `false`
    /// (preserves prior behavior); enable via
    /// `Runtime::with_require_compensation_for_irreversible`.
    pub require_compensation_for_irreversible: bool,
}

impl CompileContext {
    /// A deterministic context with `now_unix = 0`, empty budget, and no
    /// revocations — use in tests or when the writ has an unbounded time window.
    pub fn deterministic() -> Self {
        CompileContext {
            now_unix: 0,
            budget_used: BudgetCost::default(),
            revoked: HashSet::new(),
            require_compensation_for_irreversible: false,
        }
    }
}

/// Compile with a deterministic context (`now_unix = 0`, empty budget).
/// Production callers should use [`compile_with_context`] and pass an
/// explicit clock + budget.
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
        &CompileContext::deterministic(),
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

    // 2b. Revocation check. A validly-signed writ can still be void if it (or
    // its immediate parent) has been revoked. Checked before the time window so
    // a revoked-but-unexpired writ is rejected as soon as possible.
    if ctx.revoked.contains(&writ.id) {
        return Ok(Compiled::Rejected(RejectionReason::AuthorityVoid(format!(
            "writ {} has been revoked",
            writ.id
        ))));
    }
    if let Some(parent) = writ.body.parent {
        if ctx.revoked.contains(&parent) {
            return Ok(Compiled::Rejected(RejectionReason::AuthorityVoid(format!(
                "writ {} is void: parent {} has been revoked",
                writ.id, parent
            ))));
        }
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

    // 5b. Effect-ceiling check — the tool's declared effect class must be
    // granted by the writ's effect ceiling. Tool scopes authorize a tool *by
    // name*; the ceiling authorizes the *kind of effect* it may have. Without
    // this gate a read-only writ could still drive an External or Irreversible
    // tool merely because the tool name matched a scope pattern. Authority must
    // precede execution, so this runs before budget and type checks.
    let effect_class = tool.meta().effect_class;
    if !effect_within_ceiling(effect_class, &writ.body.effect_ceiling) {
        return Ok(Compiled::Rejected(RejectionReason::AuthorityVoid(format!(
            "writ effect ceiling does not grant {:?} effect required by tool '{}'",
            effect_class, effective_tool
        ))));
    }

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
            ProposalStatus::Suspended {
                channel: channel.clone(),
                reason: reason.clone(),
            },
            Some((channel.clone(), reason.clone())),
        ),
    };

    // 9b. Compensation gate. If enabled, an Irreversible tool that cannot be
    // compensated (no rollback path) must not proceed on a bare permit — it is
    // escalated to require approval, so a human signs off on an effect neither
    // the tool nor the runtime can undo.
    let (status, suspension) = if ctx.require_compensation_for_irreversible
        && suspension.is_none()
        && effect_class == EffectClass::Irreversible
        && !tool.compensable()
    {
        let channel = "irreversible-uncompensable".to_string();
        let reason = format!(
            "tool '{}' has an irreversible effect and is not compensable; approval required",
            effective_tool
        );
        (
            ProposalStatus::Suspended {
                channel: channel.clone(),
                reason: reason.clone(),
            },
            Some((channel, reason)),
        )
    } else {
        (status, suspension)
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

/// Returns true if a writ whose `ceiling` is granted may drive a tool with the
/// given `class`. Each effect class maps to the single ceiling bit it requires;
/// `Pure` requires nothing. This mirrors the per-dimension subset check in
/// `WritBody::verify_subset_of`, so an effect a parent writ could not delegate
/// is also one the runtime will not execute.
fn effect_within_ceiling(
    class: EffectClass,
    ceiling: &thymos_core::writ::EffectCeiling,
) -> bool {
    match class {
        EffectClass::Pure => true,
        EffectClass::Read => ceiling.read,
        EffectClass::Write => ceiling.write,
        EffectClass::External => ceiling.external,
        EffectClass::Irreversible => ceiling.irreversible,
    }
}
