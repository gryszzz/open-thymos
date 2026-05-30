//! Thymos runtime orchestration.
//!
//! Wires the Cognition Gateway, Compiler, Tool Gateway, Policy Engine, and
//! Ledger into the IPC (Intent → Proposal → Commit) cycle.
//!
//! Phase 1 is synchronous and single-agent. The runtime owns a fresh `World`
//! projection and rebuilds it from the ledger on `Run::resume`.

use thymos_compiler::{compile_with_context, CompileContext, Compiled};
use thymos_core::{
    commit::{Commit, CommitBody},
    error::{Error, Result},
    intent::Intent,
    proposal::RejectionReason,
    world::World,
    writ::BudgetCost,
    CommitId, TrajectoryId, COMPILER_VERSION,
};
use thymos_ledger::{project_commits, EntryPayload, Ledger};
use thymos_policy::PolicyEngine;
use thymos_tools::{EffectClass, ToolInvocation, ToolRegistry};

pub mod agent;
pub use agent::{
    run_agent, AgentEventCallback, AgentRunOptions, AgentRunSummary, AgentTraceEvent, Termination,
};

#[cfg(feature = "async")]
pub mod agent_async;
#[cfg(feature = "async")]
pub use agent_async::run_agent_streaming;

/// Registry of subject signing keys, indexed by their corresponding
/// `subject_pubkey`. The runtime consults this when a parent writ delegates:
/// the parent's signing key (held under `parent_writ.body.subject_pubkey`)
/// is required to mint a properly signed child writ.
///
/// Delegation works without a keyring (the runtime falls back to the
/// pre-signing behavior, recording an unsigned delegation edge), but the
/// child cannot in turn delegate further unless its key is here.
#[derive(Clone, Default)]
pub struct DelegationKeyring {
    inner: std::sync::Arc<
        std::sync::RwLock<
            std::collections::HashMap<
                thymos_core::crypto::PublicKey,
                thymos_core::crypto::SigningKey,
            >,
        >,
    >,
    /// Signed child writs awaiting pickup by the agent loop, keyed by the
    /// child trajectory id. When the loop is ready to drive a delegated
    /// child run, it calls `take_pending_child_writ` to retrieve and consume.
    pending_writs: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<thymos_core::TrajectoryId, thymos_core::writ::Writ>,
        >,
    >,
}

impl DelegationKeyring {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `signing_key` for the public key it derives. Subsequent
    /// delegations whose parent writ has this `subject_pubkey` will produce
    /// signed child writs.
    pub fn register(&self, signing_key: thymos_core::crypto::SigningKey) {
        let pk = thymos_core::crypto::public_key_of(&signing_key);
        let mut g = self.inner.write().unwrap();
        g.insert(pk, signing_key);
    }

    /// Clone the signing key registered for `pubkey`, if any.
    pub fn get(
        &self,
        pubkey: &thymos_core::crypto::PublicKey,
    ) -> Option<thymos_core::crypto::SigningKey> {
        self.inner.read().unwrap().get(pubkey).cloned()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Internal: stash a freshly-signed child writ for pickup by trajectory id.
    pub(crate) fn stash_writ(
        &self,
        trajectory_id: thymos_core::TrajectoryId,
        writ: thymos_core::writ::Writ,
    ) {
        self.pending_writs
            .lock()
            .unwrap()
            .insert(trajectory_id, writ);
    }

    /// Take the signed child writ for `trajectory_id`, if one was stashed by
    /// a delegation. Removes it from the keyring.
    pub fn take_pending_child_writ(
        &self,
        trajectory_id: thymos_core::TrajectoryId,
    ) -> Option<thymos_core::writ::Writ> {
        self.pending_writs.lock().unwrap().remove(&trajectory_id)
    }
}

pub struct Runtime {
    pub ledger: Ledger,
    pub tools: ToolRegistry,
    pub policy: PolicyEngine,
    pub delegation_keyring: Option<DelegationKeyring>,
    /// Optional runtime identity that signs every commit it appends. When set,
    /// commits are written via `Commit::new_signed`; replay can then require
    /// each commit verify against the corresponding public key
    /// (`ReplayConfig::require_commit_signatures`). When `None`, commits are
    /// unsigned and tamper-evidence rests on the hash chain alone.
    pub commit_signer: Option<thymos_core::crypto::SigningKey>,
    /// Redacts secrets from tool observations before they are persisted in the
    /// (append-only, undeletable) ledger. Defaults to
    /// [`Redactor::default_secrets`]; replace via [`Runtime::with_redactor`] or
    /// disable with `Redactor::none()`.
    pub redactor: thymos_core::Redactor,
    /// Revoked writs. Consulted on every compile so a capability can be pulled
    /// before its time window expires. Shared (cheap to clone) so a control
    /// surface can revoke while runs are in flight.
    pub revocations: Revocations,
}

/// Thread-safe set of revoked writ ids, consulted by the compiler on every
/// submission. Revoking a writ also voids any child whose immediate parent is
/// the revoked writ (one-level cascade enforced in the compiler).
#[derive(Clone, Default)]
pub struct Revocations {
    inner: std::sync::Arc<std::sync::RwLock<std::collections::HashSet<thymos_core::WritId>>>,
}

impl Revocations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Revoke a writ by id. Idempotent.
    pub fn revoke(&self, writ_id: thymos_core::WritId) {
        self.inner.write().unwrap().insert(writ_id);
    }

