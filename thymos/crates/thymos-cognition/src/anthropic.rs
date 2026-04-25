//! Anthropic Messages API cognition adapter.
//!
//! A stateful `Cognition` implementation that confines the model to the role
//! of Proposer: it emits `tool_use` content blocks, which this adapter
//! translates into `Intent`s. Execution, authority, and state mutation remain
//! with the runtime. Rejections observed on prior turns are surfaced back to
//! the model as `tool_result` blocks with `is_error=true`, forming a typed,
//! lossless feedback loop.
//!
//! This adapter targets Anthropic Opus 4.7 as the default. Earlier models
//! (Opus 4.6, Sonnet 4.6, Haiku 4.5) remain selectable via `with_model`.
//!
//! Capabilities:
//!   * Synchronous blocking HTTP (streaming handled by `StreamingCognition`).
//!   * Prompt caching of the stable prefix (system + tools + writ opener),
//!     enabled by default on 4.7-class models via `cache_control`.
//!   * Extended thinking (opt-in via `with_thinking`), for long-horizon tasks.
//!   * Tool choice is `auto`; any Writ-authorized tool may be called.
//!   * Bounded retries on transient transport / 429 / 5xx / overloaded_error.
//!   * Internal message trimming keeps the conversation under a configurable
//!     byte ceiling so long runs don't blow the context window.
//!
//! Environment:
//!   * `ANTHROPIC_API_KEY` — required for live requests.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};

use thymos_core::{
    error::{Error, Result},
    ids::IntentId,
    intent::{Intent, IntentBody, IntentKind},
    proposal::RejectionReason,
    writ::Writ,
};
use thymos_tools::ToolRegistry;

use crate::{Cognition, CognitionContext, CognitionStep, HistoryItem};

/// Default model id. Override via [`AnthropicCognition::with_model`].
///
/// As of this migration, Opus 4.7 is the current flagship. It preserves the
/// 4.6 tool-use schema, adds stronger long-horizon loop discipline, and
/// supports interleaved extended thinking.
pub const DEFAULT_MODEL: &str = "claude-opus-4-7";

/// Legacy alias the runtime will transparently upgrade to [`DEFAULT_MODEL`]
/// unless the caller explicitly asks for the older model by its full id.
pub const LEGACY_MODEL_4_6: &str = "claude-opus-4-6";

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Default ceiling on assistant/user message pairs kept verbatim inside the
/// adapter's `messages` buffer. Older pairs are compacted into a text summary.
const DEFAULT_MAX_INTERNAL_MESSAGES: usize = 48;

/// Default number of retries for transient Anthropic errors
/// (429, 5xx, overloaded_error, network errors). With exponential backoff.
const DEFAULT_MAX_RETRIES: u32 = 3;

/// Extended-thinking configuration for 4.7. Thinking tokens are billed as
/// output tokens and must be ≤ `max_tokens - 1` to be valid.
#[derive(Clone, Debug)]
pub struct ThinkingConfig {
    pub budget_tokens: u32,
}

/// One round of Anthropic model output converted into Thymos types.
pub struct AnthropicCognition {
    client: reqwest::blocking::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    thinking: Option<ThinkingConfig>,
    cache_prefix: bool,
    max_internal_messages: usize,
    max_retries: u32,
    /// Accumulated conversation. Appended on every round.
    messages: Vec<Value>,
    /// Mapping from the Intent we produced → the provider tool_use_id that
    /// sourced it. Used to correlate committed/rejected outcomes back onto
    /// the correct tool_result in the next turn.
    correlations: HashMap<IntentId, String>,
    /// Tool_use ids ordered as the provider emitted them in the LAST assistant
    /// turn. Every one of these MUST be answered (the API enforces this);
    /// ids whose corresponding Intent was not submitted get a synthetic
    /// "not executed" tool_result in the next turn.
    last_tool_uses: Vec<String>,
    /// Signature of the (target, args) pairs emitted on the last turn. Used
    /// to detect degenerate retry loops where cognition re-proposes an
    /// identical rejected call.
    last_call_signatures: Vec<String>,
    /// Accumulated token counters across all turns, for observability.
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_write_tokens: u64,
}

