//! Agent loop: drives a `Cognition` through the Thymos IPC Triad until
//! termination, budget exhaustion, or max steps.
//!
//! The loop's only job is:
//!   (1) build a fresh `CognitionContext` each step,
//!   (2) ask cognition for a batch of Intents,
//!   (3) submit each through the Triad, capturing typed outcomes,
//!   (4) accumulate those outcomes as `HistoryItem`s for the next step,
//!   (5) stop when cognition returns an empty step.
//!
//! All state lives in the ledger. The agent loop is stateless except for the
//! small window of `HistoryItem`s produced in the current step (which is
//! handed to cognition on the next one).

use std::sync::Arc;

use serde::Serialize;
use thymos_cognition::{Cognition, CognitionContext, HistoryItem};
use thymos_core::{
    commit::Observation,
    error::{Error, Result},
    intent::Intent,
    writ::Writ,
    TrajectoryId,
};
use thymos_ledger::{Entry, EntryPayload};

use crate::{Run, Runtime, Step};

#[derive(Debug)]
pub struct AgentRunSummary {
    pub trajectory_id: TrajectoryId,
    pub steps_executed: u32,
    pub intents_submitted: u32,
    pub commits: u32,
    pub rejections: u32,
    pub failures: u32,
    pub final_answer: Option<String>,
    pub terminated_by: Termination,
}

#[derive(Debug)]
pub enum Termination {
    CognitionDone,
    MaxStepsReached,
    Suspended,
    WritExpired,
    /// Cumulative cognition token/USD usage exceeded the writ budget. Tool
    /// budget is enforced separately by the compiler; this bounds model spend.
    BudgetExhausted,
}

pub struct AgentRunOptions {
    pub max_steps: u32,
}

impl Default for AgentRunOptions {
    fn default() -> Self {
        // 4.7 handles longer tool-chained reasoning; the budget in the Writ
        // remains the authoritative cap, so this ceiling only guards runaway
        // cognition loops.
        AgentRunOptions { max_steps: 32 }
    }
}

pub type AgentEventCallback = Arc<dyn Fn(AgentTraceEvent) + Send + Sync>;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentTraceEvent {
    RunCreated {
        trajectory_id: String,
        task: String,
        max_steps: u32,
    },
    StepStarted {
        step_index: u32,
        since_last_count: usize,
    },
    IntentDeclared {
        step_index: u32,
        intent_id: String,
        tool: String,
        rationale: String,
    },
    ProposalStaged {
        step_index: u32,
        intent_id: String,
        proposal_id: String,
        tool: String,
    },
    ProposalRejected {
        step_index: u32,
        intent_id: String,
        tool: String,
        reason: String,
    },
    ProposalSuspended {
        step_index: u32,
        intent_id: String,
        proposal_id: String,
        tool: String,
        channel: String,
        reason: String,
    },
    ExecutionStarted {
        step_index: u32,
        intent_id: String,
        proposal_id: String,
        tool: String,
    },
    ExecutionObserved {
        step_index: u32,
        intent_id: String,
        proposal_id: String,
        tool: String,
        latency_ms: u64,
        delta_ops: usize,
    },
    CommitRecorded {
        step_index: u32,
        intent_id: String,
        proposal_id: String,
        tool: String,
        commit_id: String,
        seq: u64,
    },
    ExecutionFailed {
        step_index: u32,
        intent_id: String,
        tool: String,
        error: String,
    },
    ApprovalResolved {
        step_index: u32,
        proposal_id: String,
        tool: String,
        channel: String,
        approved: bool,
    },
    RetryScheduled {
        step_index: u32,
        attempt: u32,
        delay_ms: u64,
        message: String,
    },
    RunFinished {
        trajectory_id: String,
        steps_executed: u32,
        intents_submitted: u32,
        commits: u32,
        rejections: u32,
        failures: u32,
        final_answer: Option<String>,
        terminated_by: String,
    },
}

pub(crate) fn emit_event(event_tx: Option<&AgentEventCallback>, event: AgentTraceEvent) {
    if let Some(tx) = event_tx {
        tx(event);
    }
}