    /// Restore a previously revoked writ (e.g. an erroneous revocation).
    pub fn restore(&self, writ_id: &thymos_core::WritId) {
        self.inner.write().unwrap().remove(writ_id);
    }

    pub fn is_revoked(&self, writ_id: &thymos_core::WritId) -> bool {
        self.inner.read().unwrap().contains(writ_id)
    }

    /// A snapshot of the current revocation set, handed to the compiler.
    pub fn snapshot(&self) -> std::collections::HashSet<thymos_core::WritId> {
        self.inner.read().unwrap().clone()
    }
}

impl Runtime {
    pub fn new(ledger: Ledger, tools: ToolRegistry, policy: PolicyEngine) -> Self {
        Runtime {
            ledger,
            tools,
            policy,
            delegation_keyring: None,
            commit_signer: None,
            redactor: thymos_core::Redactor::default_secrets(),
            revocations: Revocations::new(),
        }
    }

    /// Revoke a writ: subsequent submissions under it (or any child whose
    /// immediate parent is it) are rejected as `AuthorityVoid`, even if the
    /// signature and time window are still valid.
    pub fn revoke_writ(&self, writ_id: thymos_core::WritId) {
        self.revocations.revoke(writ_id);
    }

    /// Builder: attach a [`DelegationKeyring`] so child writs in delegations
    /// are properly signed when the parent's signing key is registered.
    pub fn with_delegation_keyring(mut self, keyring: DelegationKeyring) -> Self {
        self.delegation_keyring = Some(keyring);
        self
    }

    /// Builder: attach a runtime signing key so every appended commit is
    /// ed25519-signed over `canonical_json(body_without_signature)`.
    pub fn with_commit_signer(mut self, signer: thymos_core::crypto::SigningKey) -> Self {
        self.commit_signer = Some(signer);
        self
    }

    /// Builder: set the secret redactor applied to observations before they are
    /// committed. Use `Redactor::none()` to disable (not recommended).
    pub fn with_redactor(mut self, redactor: thymos_core::Redactor) -> Self {
        self.redactor = redactor;
        self
    }

    /// Build a Commit, signing it with the runtime's commit signer if one is
    /// configured. Centralizes the signed/unsigned choice so both the staged
    /// and the approval-resume commit paths stay consistent.
    fn build_commit(&self, body: CommitBody) -> Result<Commit> {
        match &self.commit_signer {
            Some(sk) => Commit::new_signed(body, sk),
            None => Commit::new(body),
        }
    }

    /// Create a new trajectory and return a Run bound to it.
    pub fn create_run(&self, note: &str, seed: &[u8]) -> Result<Run<'_>> {
        let trajectory_id = TrajectoryId::new_from_seed(seed);
        self.ledger.append_root(trajectory_id, note)?;
        Ok(Run {
            runtime: self,
            trajectory_id,
        })
    }

    /// Resume an existing trajectory. The Run picks up where it left off;
    /// world projection will fold every commit already in the ledger. Returns
    /// an error if the trajectory hasn't been rooted yet.
    pub fn resume_run(&self, trajectory_id: TrajectoryId) -> Result<Run<'_>> {
        if !self.ledger.has_trajectory(trajectory_id) {
            return Err(Error::Ledger(format!(
                "trajectory {:?} does not exist",
                trajectory_id
            )));
        }
        Ok(Run {
            runtime: self,
            trajectory_id,
        })
    }
}

