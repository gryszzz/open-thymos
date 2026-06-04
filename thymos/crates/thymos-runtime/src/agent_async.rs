//! Async agent loop: drives a `StreamingCognition` through the Thymos IPC
//! Triad with token-level streaming and non-blocking tool execution.
//!
//! This is the async counterpart of `agent.rs`. It:
//!   1. Calls `step_streaming` to get token events + a CognitionStep
//!   2. Submits Intents through the Triad (still sync — ledger is sync)
//!   3. Forwards CognitionEvents to the caller via a broadcast channel
//!   4. On `Suspended`, waits for approval via a callback channel
//!   5. Returns the same `AgentRunSummary` as the sync loop

use tokio::sync::{broadcast, mpsc};

use thymos_cognition::{CognitionContext, CognitionEvent, HistoryItem, StreamingCognition};
use thymos_core::{
    commit::Observation,
    error::{Error, Result},
    writ::Writ,
    ProposalId,
};
use thymos_ledger::{Entry, EntryPayload, LedgerStore};

use super::agent::{
    emit_event, AgentEventCallback, AgentRunOptions, AgentRunSummary, AgentTraceEvent, Termination,
};
use crate::{Run, Runtime, Step};

/// Approval decision sent through the approval channel.
#[derive(Debug, Clone)]
pub struct ApprovalDecision {
    pub approve: bool,
}

/// Callback for requesting approvals from an external system (e.g. HTTP endpoint).
/// The agent loop calls `request_approval` when a proposal is suspended; the
/// returned `oneshot::Receiver` resolves when the operator decides.
pub type ApprovalRequester = Box<
    dyn Fn(String, ProposalId, String, String) -> tokio::sync::oneshot::Receiver<ApprovalDecision>
        + Send
        + Sync,
>;