/// Drive a Cognition to completion against the Thymos runtime.
///
/// The loop is explicit: cognition proposes, runtime decides, ledger remembers.
pub fn run_agent(
    runtime: &Runtime,
    cognition: &mut dyn Cognition,
    task: &str,
    writ: &Writ,
    opts: AgentRunOptions,
    event_tx: Option<AgentEventCallback>,
) -> Result<AgentRunSummary> {
    let run = runtime.create_run(task, task.as_bytes())?;
    let trajectory_id = run.trajectory_id();
    emit_event(
        event_tx.as_ref(),
        AgentTraceEvent::RunCreated {
            trajectory_id: trajectory_id.to_string(),
            task: task.to_string(),
            max_steps: opts.max_steps,
        },
    );

    #[cfg(feature = "telemetry")]
    tracing::info!(
        %trajectory_id,
        max_steps = opts.max_steps,
        "agent run started"
    );

    let mut since_last: Vec<HistoryItem> = Vec::new();
    let mut steps_executed = 0u32;
    // Cumulative cognition (model) spend, debited against the writ budget so
    // that model token/USD usage is capability-bounded — not just tool calls.
    let mut cognition_tokens_used = 0u64;
    let mut cognition_usd_used = 0u64;
    let mut intents_submitted = 0u32;
    let mut commits = 0u32;
    let mut rejections = 0u32;
    let mut failures = 0u32;
    let mut final_answer: Option<String> = None;
    let mut terminated_by = Termination::MaxStepsReached;

    for step_idx in 0..opts.max_steps {
        #[cfg(feature = "telemetry")]
        let _step_span = tracing::info_span!("agent.step", step = step_idx).entered();

        let world = run.project_world()?;

        let since_last_count = since_last.len();
        emit_event(
            event_tx.as_ref(),
            AgentTraceEvent::StepStarted {
                step_index: step_idx,
                since_last_count,
            },
        );

        let step = cognition.step(&CognitionContext {
            task,
            writ,
            world: &world,
            tools: &runtime.tools,
            since_last: std::mem::take(&mut since_last),
            step_index: step_idx,
        })?;

        steps_executed += 1;

        // Debit cognition usage against the writ budget. The tokens were
        // already spent producing this step, so if cumulative spend exceeds
        // the budget we stop before submitting any of this step's intents.
        cognition_tokens_used = cognition_tokens_used.saturating_add(step.usage.total_tokens());
        cognition_usd_used = cognition_usd_used.saturating_add(step.usage.usd_millicents);
        if cognition_tokens_used > writ.body.budget.tokens
            || cognition_usd_used > writ.body.budget.usd_millicents
        {
            terminated_by = Termination::BudgetExhausted;
            final_answer = Some(format!(
                "cognition budget exhausted: used {} tokens / {} usd_millicents; writ allows {} / {}",
                cognition_tokens_used,
                cognition_usd_used,
                writ.body.budget.tokens,
                writ.body.budget.usd_millicents
            ));
            break;
        }

        if step.intents.is_empty() {
            final_answer = step.final_answer;
            terminated_by = Termination::CognitionDone;
            break;
        }

        for intent in step.intents {
            intents_submitted += 1;
            emit_event(
                event_tx.as_ref(),
                AgentTraceEvent::IntentDeclared {
                    step_index: step_idx,
                    intent_id: intent.id.to_string(),
                    tool: intent.body.target.clone(),
                    rationale: intent.body.rationale.clone(),
                },
            );
            let result =
                match run.submit_with_trace(intent.clone(), writ, step_idx, event_tx.as_ref(), None) {
                    Ok(result) => result,
                    Err(err) => {
                        failures += 1;
                        let error = err.to_string();
                        emit_event(
                            event_tx.as_ref(),
                            AgentTraceEvent::ExecutionFailed {
                                step_index: step_idx,
                                intent_id: intent.id.to_string(),
                                tool: intent.body.target.clone(),
                                error: error.clone(),
                            },
                        );
                        since_last.push(HistoryItem::Failed { intent, error });
                        continue;
                    }
                };

            match result {
                Step::Committed(commit_id) => {
                    commits += 1;
                    let observation = last_observation(&run, commit_id)?;
                    since_last.push(HistoryItem::Committed {
                        intent,
                        observation,
                    });
                }
                Step::Rejected(reason) => {
                    rejections += 1;
                    since_last.push(HistoryItem::Rejected { intent, reason });
                }
                Step::Suspended { channel, reason } => {
                    terminated_by = Termination::Suspended;
                    return Ok(AgentRunSummary {
                        trajectory_id,
                        steps_executed,
                        intents_submitted,
                        commits,
                        rejections,
                        failures,
                        final_answer: Some(format!(
                            "suspended for approval on channel '{}': {}",
                            channel, reason
                        )),
                        terminated_by,
                    });
                }
                Step::Delegated {
                    child_trajectory_id,
                    final_answer: child_answer,
                } => {
                    // Surface the delegation result as a committed observation
                    // so cognition sees it on the next turn.
                    let observation = thymos_core::commit::Observation {
                        tool: "delegate".into(),
                        output: serde_json::json!({
                            "child_trajectory_id": child_trajectory_id.to_string(),
                            "final_answer": child_answer,
                        }),
                        latency_ms: 0,
                    };
                    since_last.push(HistoryItem::Committed {
                        intent,
                        observation,
                    });
                }
            }
        }
    }

    #[cfg(feature = "telemetry")]
    tracing::info!(
        %trajectory_id,
        steps_executed,
        intents_submitted,
        commits,
        rejections,
        terminated_by = ?terminated_by,
        "agent run finished"
    );

    Ok(AgentRunSummary {
        trajectory_id,
        steps_executed,
        intents_submitted,
        commits,
        rejections,
        failures,
        final_answer,
        terminated_by,
    })
}

/// Fetch the Observation from the commit that just landed. The ledger is the
/// source of truth — we don't trust cached values.
fn last_observation(run: &Run<'_>, commit_id: thymos_core::CommitId) -> Result<Observation> {
    let entries: Vec<Entry> = run.runtime().ledger.entries(run.trajectory_id())?;
    for e in entries.into_iter().rev() {
        if let EntryPayload::Commit(c) = &e.payload {
            if c.id == commit_id {
                return c
                    .body
                    .observations
                    .first()
                    .cloned()
                    .ok_or_else(|| Error::Other("commit has no observation".into()));
            }
        }
    }
    Err(Error::Other(format!(
        "commit {:?} not found in trajectory",
        commit_id
    )))
}

// Convenience: so that `Intent` equality in history is cheap for debugging.
#[allow(dead_code)]
fn _intent_eq(a: &Intent, b: &Intent) -> bool {
    a.id == b.id
}