pub struct Run<'a> {
    runtime: &'a Runtime,
    trajectory_id: TrajectoryId,
}

/// The result of submitting one Intent to the runtime.
#[derive(Debug)]
pub enum Step {
    Committed(CommitId),
    Rejected(RejectionReason),
    /// Policy returned RequireApproval; the proposal is reified in the ledger.
    Suspended {
        channel: String,
        reason: String,
    },
    /// A delegation was executed — child ran to completion.
    Delegated {
        child_trajectory_id: TrajectoryId,
        final_answer: Option<String>,
    },
}

impl<'a> Run<'a> {
    pub fn trajectory_id(&self) -> TrajectoryId {
        self.trajectory_id
    }

    /// Accessor for the enclosing runtime. Used by the agent loop to reach
    /// the ledger for observation lookup.
    pub fn runtime(&self) -> &Runtime {
        self.runtime
    }

    /// Reconstruct the World projection by folding the ledger for this
    /// trajectory up to the current head. For branched trajectories, first
    /// folds the ancestor chain up to the branch point, then this trajectory's
    /// own commits on top.
    pub fn project_world(&self) -> Result<World> {
        let entries = self.runtime.ledger.entries(self.trajectory_id)?;

        // Check if this is a branch. If so, recursively fold the ancestor.
        let mut world = if let Some(entry) = entries.first() {
            if let EntryPayload::Branch {
                source_trajectory_id,
                source_commit_id,
                ..
            } = &entry.payload
            {
                project_world_up_to(
                    &self.runtime.ledger,
                    *source_trajectory_id,
                    Some(*source_commit_id),
                )?
            } else {
                World::default()
            }
        } else {
            World::default()
        };

        let commits = project_commits(&entries);
        for c in commits {
            world.apply(&c.body.delta, c.id)?;
        }
        Ok(world)
    }