impl AnthropicCognition {
    /// Construct a new client reading `ANTHROPIC_API_KEY` from the environment.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| Error::Other("ANTHROPIC_API_KEY is not set".into()))?;
        Self::with_api_key(api_key)
    }

    pub fn with_api_key(api_key: String) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| Error::Other(format!("reqwest client build: {e}")))?;
        Ok(AnthropicCognition {
            client,
            api_key,
            model: DEFAULT_MODEL.into(),
            max_tokens: 4096,
            thinking: None,
            cache_prefix: true,
            max_internal_messages: DEFAULT_MAX_INTERNAL_MESSAGES,
            max_retries: DEFAULT_MAX_RETRIES,
            messages: Vec::new(),
            correlations: HashMap::new(),
            last_tool_uses: Vec::new(),
            last_call_signatures: Vec::new(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_write_tokens: 0,
        })
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = resolve_model_alias(model.into());
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Enable extended thinking with the given token budget. The runtime will
    /// ensure `budget_tokens < max_tokens`.
    pub fn with_thinking(mut self, budget_tokens: u32) -> Self {
        self.thinking = Some(ThinkingConfig { budget_tokens });
        self
    }

    /// Disable the prompt-cache breakpoint on the stable prefix.
    /// Caching is on by default and rarely worth turning off.
    pub fn without_cache_prefix(mut self) -> Self {
        self.cache_prefix = false;
        self
    }

    pub fn with_max_internal_messages(mut self, n: usize) -> Self {
        self.max_internal_messages = n.max(4);
        self
    }

    /// Configure the number of retries for transient transport failures.
    /// Also used by the streaming adapter when wrapping this cognition.
    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }
}

/// Classifier: is this HTTP status (and optional error body) retryable?
///
/// Extracted as a free function so both the blocking and streaming code paths
/// can share a single policy, and so it is directly unit-testable.
pub fn is_transient_status(status: u16, error_type: Option<&str>) -> bool {
    if status == 429 || (500..600).contains(&status) {
        return true;
    }
    matches!(
        error_type,
        Some("overloaded_error") | Some("api_error") | Some("timeout")
    )
}

/// Compute the exponential backoff delay (ms) for the Nth retry attempt.
/// Shared between sync and streaming retry loops.
pub fn backoff_delay_ms(attempt: u32) -> u64 {
    500u64 << attempt.min(6)
}

/// Map short / legacy aliases to canonical model ids.
///
/// Explicit full ids pass through unchanged, so operators can pin any model.
fn resolve_model_alias(name: String) -> String {
    match name.as_str() {
        "opus" | "opus-4.7" | "opus-latest" => "claude-opus-4-7".into(),
        "opus-4.6" => "claude-opus-4-6".into(),
        "sonnet" | "sonnet-4.6" | "sonnet-latest" => "claude-sonnet-4-6".into(),
        "haiku" | "haiku-4.5" | "haiku-latest" => "claude-haiku-4-5".into(),
        _ => name,
    }
}