/// Run an async agent loop with streaming cognition events.
///
/// `event_tx` receives every `CognitionEvent` the model emits (tokens,
/// tool-use deltas, turn completions). The caller can forward these over
/// SSE to a client.
///
/// `approval_requester` is called when a proposal needs human approval. If
/// `None`, suspensions terminate the run (Phase 1 behavior).
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_streaming<L: LedgerStore>(
    runtime: &Runtime<L>,
    cognition: &mut dyn StreamingCognition,
    task: &str,
    writ: &Writ,
    opts: AgentRunOptions,
    event_tx: broadcast::Sender<CognitionEvent>,
    approval_requester: Option<ApprovalRequester>,
    trace_tx: Option<AgentEventCallback>,
) -> Result<AgentRunSummary> {
    let run = runtime.create_run(task, task.as_bytes())?;
    let trajectory_id = run.trajectory_id();
    emit_event(
        trace_tx.as_ref(),
        AgentTraceEvent::RunCreated {
            trajectory_id: trajectory_id.to_string(),
            task: task.to_string(),
            max_steps: opts.max_steps,
        },
    );

    let mut since_last: Vec<HistoryItem> = Vec::new();
    let mut steps_executed = 0u32;
    let mut intents_submitted = 0u32;
    let mut commits = 0u32;
    let mut rejections = 0u32;
    let mut failures = 0u32;
    let mut final_answer: Option<String> = None;
    let mut terminated_by = Termination::MaxStepsReached;

    for step_idx in 0..opts.max_steps {
        // Check writ time window.
        if writ.body.time_window.expires_at != u64::MAX {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now > writ.body.time_window.expires_at {
                terminated_by = Termination::WritExpired;
                final_answer = Some("writ has expired".into());
                break;
            }
        }

        let world = run.project_world()?;

        // Create a per-step channel for cognition events.
        let (step_tx, mut step_rx) = mpsc::channel::<CognitionEvent>(64);

        // Forward events from the step channel to the broadcast channel.
        let event_tx2 = event_tx.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(evt) = step_rx.recv().await {
                let _ = event_tx2.send(evt);
            }
        });

        let since_last_count = since_last.len();
        emit_event(
            trace_tx.as_ref(),
            AgentTraceEvent::StepStarted {
                step_index: step_idx,
                since_last_count,
            },
        );

        let ctx = CognitionContext {
            task,
            writ,
            world: &world,
            tools: &runtime.tools,
            since_last: std::mem::take(&mut since_last),
            step_index: step_idx,
        };

        // Retry with exponential backoff on transient errors.
        let step = {
            let mut attempt = 0u32;
            let max_retries = 3u32;
            loop {
                match cognition.step_streaming(&ctx, step_tx.clone()).await {
                    Ok(s) => break s,
                    Err(e) => {
                        attempt += 1;
                        let is_retryable = {
                            let msg = e.to_string().to_lowercase();
                            msg.contains("429")
                                || msg.contains("rate limit")
                                || msg.contains("500")
                                || msg.contains("503")
                                || msg.contains("timeout")
                                || msg.contains("connection")
                        };
                        if !is_retryable || attempt > max_retries {
                            return Err(e);
                        }
                        let backoff_ms = 1000 * 2u64.pow(attempt - 1); // 1s, 2s, 4s
                        emit_event(
                            trace_tx.as_ref(),
                            AgentTraceEvent::RetryScheduled {
                                step_index: step_idx,
                                attempt,
                                delay_ms: backoff_ms,
                                message: e.to_string(),
                            },
                        );
                        let _ = event_tx.send(CognitionEvent::Error {
                            message: format!(
                                "transient error (attempt {}/{}), retrying in {}ms: {}",
                                attempt, max_retries, backoff_ms, e
                            ),
                        });
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    }
                }
            }
        };

        // Drop the local sender so the forwarder's recv() returns None once
        // the clone held by `step_streaming` has also been dropped. Without
        // this, the forwarder hangs forever and the agent loop deadlocks.
        drop(step_tx);
        let _ = forwarder.await;

        steps_executed += 1;

        if step.intents.is_empty() {
            final_answer = step.final_answer;
            terminated_by = Termination::CognitionDone;
            break;
        }

        for intent in step.intents {
            intents_submitted += 1;
            emit_event(
                trace_tx.as_ref(),
                AgentTraceEvent::IntentDeclared {
                    step_index: step_idx,
                    intent_id: intent.id.to_string(),
                    tool: intent.body.target.clone(),
                    rationale: intent.body.rationale.clone(),
                },
            );
            let result = {
                let intent_clone = intent.clone();
                let writ_clone = writ.clone();
                run.submit_with_trace(intent_clone, &writ_clone, step_idx, trace_tx.as_ref(), None)
            };
            let result = match result {
                Ok(result) => result,
                Err(err) => {
                    failures += 1;
                    let error = err.to_string();
                    emit_event(
                        trace_tx.as_ref(),
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
                    // Find the proposal ID from the ledger (last PendingApproval entry).
                    let proposal_id = find_last_pending_proposal(&run)?;

                    match &approval_requester {
                        Some(requester) => {
                            // Notify SSE clients about the pending approval.
                            let _ = event_tx.send(CognitionEvent::Error {
                                message: format!(
                                    "awaiting approval on channel '{}': {}",
                                    channel, reason
                                ),
                            });

                            // Request approval and wait.
                            let rx = requester(
                                trajectory_id.to_string(),
                                proposal_id,
                                channel.clone(),
                                reason.clone(),
                            );

                            match rx.await {
                                Ok(decision) => {
                                    let step_result = run.resume_with_approval_trace(
                                        proposal_id,
                                        decision.approve,
                                        writ,
                                        step_idx,
                                        trace_tx.as_ref(),
                                    );
                                    let step_result = match step_result {
                                        Ok(step_result) => step_result,
                                        Err(err) => {
                                            failures += 1;
                                            let error = err.to_string();
                                            emit_event(
                                                trace_tx.as_ref(),
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
                                    match step_result {
                                        Step::Committed(commit_id) => {
                                            commits += 1;
                                            let observation = last_observation(&run, commit_id)?;
                                            since_last.push(HistoryItem::Committed {
                                                intent,
                                                observation,
                                            });
                                        }
                                        Step::Rejected(rej_reason) => {
                                            rejections += 1;
                                            since_last.push(HistoryItem::Rejected {
                                                intent,
                                                reason: rej_reason,
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                                Err(_) => {
                                    // Channel dropped — approval requester died.
                                    terminated_by = Termination::Suspended;
                                    return Ok(AgentRunSummary {
                                        trajectory_id,
                                        steps_executed,
                                        intents_submitted,
                                        commits,
                                        rejections,
                                        failures,
                                        final_answer: Some(format!(
                                            "approval channel dropped for '{}': {}",
                                            channel, reason
                                        )),
                                        terminated_by,
                                    });
                                }
                            }
                        }
                        None => {
                            // No approval callback — terminate (Phase 1 behavior).
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
                    }
                }
                Step::Delegated {
                    child_trajectory_id,
                    final_answer: child_answer,
                } => {
                    let observation = Observation {
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

/// Find the ProposalId of the most recent PendingApproval entry in this run.
fn find_last_pending_proposal<L: LedgerStore>(run: &Run<'_, L>) -> Result<ProposalId> {
    let entries: Vec<Entry> = run.runtime().ledger.entries(run.trajectory_id())?;
    for e in entries.into_iter().rev() {
        if let EntryPayload::PendingApproval { proposal, .. } = &e.payload {
            return Ok(proposal.id);
        }
    }
    Err(Error::Other(
        "no pending approval found in trajectory".into(),
    ))
}

/// Fetch the Observation from the commit that just landed.
fn last_observation<L: LedgerStore>(
    run: &Run<'_, L>,
    commit_id: thymos_core::CommitId,
) -> Result<Observation> {
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