    /// Create a new trajectory branched from a specific commit in this
    /// trajectory. The new Run starts with the world state as of that commit.
    pub fn branch_from(&self, commit_id: CommitId, note: &str) -> Result<Run<'_>> {
        let seed = format!("branch-{}-{}", self.trajectory_id, commit_id);
        let new_traj = TrajectoryId::new_from_seed(seed.as_bytes());
        self.runtime
            .ledger
            .append_branch_root(new_traj, self.trajectory_id, commit_id, note)?;
        Ok(Run {
            runtime: self.runtime,
            trajectory_id: new_traj,
        })
    }

    /// Project accumulated budget usage for this trajectory by summing
    /// `budget_cost` fields across all committed entries.
    pub fn project_budget_used(&self) -> Result<BudgetCost> {
        let entries = self.runtime.ledger.entries(self.trajectory_id)?;
        let mut acc = BudgetCost::default();
        for e in &entries {
            if let EntryPayload::Commit(c) = &e.payload {
                acc = acc.saturating_add(&c.body.budget_cost);
            }
        }
        Ok(acc)
    }

    /// Idempotency helper: the `CommitId` already recorded for `proposal_id` in
    /// this trajectory, if any. Backs exactly-once execution of
    /// External/Irreversible tools — re-submitting or re-approving the same
    /// (content-addressed) proposal returns the prior commit rather than
    /// repeating the side effect.
    fn find_commit_for_proposal(
        &self,
        proposal_id: thymos_core::ProposalId,
    ) -> Result<Option<CommitId>> {
        let entries = self.runtime.ledger.entries(self.trajectory_id)?;
        for e in &entries {
            if let EntryPayload::Commit(c) = &e.payload {
                if c.body.proposal_id == proposal_id {
                    return Ok(Some(c.id));
                }
            }
        }
        Ok(None)
    }

    /// Submit one Intent. Runs it through the full Triad.
    pub fn submit(&self, intent: Intent, writ: &thymos_core::writ::Writ) -> Result<Step> {
        self.submit_with_trace(intent, writ, 0, None)
    }

    /// Submit one intent while emitting structured trace events for operator
    /// surfaces that need live Intent → Proposal → Execution → Result state.
    pub fn submit_with_trace(
        &self,
        intent: Intent,
        writ: &thymos_core::writ::Writ,
        step_index: u32,
        trace: Option<&crate::AgentEventCallback>,
    ) -> Result<Step> {
        #[cfg(feature = "telemetry")]
        let _span = tracing::info_span!(
            "triad.submit",
            tool = %intent.body.target,
            kind = ?intent.body.kind,
            trajectory = %self.trajectory_id,
        )
        .entered();

        // Fold world.
        let world = self.project_world()?;

        // Project budget usage for the compile context.
        let budget_used = self.project_budget_used()?;
        // Source the clock at the runtime layer — the compiler stays pure
        // (see thymos_compiler::CompileContext doc-comment).
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ctx = CompileContext {
            now_unix,
            budget_used,
            revoked: self.runtime.revocations.snapshot(),
        };

        // Compile (with budget + time-window checks).
        #[cfg(feature = "telemetry")]
        let _compile_span = tracing::info_span!("triad.compile").entered();

        let compiled = compile_with_context(
            &intent,
            writ,
            &world,
            &self.runtime.tools,
            &self.runtime.policy,
            &ctx,
        )?;

        #[cfg(feature = "telemetry")]
        drop(_compile_span);

        match compiled {
            Compiled::Rejected(reason) => {
                self.runtime.ledger.append_rejection(
                    self.trajectory_id,
                    intent.id,
                    reason.clone(),
                )?;
                crate::agent::emit_event(
                    trace,
                    crate::AgentTraceEvent::ProposalRejected {
                        step_index,
                        intent_id: intent.id.to_string(),
                        tool: intent.body.target.clone(),
                        reason: format!("{reason:?}"),
                    },
                );
                Ok(Step::Rejected(reason))
            }
            Compiled::Suspended {
                proposal,
                channel,
                reason,
            } => {
                let proposal_id = proposal.id.to_string();
                let proposal_tool = proposal.body.plan.tool.clone();
                // Reify the pending approval in the ledger so it survives restarts.
                self.runtime.ledger.append_pending_approval(
                    self.trajectory_id,
                    proposal,
                    channel.clone(),
                    reason.clone(),
                )?;
                crate::agent::emit_event(
                    trace,
                    crate::AgentTraceEvent::ProposalSuspended {
                        step_index,
                        intent_id: intent.id.to_string(),
                        proposal_id,
                        tool: proposal_tool,
                        channel: channel.clone(),
                        reason: reason.clone(),
                    },
                );
                Ok(Step::Suspended { channel, reason })
            }
            Compiled::Staged(proposal) => {
                crate::agent::emit_event(
                    trace,
                    crate::AgentTraceEvent::ProposalStaged {
                        step_index,
                        intent_id: intent.id.to_string(),
                        proposal_id: proposal.id.to_string(),
                        tool: proposal.body.plan.tool.clone(),
                    },
                );
                // Intercept delegation: spawn a child trajectory instead of
                // executing a tool.
                if proposal.body.plan.tool == "delegate" {
                    return self.execute_delegation(&proposal, writ);
                }

                let tool = self.runtime.tools.get(&proposal.body.plan.tool)?;

                // Idempotency: an External/Irreversible effect must never run
                // twice for the same proposal. ProposalId is content-addressed,
                // so a retry or re-submit of the same intent yields the same id;
                // if a commit already recorded it, return that commit instead of
                // repeating the side effect.
                if matches!(
                    tool.meta().effect_class,
                    EffectClass::External | EffectClass::Irreversible
                ) {
                    if let Some(existing) = self.find_commit_for_proposal(proposal.id)? {
                        return Ok(Step::Committed(existing));
                    }
                }

                // Pre-compute estimated cost for the commit record.
                let estimated_cost = tool.estimate_cost(&proposal.body.plan.args);

                let inv = ToolInvocation {
                    args: &proposal.body.plan.args,
                    world: &world,
                };

                #[cfg(feature = "telemetry")]
                let _exec_span = tracing::info_span!(
                    "triad.execute",
                    tool = %proposal.body.plan.tool,
                )
                .entered();

                crate::agent::emit_event(
                    trace,
                    crate::AgentTraceEvent::ExecutionStarted {
                        step_index,
                        intent_id: intent.id.to_string(),
                        proposal_id: proposal.id.to_string(),
                        tool: proposal.body.plan.tool.clone(),
                    },
                );

                let mut outcome = tool
                    .execute(&inv)
                    .map_err(|e| Error::ToolExecution(e.to_string()))?;

                // Verify postconditions (contract-declared).
                tool.check_postconditions(&inv, &outcome.delta)?;

                // Redact secrets before the observation is persisted to the
                // append-only ledger (and re-surfaced to cognition).
                outcome.observation.output =
                    self.runtime.redactor.redact(&outcome.observation.output);

                #[cfg(feature = "telemetry")]
                {
                    tracing::info!(
                        latency_ms = outcome.observation.latency_ms,
                        delta_ops = outcome.delta.0.len(),
                        "tool executed"
                    );
                    drop(_exec_span);
                }

                crate::agent::emit_event(
                    trace,
                    crate::AgentTraceEvent::ExecutionObserved {
                        step_index,
                        intent_id: intent.id.to_string(),
                        proposal_id: proposal.id.to_string(),
                        tool: proposal.body.plan.tool.clone(),
                        latency_ms: outcome.observation.latency_ms,
                        delta_ops: outcome.delta.0.len(),
                    },
                );

                // Look up parent head for the commit.
                let (parent_hash, parent_seq) = self.runtime.ledger.head(self.trajectory_id)?;

                // Trial-apply the delta to make sure it would commit cleanly.
                let mut trial = world.clone();
                trial.apply(&outcome.delta, CommitId(parent_hash))?;

                // Record actual latency into the budget cost.
                let budget_cost = BudgetCost {
                    wall_clock_ms: outcome.observation.latency_ms,
                    ..estimated_cost
                };

                #[cfg(feature = "telemetry")]
                let _commit_span =
                    tracing::info_span!("triad.commit", seq = parent_seq + 1).entered();

                let commit_body = CommitBody {
                    parent: vec![CommitId(parent_hash)],
                    trajectory_id: self.trajectory_id,
                    proposal_id: proposal.id,
                    intent_id: proposal.body.intent_id,
                    writ_id: writ.id,
                    seq: parent_seq + 1,
                    delta: outcome.delta,
                    observations: vec![outcome.observation],
                    policy_trace: proposal.body.policy_trace.clone(),
                    compiler_version: COMPILER_VERSION.into(),
                    policy_set_hash: self.runtime.policy.policy_set_hash(),
                    budget_cost,
                    signature: None,
                };
                let commit = self.runtime.build_commit(commit_body)?;
                let committed_id = CommitId(commit.id.0);

                self.runtime.ledger.append_commit(commit)?;

                crate::agent::emit_event(
                    trace,
                    crate::AgentTraceEvent::CommitRecorded {
                        step_index,
                        intent_id: intent.id.to_string(),
                        proposal_id: proposal.id.to_string(),
                        tool: proposal.body.plan.tool.clone(),
                        commit_id: committed_id.to_string(),
                        seq: parent_seq + 1,
                    },
                );

                #[cfg(feature = "telemetry")]
                tracing::info!(commit_id = %committed_id, "committed");

                Ok(Step::Committed(committed_id))
            }
        }
    }

    /// Execute a delegation: mint a child writ, create a child trajectory,
    /// record the delegation edge. Returns `Step::Delegated`. The child
    /// trajectory is created but not driven — the caller (agent loop) is
    /// responsible for providing cognition for the child.
    fn execute_delegation(
        &self,
        proposal: &thymos_core::proposal::Proposal,
        parent_writ: &thymos_core::writ::Writ,
    ) -> Result<Step> {
        use thymos_core::writ::{ToolPattern, WritBody};

        let args = &proposal.body.plan.args;
        let child_task = args
            .get("task")
            .and_then(|v| v.as_str())
            .unwrap_or("delegated task")
            .to_string();

        // Extract tool_scopes from args (optional; defaults to parent scopes).
        let child_scopes: Vec<ToolPattern> = args
            .get("tool_scopes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToolPattern::exact))
                    .collect()
            })
            .unwrap_or_else(|| parent_writ.body.tool_scopes.clone());

        // Mint a child writ. Budget is halved from parent's remaining.
        let child_budget = thymos_core::writ::Budget {
            tokens: parent_writ.body.budget.tokens / 2,
            tool_calls: parent_writ.body.budget.tool_calls / 2,
            wall_clock_ms: parent_writ.body.budget.wall_clock_ms / 2,
            usd_millicents: parent_writ.body.budget.usd_millicents / 2,
        };

        // Generate a child key (the child subject becomes the child's issuer
        // for further delegation).
        let child_key = thymos_core::crypto::generate_signing_key();
        let child_pubkey = thymos_core::crypto::public_key_of(&child_key);

        let child_body = WritBody {
            issuer: parent_writ.body.subject.clone(),
            issuer_pubkey: parent_writ.body.subject_pubkey,
            subject: format!("{}-child", parent_writ.body.subject),
            subject_pubkey: child_pubkey,
            parent: Some(parent_writ.id),
            tenant_id: parent_writ.body.tenant_id.clone(),
            tool_scopes: child_scopes,
            budget: child_budget,
            effect_ceiling: parent_writ.body.effect_ceiling.clone(),
            time_window: parent_writ.body.time_window.clone(),
            delegation: thymos_core::writ::DelegationBounds {
                max_depth: parent_writ.body.delegation.max_depth.saturating_sub(1),
                may_subdivide: parent_writ.body.delegation.may_subdivide,
            },
        };

        // Sign the child writ when we have the parent subject's signing key
        // in the runtime's keyring (the parent registers it before delegating).
        // If the parent key isn't registered, fall back to recording the
        // delegation edge without producing a signed writ — agent code that
        // needs to drive the child trajectory will have to mint its own writ.
        let signed_child: Option<thymos_core::writ::Writ> = self
            .runtime
            .delegation_keyring
            .as_ref()
            .and_then(|kr| kr.get(&parent_writ.body.subject_pubkey))
            .and_then(|parent_sk| {
                // mint_child verifies the body is a strict subset of parent's
                // and that issuer_pubkey == parent.subject_pubkey, then signs.
                parent_writ.mint_child(child_body, &parent_sk).ok()
            });

        // If we did mint a signed child, register the child's key so the
        // child can in turn delegate further.
        if signed_child.is_some() {
            if let Some(kr) = &self.runtime.delegation_keyring {
                kr.register(child_key);
            }
        }

        let child_seed = format!("delegate-{}-{}", child_task, proposal.id);
        let child_traj = TrajectoryId::new_from_seed(child_seed.as_bytes());
        self.runtime
            .ledger
            .append_root(child_traj, &format!("delegated: {}", child_task))?;

        // Record the delegation edge in the parent trajectory.
        self.runtime
            .ledger
            .append_delegation(self.trajectory_id, child_traj, &child_task, None)?;

        // Stash the signed child writ on the runtime for downstream agent code
        // to retrieve via `take_pending_child_writ` (keyed by trajectory id).
        if let Some(writ) = signed_child {
            if let Some(kr) = &self.runtime.delegation_keyring {
                kr.stash_writ(child_traj, writ);
            }
        }

        Ok(Step::Delegated {
            child_trajectory_id: child_traj,
            final_answer: None,
        })
    }

    /// Resume a previously suspended proposal. If `approve` is true, the
    /// proposal is executed through the tool and committed. If false, it's
    /// rejected as PolicyDenied.
    pub fn resume_with_approval(
        &self,
        proposal_id: thymos_core::ProposalId,
        approve: bool,
        writ: &thymos_core::writ::Writ,
    ) -> Result<Step> {
        self.resume_with_approval_trace(proposal_id, approve, writ, 0, None)
    }

    pub fn resume_with_approval_trace(
        &self,
        proposal_id: thymos_core::ProposalId,
        approve: bool,
        writ: &thymos_core::writ::Writ,
        step_index: u32,
        trace: Option<&crate::AgentEventCallback>,
    ) -> Result<Step> {
        // Find the PendingApproval entry for this proposal.
        let entries = self.runtime.ledger.entries(self.trajectory_id)?;
        let pending = entries.iter().find_map(|e| {
            if let EntryPayload::PendingApproval {
                proposal, channel, ..
            } = &e.payload
            {
                if proposal.id == proposal_id {
                    return Some((proposal.clone(), channel.clone()));
                }
            }
            None
        });
        let (proposal, approval_channel) = pending.ok_or_else(|| {
            Error::Other(format!(
                "no pending approval for proposal {:?}",
                proposal_id
            ))
        })?;

        crate::agent::emit_event(
            trace,
            crate::AgentTraceEvent::ApprovalResolved {
                step_index,
                proposal_id: proposal.id.to_string(),
                tool: proposal.body.plan.tool.clone(),
                channel: approval_channel,
                approved: approve,
            },
        );

        if !approve {
            self.runtime.ledger.append_rejection(
                self.trajectory_id,
                proposal.body.intent_id,
                RejectionReason::PolicyDenied("approval denied by operator".into()),
            )?;
            return Ok(Step::Rejected(RejectionReason::PolicyDenied(
                "approval denied by operator".into(),
            )));
        }

        // Approved: re-execute the tool against the current world.
        let world = self.project_world()?;
        let tool = self.runtime.tools.get(&proposal.body.plan.tool)?;

        // Idempotency guard (see staged path): an approved External/Irreversible
        // proposal that already produced a commit must not run again — e.g. a
        // double approval or a retry after a partial failure.
        if matches!(
            tool.meta().effect_class,
            EffectClass::External | EffectClass::Irreversible
        ) {
            if let Some(existing) = self.find_commit_for_proposal(proposal.id)? {
                return Ok(Step::Committed(existing));
            }
        }

        let estimated_cost = tool.estimate_cost(&proposal.body.plan.args);

        let inv = ToolInvocation {
            args: &proposal.body.plan.args,
            world: &world,
        };
        crate::agent::emit_event(
            trace,
            crate::AgentTraceEvent::ExecutionStarted {
                step_index,
                intent_id: proposal.body.intent_id.to_string(),
                proposal_id: proposal.id.to_string(),
                tool: proposal.body.plan.tool.clone(),
            },
        );
        let mut outcome = tool
            .execute(&inv)
            .map_err(|e| Error::ToolExecution(e.to_string()))?;
        tool.check_postconditions(&inv, &outcome.delta)?;
        outcome.observation.output = self.runtime.redactor.redact(&outcome.observation.output);
        crate::agent::emit_event(
            trace,
            crate::AgentTraceEvent::ExecutionObserved {
                step_index,
                intent_id: proposal.body.intent_id.to_string(),
                proposal_id: proposal.id.to_string(),
                tool: proposal.body.plan.tool.clone(),
                latency_ms: outcome.observation.latency_ms,
                delta_ops: outcome.delta.0.len(),
            },
        );

        let (parent_hash, parent_seq) = self.runtime.ledger.head(self.trajectory_id)?;

        let mut trial = world.clone();
        trial.apply(&outcome.delta, CommitId(parent_hash))?;

        let budget_cost = BudgetCost {
            wall_clock_ms: outcome.observation.latency_ms,
            ..estimated_cost
        };
        let commit_body = CommitBody {
            parent: vec![CommitId(parent_hash)],
            trajectory_id: self.trajectory_id,
            proposal_id: proposal.id,
            intent_id: proposal.body.intent_id,
            writ_id: writ.id,
            seq: parent_seq + 1,
            delta: outcome.delta,
            observations: vec![outcome.observation],
            policy_trace: proposal.body.policy_trace.clone(),
            compiler_version: COMPILER_VERSION.into(),
            policy_set_hash: self.runtime.policy.policy_set_hash(),
            budget_cost,
            signature: None,
        };
        let commit = self.runtime.build_commit(commit_body)?;
        let committed_id = CommitId(commit.id.0);
        self.runtime.ledger.append_commit(commit)?;
        crate::agent::emit_event(
            trace,
            crate::AgentTraceEvent::CommitRecorded {
                step_index,
                intent_id: proposal.body.intent_id.to_string(),
                proposal_id: proposal.id.to_string(),
                tool: proposal.body.plan.tool.clone(),
                commit_id: committed_id.to_string(),
                seq: parent_seq + 1,
            },
        );
        Ok(Step::Committed(committed_id))
    }

    /// Summarize the trajectory for debugging/demo output.
    pub fn summary(&self) -> Result<TrajectorySummary> {
        let entries = self.runtime.ledger.entries(self.trajectory_id)?;
        let mut commits = 0usize;
        let mut rejections = 0usize;
        let mut roots = 0usize;
        let mut pending_approvals = 0usize;
        for e in &entries {
            match e.kind {
                thymos_ledger::EntryKind::Root => roots += 1,
                thymos_ledger::EntryKind::Commit => commits += 1,
                thymos_ledger::EntryKind::Rejection => rejections += 1,
                thymos_ledger::EntryKind::PendingApproval => pending_approvals += 1,
                thymos_ledger::EntryKind::Delegation => {}
                thymos_ledger::EntryKind::Branch => {}
            }
        }
        self.runtime.ledger.verify_integrity(self.trajectory_id)?;
        Ok(TrajectorySummary {
            entries_total: entries.len(),
            roots,
            commits,
            rejections,
            pending_approvals,
            entries,
        })
    }
}