impl Cognition for AnthropicCognition {
    fn step(&mut self, ctx: &CognitionContext<'_>) -> Result<CognitionStep> {
        // 1. On the first call, seed the conversation with the task + context.
        //    On subsequent calls, convert `since_last` into tool_result blocks
        //    that answer every outstanding tool_use id.
        if self.messages.is_empty() {
            let opener = build_opening_user_message(ctx);
            self.messages.push(json!({
                "role": "user",
                "content": opener,
            }));
        } else {
            let results = build_tool_results(ctx, &self.last_tool_uses, &mut self.correlations);
            self.messages.push(json!({
                "role": "user",
                "content": results,
            }));
        }

        // 1b. Trim internal history if it has grown past the ceiling. This
        //     protects long-running trajectories from context-window blow-up
        //     and degradation on 4.7's longer default reasoning horizons.
        self.trim_internal_history();

        // 2. Assemble the request with optional cache_control breakpoints.
        let tools_payload = build_tools_payload(ctx.tools, self.cache_prefix);
        let system = build_system_prompt_blocks(ctx.writ, self.cache_prefix);

        let mut req_body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": system,
            "tools": tools_payload,
            "messages": self.messages,
        });

        if let Some(t) = &self.thinking {
            // thinking budget must be strictly less than max_tokens
            let budget = t.budget_tokens.min(self.max_tokens.saturating_sub(1));
            req_body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget,
            });
        }

        // 3. POST with bounded retry on transient errors.
        let resp_json = self.post_with_retry(&req_body)?;

        // 4. Update usage counters (including cache stats when present).
        if let Some(usage) = resp_json.get("usage") {
            self.total_input_tokens += usage
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            self.total_output_tokens += usage
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            self.total_cache_read_tokens += usage
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            self.total_cache_write_tokens += usage
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }

        // 5. Append the raw assistant message to history for the next turn.
        //    Thinking blocks are preserved verbatim — the API rejects
        //    edited/stripped thinking content on subsequent turns.
        let content = resp_json
            .get("content")
            .cloned()
            .unwrap_or_else(|| json!([]));
        self.messages.push(json!({
            "role": "assistant",
            "content": content.clone(),
        }));

        // 6. Extract tool_use blocks as Intents + capture final text (if any).
        let mut new_tool_uses: Vec<String> = Vec::new();
        let mut new_signatures: Vec<String> = Vec::new();
        let mut intents: Vec<Intent> = Vec::new();
        let mut text_parts: Vec<String> = Vec::new();

        if let Some(blocks) = content.as_array() {
            for block in blocks {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            text_parts.push(t.to_string());
                        }
                    }
                    Some("thinking") | Some("redacted_thinking") => {
                        // Kept in `self.messages` verbatim; not surfaced as
                        // Intent or final_answer. Preserved so future turns
                        // pass API validation.
                    }
                    Some("tool_use") => {
                        let tool_use_id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| Error::Other("tool_use missing id".into()))?
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| Error::Other("tool_use missing name".into()))?
                            .to_string();
                        let input = block.get("input").cloned().unwrap_or(json!({}));

                        let mut nonce = [0u8; 16];
                        for (i, b) in tool_use_id.as_bytes().iter().take(16).enumerate() {
                            nonce[i] = *b;
                        }

                        let signature = format!(
                            "{}::{}",
                            name,
                            serde_json::to_string(&input).unwrap_or_default()
                        );
                        new_signatures.push(signature);

                        let intent = Intent::new(IntentBody {
                            parent_commit: None,
                            author: format!("anthropic:{}", self.model),
                            kind: IntentKind::Act,
                            target: name,
                            args: input,
                            rationale: text_parts.join("\n"),
                            nonce,
                        })?;

                        self.correlations.insert(intent.id, tool_use_id.clone());
                        new_tool_uses.push(tool_use_id);
                        intents.push(intent);
                    }
                    _ => {}
                }
            }
        }

        self.last_tool_uses = new_tool_uses;
        self.last_call_signatures = new_signatures;

        let stop_reason = resp_json
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Handle all 4.7 stop reasons:
        //   end_turn    — model finished naturally. Final answer if no tools.
        //   tool_use    — model wants to call tools; continue the loop.
        //   max_tokens  — output truncated; treat as error.
        //   stop_sequence — hit stop sequence (we don't set any); treat like end_turn.
        //   pause_turn  — long-running task paused; cognition must be re-stepped
        //                 with the same input to resume (we treat as continue).
        //   refusal     — model declined; terminate with an explanatory answer.
        let final_answer = match stop_reason {
            "end_turn" | "stop_sequence" if intents.is_empty() => {
                let txt = text_parts.join("\n");
                if txt.is_empty() {
                    None
                } else {
                    Some(txt)
                }
            }
            "max_tokens" if intents.is_empty() => {
                return Err(Error::Other(
                    "anthropic stop_reason=max_tokens with no tool_use: response truncated".into(),
                ));
            }
            "refusal" => {
                let txt = text_parts.join("\n");
                Some(if txt.is_empty() {
                    "(model refusal with no accompanying text)".into()
                } else {
                    format!("(model refused) {txt}")
                })
            }
            _ => None,
        };

        Ok(CognitionStep {
            intents,
            final_answer,
        })
    }
}

impl AnthropicCognition {
    /// POST `/v1/messages` with bounded retry on transient failures.
    ///
    /// Retry policy is shared with the streaming adapter via
    /// [`is_transient_status`] and [`backoff_delay_ms`], so both code paths
    /// treat 429 / 5xx / `overloaded_error` identically.
    fn post_with_retry(&self, req_body: &Value) -> Result<Value> {
        let mut attempt: u32 = 0;
        loop {
            let resp = self
                .client
                .post(API_URL)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(req_body)
                .send();

            // Transport failure: always transient up to the retry bound.
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    if attempt < self.max_retries {
                        std::thread::sleep(Duration::from_millis(backoff_delay_ms(attempt)));
                        attempt += 1;
                        continue;
                    }
                    return Err(Error::Other(format!("anthropic request failed: {e}")));
                }
            };

