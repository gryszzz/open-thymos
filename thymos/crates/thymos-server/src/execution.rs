use serde::Serialize;

use crate::{RunRecord, RunStatus};
use thymos_runtime::{AgentRunSummary, AgentTraceEvent, Termination};

const MAX_LOG_ENTRIES: usize = 400;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl ExecutionStatus {
    /// A run that has reached an end state — no further snapshots will follow,
    /// so SSE subscribers can close the stream instead of hanging open.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ExecutionStatus::Completed | ExecutionStatus::Failed | ExecutionStatus::Cancelled
        )
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    System,
    Intent,
    Proposal,
    Execution,
    Result,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct ExecutionCounters {
    pub steps_started: u32,
    pub intents_declared: u32,
    pub proposals_staged: u32,
    pub commits: u32,
    pub rejections: u32,
    pub failures: u32,
    pub recoveries: u32,
    pub retries: u32,
    pub approvals_pending: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLogLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutionLogEntry {
    pub idx: u64,
    pub timestamp_ms: u64,
    pub phase: ExecutionPhase,
    pub level: ExecutionLogLevel,
    pub title: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutionSession {
    pub run_id: String,
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trajectory_id: Option<String>,
    pub status: ExecutionStatus,
    pub phase: ExecutionPhase,
    pub operator_state: String,
    pub current_step: u32,
    pub max_steps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_answer: Option<String>,
    pub counters: ExecutionCounters,
    pub updated_at_ms: u64,
    pub log: Vec<ExecutionLogEntry>,
    #[serde(skip)]
    next_log_index: u64,
}

impl ExecutionSession {
    pub fn new(run_id: &str, task: &str, max_steps: u32) -> Self {
        let mut session = Self {
            run_id: run_id.into(),
            task: task.into(),
            trajectory_id: None,
            status: ExecutionStatus::Running,
            phase: ExecutionPhase::System,
            operator_state: "Booting Thymos runtime".into(),
            current_step: 0,
            max_steps,
            active_tool: None,
            final_answer: None,
            counters: ExecutionCounters::default(),
            updated_at_ms: now_ms(),
            log: Vec::new(),
            next_log_index: 1,
        };
        session.push_log(
            ExecutionPhase::System,
            ExecutionLogLevel::Info,
            "Runtime accepted task",
            "Thymos initialized a shared execution session for this run.",
            None,
            None,
            None,
            None,
            None,
            None,
        );
        session
    }

    pub fn from_run_record(run_id: &str, record: &RunRecord) -> Self {
        let max_steps = record
            .summary
            .as_ref()
            .map(|summary| summary.steps_executed.max(1))
            .unwrap_or(1);
        let mut session = Self::new(run_id, &record.task, max_steps);
        if !record.trajectory_id.is_empty() {
            session.trajectory_id = Some(record.trajectory_id.clone());
        }
        session.current_step = record
            .summary
            .as_ref()
            .map(|summary| summary.steps_executed)
            .unwrap_or(0);
        session.final_answer = record
            .summary
            .as_ref()
            .and_then(|summary| summary.final_answer.clone());
        session.counters.commits = record
            .summary
            .as_ref()
            .map(|summary| summary.commits)
            .unwrap_or(0);
        session.counters.rejections = record
            .summary
            .as_ref()
            .map(|summary| summary.rejections)
            .unwrap_or(0);
        session.counters.failures = record
            .summary
            .as_ref()
            .map(|summary| summary.failures)
            .unwrap_or(0);
        session.phase = if record.status == RunStatus::Running {
            ExecutionPhase::Intent
        } else {
            ExecutionPhase::Result
        };
        session.status = match record.status {
            RunStatus::Running => ExecutionStatus::Running,
            RunStatus::Completed => ExecutionStatus::Completed,
            RunStatus::Failed => ExecutionStatus::Failed,
        };
        session.operator_state = match record.status {
            RunStatus::Running => "Execution session restored".into(),
            RunStatus::Completed => "Run completed".into(),
            RunStatus::Failed => "Run ended before completion".into(),
        };
        if let Some(summary) = &record.summary {
            session.push_log(
                ExecutionPhase::Result,
                if record.status == RunStatus::Completed {
                    ExecutionLogLevel::Success
                } else {
                    ExecutionLogLevel::Warning
                },
                "Recovered persisted run state",
                format!(
                    "Termination: {}. Commits: {}. Rejections: {}. Failures: {}.",
                    summary.terminated_by, summary.commits, summary.rejections, summary.failures
                ),
                Some(summary.steps_executed.saturating_sub(1)),
                None,
                None,
                None,
                None,
                None,
            );
        }
        session
    }

    pub fn apply_trace(&mut self, event: AgentTraceEvent) {
        match event {
            AgentTraceEvent::RunCreated {
                trajectory_id,
                task: _,
                max_steps,
            } => {
                self.trajectory_id = Some(trajectory_id);
                self.max_steps = max_steps;
                self.status = ExecutionStatus::Running;
                self.phase = ExecutionPhase::Intent;
                self.operator_state = "Planning first move".into();
            }
            AgentTraceEvent::StepStarted {
                step_index,
                since_last_count,
            } => {
                self.status = ExecutionStatus::Running;
                self.phase = ExecutionPhase::Intent;
                self.current_step = step_index + 1;
                self.counters.steps_started = self.counters.steps_started.max(step_index + 1);
                self.operator_state = format!("Planning step {}", step_index + 1);
                self.active_tool = None;
                self.push_log(
                    ExecutionPhase::Intent,
                    ExecutionLogLevel::Info,
                    format!("Step {} entered intent phase", step_index + 1),
                    format!(
                        "Loaded {} fresh runtime observations into the planning loop.",
                        since_last_count
                    ),
                    Some(step_index),
                    None,
                    None,
                    None,
                    None,
                    None,
                );
            }
            AgentTraceEvent::IntentDeclared {
                step_index,
                intent_id,
                tool,
                rationale,
            } => {
                self.phase = ExecutionPhase::Intent;
                self.active_tool = Some(tool.clone());
                self.counters.intents_declared += 1;
                self.operator_state = format!("Intent declared for {}", tool);
                self.push_log(
                    ExecutionPhase::Intent,
                    ExecutionLogLevel::Info,
                    format!("Intent issued for {}", tool),
                    rationale,
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    None,
                    None,
                    None,
                );
            }
            AgentTraceEvent::ProposalStaged {
                step_index,
                intent_id,
                proposal_id,
                tool,
            } => {
                self.phase = ExecutionPhase::Proposal;
                self.active_tool = Some(tool.clone());
                self.counters.proposals_staged += 1;
                self.operator_state = format!("Proposal cleared for {}", tool);
                self.push_log(
                    ExecutionPhase::Proposal,
                    ExecutionLogLevel::Success,
                    format!("Proposal staged for {}", tool),
                    "Policy, writ scope, and budget checks passed.",
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    Some(proposal_id),
                    None,
                    None,
                );
            }
            AgentTraceEvent::ProposalRejected {
                step_index,
                intent_id,
                tool,
                reason,
            } => {
                self.phase = ExecutionPhase::Result;
                self.active_tool = Some(tool.clone());
                self.counters.rejections += 1;
                self.counters.recoveries += 1;
                self.operator_state = "Adjusting after a rejected proposal".into();
                self.push_log(
                    ExecutionPhase::Proposal,
                    ExecutionLogLevel::Warning,
                    format!("Proposal rejected for {}", tool),
                    reason,
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    None,
                    None,
                    None,
                );
            }
            AgentTraceEvent::ProposalSuspended {
                step_index,
                intent_id,
                proposal_id,
                tool,
                channel,
                reason,
            } => {
                self.status = ExecutionStatus::WaitingApproval;
                self.phase = ExecutionPhase::Proposal;
                self.active_tool = Some(tool.clone());
                self.counters.approvals_pending += 1;
                self.operator_state = format!("Awaiting approval on {}", channel);
                self.push_log(
                    ExecutionPhase::Proposal,
                    ExecutionLogLevel::Warning,
                    format!("Approval requested for {}", tool),
                    format!("Channel {}: {}", channel, reason),
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    Some(proposal_id),
                    None,
                    None,
                );
            }
            AgentTraceEvent::ApprovalResolved {
                step_index,
                proposal_id,
                tool,
                channel,
                approved,
            } => {
                self.status = ExecutionStatus::Running;
                self.phase = ExecutionPhase::Proposal;
                self.active_tool = Some(tool.clone());
                self.counters.approvals_pending = self.counters.approvals_pending.saturating_sub(1);
                self.operator_state = if approved {
                    "Approval received; resuming execution".into()
                } else {
                    "Approval denied; agent will adjust".into()
                };
                self.push_log(
                    ExecutionPhase::Proposal,
                    if approved {
                        ExecutionLogLevel::Success
                    } else {
                        ExecutionLogLevel::Warning
                    },
                    if approved {
                        format!("Approval granted for {}", tool)
                    } else {
                        format!("Approval denied for {}", tool)
                    },
                    format!("Channel {} resolved by the operator.", channel),
                    Some(step_index),
                    Some(tool),
                    None,
                    Some(proposal_id),
                    None,
                    None,
                );
            }
            AgentTraceEvent::ExecutionStarted {
                step_index,
                intent_id,
                proposal_id,
                tool,
            } => {
                self.status = ExecutionStatus::Running;
                self.phase = ExecutionPhase::Execution;
                self.active_tool = Some(tool.clone());
                self.operator_state = format!("Executing {}", tool);
                self.push_log(
                    ExecutionPhase::Execution,
                    ExecutionLogLevel::Info,
                    format!("Execution started for {}", tool),
                    "Thymos entered the real tool execution phase.",
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    Some(proposal_id),
                    None,
                    None,
                );
            }
            AgentTraceEvent::ExecutionObserved {
                step_index,
                intent_id,
                proposal_id,
                tool,
                latency_ms,
                delta_ops,
            } => {
                self.phase = ExecutionPhase::Execution;
                self.active_tool = Some(tool.clone());
                self.operator_state = format!("Observed result from {}", tool);
                self.push_log(
                    ExecutionPhase::Execution,
                    ExecutionLogLevel::Success,
                    format!("Execution observed for {}", tool),
                    format!("{} delta ops produced in {} ms.", delta_ops, latency_ms),
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    Some(proposal_id),
                    None,
                    None,
                );
            }
            AgentTraceEvent::CommitRecorded {
                step_index,
                intent_id,
                proposal_id,
                tool,
                commit_id,
                seq,
            } => {
                self.phase = ExecutionPhase::Result;
                self.active_tool = Some(tool.clone());
                self.counters.commits += 1;
                self.operator_state = format!("Committed result from {}", tool);
                self.push_log(
                    ExecutionPhase::Result,
                    ExecutionLogLevel::Success,
                    format!("Result committed for {}", tool),
                    format!("Ledger seq {} accepted and stored.", seq),
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    Some(proposal_id),
                    Some(commit_id),
                    Some(seq),
                );
            }
            AgentTraceEvent::ExecutionFailed {
                step_index,
                intent_id,
                tool,
                error,
            } => {
                self.phase = ExecutionPhase::Result;
                self.active_tool = Some(tool.clone());
                self.counters.failures += 1;
                self.counters.recoveries += 1;
                self.operator_state = "Recovering from a runtime failure".into();
                self.push_log(
                    ExecutionPhase::Result,
                    ExecutionLogLevel::Error,
                    format!("Execution failed for {}", tool),
                    error,
                    Some(step_index),
                    Some(tool),
                    Some(intent_id),
                    None,
                    None,
                    None,
                );
            }
            AgentTraceEvent::RetryScheduled {
                step_index,
                attempt,
                delay_ms,
                message,
            } => {
                self.phase = ExecutionPhase::System;
                self.counters.retries += 1;
                self.operator_state = "Retrying a transient cognition failure".into();
                self.push_log(
                    ExecutionPhase::System,
                    ExecutionLogLevel::Warning,
                    format!("Retry {} scheduled", attempt),
                    format!("Backing off for {} ms. {}", delay_ms, message),
                    Some(step_index),
                    None,
                    None,
                    None,
                    None,
                    None,
                );
            }
            AgentTraceEvent::RunFinished {
                trajectory_id,
                steps_executed,
                intents_submitted,
                commits,
                rejections,
                failures,
                final_answer,
                terminated_by,
            } => {
                self.trajectory_id = Some(trajectory_id);
                self.current_step = steps_executed;
                self.counters.steps_started = self.counters.steps_started.max(steps_executed);
                self.counters.intents_declared =
                    self.counters.intents_declared.max(intents_submitted);
                self.counters.commits = commits;
                self.counters.rejections = rejections;
                self.counters.failures = failures;
                self.final_answer = final_answer;
                self.phase = ExecutionPhase::Result;
                self.status = if terminated_by == format!("{:?}", Termination::CognitionDone) {
                    ExecutionStatus::Completed
                } else {
                    ExecutionStatus::Failed
                };
                self.operator_state = match self.status {
                    ExecutionStatus::Completed => "Task resolved and verified by runtime".into(),
                    _ => format!("Run ended before completion: {}", terminated_by),
                };
                self.push_log(
                    ExecutionPhase::Result,
                    if self.status == ExecutionStatus::Completed {
                        ExecutionLogLevel::Success
                    } else {
                        ExecutionLogLevel::Error
                    },
                    "Run finished",
                    format!(
                        "Termination: {}. Commits: {}. Rejections: {}. Failures: {}.",
                        terminated_by, commits, rejections, failures
                    ),
                    Some(steps_executed.saturating_sub(1)),
                    self.active_tool.clone(),
                    None,
                    None,
                    None,
                    None,
                );
            }
        }
        self.updated_at_ms = now_ms();
    }

    pub fn apply_summary(&mut self, summary: &AgentRunSummary) {
        self.trajectory_id = Some(summary.trajectory_id.to_string());
        self.current_step = summary.steps_executed;
        self.counters.steps_started = self.counters.steps_started.max(summary.steps_executed);
        self.counters.intents_declared = self
            .counters
            .intents_declared
            .max(summary.intents_submitted);
        self.counters.commits = summary.commits;
        self.counters.rejections = summary.rejections;
        self.counters.failures = summary.failures;
        self.final_answer = summary.final_answer.clone();
        self.phase = ExecutionPhase::Result;
        self.status = if matches!(summary.terminated_by, Termination::CognitionDone) {
            ExecutionStatus::Completed
        } else {
            ExecutionStatus::Failed
        };
        self.operator_state = match self.status {
            ExecutionStatus::Completed => "Task resolved and verified by runtime".into(),
            _ => format!("Run ended before completion: {:?}", summary.terminated_by),
        };
        self.updated_at_ms = now_ms();
    }

    pub fn mark_failed(&mut self, detail: impl Into<String>) {
        let detail = detail.into();
        self.status = ExecutionStatus::Failed;
        self.phase = ExecutionPhase::Result;
        self.operator_state = "Runtime stopped on an unrecovered error".into();
        // The chat renders `final_answer` as the reply bubble; give it the
        // plain-English line. The raw detail stays in the log entry below.
        self.final_answer = Some(thymos_runtime::agent_async::humanize_provider_error(
            &detail,
        ));
        self.push_log(
            ExecutionPhase::Result,
            ExecutionLogLevel::Error,
            "Run failed",
            detail,
            Some(self.current_step.saturating_sub(1)),
            self.active_tool.clone(),
            None,
            None,
            None,
            None,
        );
    }

    pub fn mark_cancelled(&mut self) {
        self.status = ExecutionStatus::Cancelled;
        self.phase = ExecutionPhase::Result;
        self.operator_state = "Run cancelled by operator".into();
        self.final_answer = Some("Run cancelled.".into());
        self.push_log(
            ExecutionPhase::Result,
            ExecutionLogLevel::Warning,
            "Run cancelled",
            "The operator cancelled this execution session.",
            Some(self.current_step.saturating_sub(1)),
            self.active_tool.clone(),
            None,
            None,
            None,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push_log(
        &mut self,
        phase: ExecutionPhase,
        level: ExecutionLogLevel,
        title: impl Into<String>,
        detail: impl Into<String>,
        step_index: Option<u32>,
        tool: Option<String>,
        intent_id: Option<String>,
        proposal_id: Option<String>,
        commit_id: Option<String>,
        seq: Option<u64>,
    ) {
        self.log.push(ExecutionLogEntry {
            idx: self.next_log_index,
            timestamp_ms: now_ms(),
            phase,
            level,
            title: title.into(),
            detail: detail.into(),
            step_index,
            tool,
            intent_id,
            proposal_id,
            commit_id,
            seq,
        });
        self.next_log_index += 1;
        if self.log.len() > MAX_LOG_ENTRIES {
            let overflow = self.log.len() - MAX_LOG_ENTRIES;
            self.log.drain(0..overflow);
        }
        self.updated_at_ms = now_ms();
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