/// Project world state for a trajectory, optionally stopping at a specific
/// commit (inclusive). Handles recursive ancestor chains for branched
/// trajectories.
fn project_world_up_to(
    ledger: &Ledger,
    trajectory_id: TrajectoryId,
    up_to: Option<CommitId>,
) -> Result<World> {
    let entries = ledger.entries(trajectory_id)?;

    // Recurse into ancestor if this is a branch.
    let mut world = if let Some(entry) = entries.first() {
        if let EntryPayload::Branch {
            source_trajectory_id,
            source_commit_id,
            ..
        } = &entry.payload
        {
            project_world_up_to(ledger, *source_trajectory_id, Some(*source_commit_id))?
        } else {
            World::default()
        }
    } else {
        World::default()
    };

    let commits = project_commits(&entries);
    for c in commits {
        world.apply(&c.body.delta, c.id)?;
        if up_to == Some(c.id) {
            break;
        }
    }
    Ok(world)
}

pub struct TrajectorySummary {
    pub entries_total: usize,
    pub roots: usize,
    pub commits: usize,
    pub rejections: usize,
    pub pending_approvals: usize,
    pub entries: Vec<thymos_ledger::Entry>,
}

// Convenience re-exports for example code.
pub use thymos_core::{
    crypto::{generate_signing_key, public_key_of},
    delta::{DeltaOp, StructuredDelta as Delta},
    intent::{Intent as CoreIntent, IntentBody, IntentKind},
    proposal::{PolicyDecision, RejectionReason as CoreRejectionReason},
    world::{ResourceKey, World as CoreWorld},
    writ::{Budget, DelegationBounds, EffectCeiling, TimeWindow, ToolPattern, Writ, WritBody},
};