            let status = resp.status();
            // Quick transient HTTP status check before we parse the body.
            if !status.is_success()
                && is_transient_status(status.as_u16(), None)
                && attempt < self.max_retries
            {
                std::thread::sleep(Duration::from_millis(backoff_delay_ms(attempt)));
                attempt += 1;
                continue;
            }

            let resp_json: Value = resp
                .json()
                .map_err(|e| Error::Other(format!("anthropic response parse: {e}")))?;

            if !status.is_success() {
                // Body-level classifier — catches e.g. `overloaded_error` behind a 200.
                let error_type = resp_json
                    .get("error")
                    .and_then(|e| e.get("type"))
                    .and_then(|v| v.as_str());
                if is_transient_status(status.as_u16(), error_type) && attempt < self.max_retries {
                    std::thread::sleep(Duration::from_millis(backoff_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Err(Error::Other(format!(
                    "anthropic API error {status}: {resp_json}"
                )));
            }

            return Ok(resp_json);
        }
    }

    /// Compact older turns into a textual summary inserted at the top of the
    /// conversation. Preserves tool_use/tool_result pairing by always keeping
    /// the most recent pair intact.
    fn trim_internal_history(&mut self) {
        if self.messages.len() <= self.max_internal_messages {
            return;
        }

        // Keep the opening user message and the last N-1 messages. Summarize
        // the middle. We keep the opener because it carries the task.
        let keep_tail = self.max_internal_messages.saturating_sub(1);
        if self.messages.len() <= keep_tail + 1 {
            return;
        }

        let opener = self.messages.first().cloned();
        let drain_start = 1;
        let drain_end = self.messages.len() - keep_tail;

        // Safety: never split across a tool_use / tool_result boundary —
        // the next user message after our cut MUST be a pure user message
        // (not a tool_result), otherwise the API will complain.
        // Walk forward from drain_end until we hit a boundary that is safe.
        let mut safe_end = drain_end;
        while safe_end < self.messages.len() {
            if !is_tool_result_user_message(&self.messages[safe_end]) {
                break;
            }
            safe_end += 1;
        }
        if safe_end >= self.messages.len() {
            // Nothing safe to keep; abort trim.
            return;
        }

        let mut dropped = 0usize;
        let mut tool_calls_seen = 0usize;
        for i in drain_start..safe_end {
            dropped += 1;
            if let Some(arr) = self.messages[i].get("content").and_then(|c| c.as_array()) {
                for b in arr {
                    if b.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                        tool_calls_seen += 1;
                    }
                }
            }
        }

        let summary = format!(
            "[history compacted] {dropped} earlier turn(s) elided, containing \
             {tool_calls_seen} tool call(s). Recent outcomes remain below.",
        );

        let mut compacted: Vec<Value> = Vec::with_capacity(self.messages.len() - dropped + 1);
        if let Some(o) = opener {
            compacted.push(o);
        }
        compacted.push(json!({
            "role": "user",
            "content": [{ "type": "text", "text": summary }],
        }));
        compacted.extend(self.messages.drain(safe_end..));

        self.messages = compacted;
    }
}

fn is_tool_result_user_message(msg: &Value) -> bool {
    if msg.get("role").and_then(|v| v.as_str()) != Some("user") {
        return false;
    }
    if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
        arr.iter()
            .any(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
    } else {
        false
    }
}

// ---------- helpers ------------------------------------------------------

fn build_system_prompt_text(writ: &Writ) -> String {
    let mut scopes: Vec<String> = writ
        .body
        .tool_scopes
        .iter()
        .map(|p| p.tool.clone())
        .collect();
    scopes.sort();
    let scopes_str = scopes.join(", ");
    let b = &writ.body.budget;
    format!(
        "You are a cognition process operating inside the Thymos runtime.

You do not take actions directly. You emit tool_use blocks that describe \
PROPOSED actions. The runtime compiles each proposal, evaluates policy \
against a bounded Capability Writ, and either commits the effect or rejects \
the proposal. The runtime is the sole source of truth.

Constraints you MUST respect:
  * You may only call tools that match your Writ scope. Current scope: [{scopes_str}].
  * Your budget is bounded: {tokens} tokens, {calls} tool calls, ~{usd} USD (in millicents), {time}ms wall-clock.
  * If a proposal is rejected, you will see the typed reason as a tool_result \
with is_error=true. Adjust your next proposal. Do not retry an identical \
call (same tool + same args) after a rejection — change the arguments or \
choose a different tool.
  * When you have achieved the task, reply with plain text and no tool_use \
blocks. That is how you signal completion. Do not emit speculative tool \
calls after the task is done.
  * Each proposal should include a brief rationale (your text content next to \
the tool_use) so the ledger records why you proposed it. Keep rationales \
concise — one sentence of intent, not a plan dump.",
        scopes_str = scopes_str,
        tokens = b.tokens,
        calls = b.tool_calls,
        usd = b.usd_millicents,
        time = b.wall_clock_ms,
    )
}

/// Build the `system` field as a block array. When `cache_prefix` is true,
/// the (stable) system prompt is marked with `cache_control: ephemeral`,
/// which lets Anthropic serve the prefix from cache on turn N+1.
fn build_system_prompt_blocks(writ: &Writ, cache_prefix: bool) -> Value {
    let text = build_system_prompt_text(writ);
    if cache_prefix {
        json!([{
            "type": "text",
            "text": text,
            "cache_control": { "type": "ephemeral" }
        }])
    } else {
        json!([{ "type": "text", "text": text }])
    }
}

fn build_opening_user_message(ctx: &CognitionContext<'_>) -> Value {
    let world_summary = summarize_world(ctx);
    json!([
        {
            "type": "text",
            "text": format!(
                "Task: {}\n\nCurrent world state:\n{}\n\nProceed.",
                ctx.task, world_summary
            )
        }
    ])
}

fn summarize_world(ctx: &CognitionContext<'_>) -> String {
    if ctx.world.resources.is_empty() {
        return "(empty)".into();
    }
    let mut lines = Vec::new();
    for (key, state) in &ctx.world.resources {
        let v = serde_json::to_string(&state.value).unwrap_or_else(|_| "<unprintable>".into());
        lines.push(format!(
            "  {}:{} v{} = {}",
            key.kind, key.id, state.version, v
        ));
    }
    lines.join("\n")
}

fn build_tools_payload(tools: &ToolRegistry, cache_prefix: bool) -> Vec<Value> {
    let names: Vec<String> = tools.names().map(|s| s.to_string()).collect();
    let mut out = Vec::with_capacity(names.len());
    let last_idx = names.len().saturating_sub(1);
    for (i, name) in names.iter().enumerate() {
        if let Ok(tool) = tools.get(name) {
            let mut entry = json!({
                "name": tool.meta().name,
                "description": tool.description(),
                "input_schema": tool.input_schema(),
            });
            // Attach the cache breakpoint to the last tool, so the entire
            // tools block (a stable prefix across turns) is cacheable.
            if cache_prefix && i == last_idx {
                entry["cache_control"] = json!({ "type": "ephemeral" });
            }
            out.push(entry);
        }
    }
    out
}

fn build_tool_results(
    ctx: &CognitionContext<'_>,
    expected_tool_use_ids: &[String],
    correlations: &mut HashMap<IntentId, String>,
) -> Vec<Value> {
    // Index history by Intent id.
    let mut outcomes: HashMap<IntentId, HistoryOutcome> = HashMap::new();
    for item in &ctx.since_last {
        match item {
            HistoryItem::Committed {
                intent,
                observation,
            } => {
                outcomes.insert(
                    intent.id,
                    HistoryOutcome::Committed(observation.output.clone()),
                );
            }
            HistoryItem::Rejected { intent, reason } => {
                outcomes.insert(intent.id, HistoryOutcome::Rejected(reason.clone()));
            }
            HistoryItem::Failed { intent, error } => {
                outcomes.insert(intent.id, HistoryOutcome::Failed(error.clone()));
            }
        }
    }

    // For every tool_use id the provider emitted last turn, produce a tool_result.
    let mut results = Vec::with_capacity(expected_tool_use_ids.len());
    for tool_use_id in expected_tool_use_ids {
        // Find which Intent id maps to this tool_use_id.
        let matching_intent_id = correlations
            .iter()
            .find(|(_, v)| *v == tool_use_id)
            .map(|(k, _)| *k);

        let (content, is_error) = match matching_intent_id.and_then(|id| outcomes.remove(&id)) {
            Some(HistoryOutcome::Committed(output)) => (
                format!(
                    "Committed. Observation:\n{}",
                    serde_json::to_string_pretty(&output).unwrap_or_default()
                ),
                false,
            ),
            Some(HistoryOutcome::Rejected(reason)) => {
                (format!("Rejected by runtime. Reason: {reason:?}"), true)
            }
            Some(HistoryOutcome::Failed(error)) => (
                format!("Execution failed after staging. Error: {error}"),
                true,
            ),
            None => (
                "Proposal was not executed this turn (runtime deferred or suspended).".into(),
                true,
            ),
        };

        results.push(json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }));
    }

    // Drop correlations for every tool_use id we just answered.
    correlations.retain(|_, v| !expected_tool_use_ids.contains(v));
    results
}