#[cfg(test)]
mod keyring_tests {
    use super::*;
    use thymos_core::crypto::{generate_signing_key, public_key_of};

    #[test]
    fn keyring_roundtrips_signing_key_by_pubkey() {
        let kr = DelegationKeyring::new();
        assert!(kr.is_empty());
        let sk = generate_signing_key();
        let pk = public_key_of(&sk);
        kr.register(sk);
        assert_eq!(kr.len(), 1);
        let retrieved = kr.get(&pk).expect("key present");
        assert_eq!(public_key_of(&retrieved), pk);
    }

    #[test]
    fn keyring_returns_none_for_unknown_pubkey() {
        let kr = DelegationKeyring::new();
        let unknown = public_key_of(&generate_signing_key());
        assert!(kr.get(&unknown).is_none());
    }

    #[test]
    fn pending_writs_are_take_once() {
        use thymos_core::TrajectoryId;
        let kr = DelegationKeyring::new();
        let sk = generate_signing_key();
        let parent_pk = public_key_of(&sk);
        // Build a minimal self-signed writ to stash.
        let body = WritBody {
            issuer: "tester".into(),
            issuer_pubkey: parent_pk,
            subject: "tester".into(),
            subject_pubkey: parent_pk,
            parent: None,
            tenant_id: "t".into(),
            tool_scopes: vec![ToolPattern::exact("noop")],
            budget: Budget {
                tokens: 1,
                tool_calls: 1,
                wall_clock_ms: 1,
                usd_millicents: 1,
            },
            effect_ceiling: EffectCeiling::read_write_local(),
            time_window: TimeWindow {
                not_before: 0,
                expires_at: u64::MAX,
            },
            delegation: DelegationBounds {
                max_depth: 0,
                may_subdivide: false,
            },
        };
        let writ = Writ::sign(body, &sk).unwrap();
        let traj = TrajectoryId::new_from_seed(b"x");
        kr.stash_writ(traj, writ);
        assert!(kr.take_pending_child_writ(traj).is_some());
        assert!(kr.take_pending_child_writ(traj).is_none());
    }
}