enum HistoryOutcome {
    Committed(serde_json::Value),
    Rejected(RejectionReason),
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_model_alias_maps_short_names() {
        assert_eq!(resolve_model_alias("opus".into()), "claude-opus-4-7");
        assert_eq!(resolve_model_alias("opus-4.7".into()), "claude-opus-4-7");
        assert_eq!(resolve_model_alias("opus-4.6".into()), "claude-opus-4-6");
        assert_eq!(resolve_model_alias("sonnet".into()), "claude-sonnet-4-6");
        assert_eq!(resolve_model_alias("haiku".into()), "claude-haiku-4-5");
    }

    #[test]
    fn resolve_model_alias_passes_full_ids_through() {
        assert_eq!(
            resolve_model_alias("claude-opus-4-7".into()),
            "claude-opus-4-7"
        );
        assert_eq!(
            resolve_model_alias("claude-3-5-sonnet-20241022".into()),
            "claude-3-5-sonnet-20241022"
        );
    }

    #[test]
    fn is_transient_status_flags_429_and_5xx() {
        assert!(is_transient_status(429, None));
        assert!(is_transient_status(500, None));
        assert!(is_transient_status(502, None));
        assert!(is_transient_status(503, None));
        assert!(is_transient_status(529, None));
        assert!(!is_transient_status(400, None));
        assert!(!is_transient_status(401, None));
        assert!(!is_transient_status(403, None));
        assert!(!is_transient_status(404, None));
        assert!(!is_transient_status(422, None));
    }

    #[test]
    fn is_transient_status_flags_overloaded_error_body() {
        assert!(is_transient_status(500, Some("overloaded_error")));
        assert!(is_transient_status(400, Some("overloaded_error")));
        assert!(is_transient_status(400, Some("api_error")));
        assert!(!is_transient_status(400, Some("invalid_request_error")));
        assert!(!is_transient_status(400, Some("permission_error")));
    }

    #[test]
    fn backoff_delay_is_exponential_and_clamped() {
        assert_eq!(backoff_delay_ms(0), 500);
        assert_eq!(backoff_delay_ms(1), 1000);
        assert_eq!(backoff_delay_ms(2), 2000);
        assert_eq!(backoff_delay_ms(3), 4000);
        assert_eq!(backoff_delay_ms(6), 32_000);
        // Clamp at attempt=6 to prevent u64 overflow on pathological attempts.
        assert_eq!(backoff_delay_ms(100), 32_000);
    }

    #[test]
    fn with_max_retries_is_observable() {
        let c = AnthropicCognition::with_api_key("test".into()).unwrap();
        assert_eq!(c.max_retries(), DEFAULT_MAX_RETRIES);
        let c = c.with_max_retries(7);
        assert_eq!(c.max_retries(), 7);
        let c = c.with_max_retries(0);
        assert_eq!(c.max_retries(), 0);
    }

    #[test]
    fn is_tool_result_user_message_discriminates() {
        let tool_result = json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "tu_1",
                "content": "ok",
                "is_error": false,
            }],
        });
        let plain_user = json!({
            "role": "user",
            "content": [{ "type": "text", "text": "hello" }],
        });
        let assistant = json!({
            "role": "assistant",
            "content": [{ "type": "text", "text": "hi" }],
        });
        assert!(is_tool_result_user_message(&tool_result));
        assert!(!is_tool_result_user_message(&plain_user));
        assert!(!is_tool_result_user_message(&assistant));
    }
}
