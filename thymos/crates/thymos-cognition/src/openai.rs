//! OpenAI Chat Completions API cognition adapter.
//!
//! Mirrors the Anthropic adapter pattern: the model emits `tool_calls` in
//! assistant messages, which this adapter translates into `Intent`s.
//! Committed/rejected outcomes are fed back as `tool` messages on the next turn.
//!
//! Environment:
//!   * `OPENAI_API_KEY` — required.
//!   * `OPENAI_MODEL` — optional, defaults to `gpt-4o`.
//!   * `OPENAI_BASE_URL` — optional, for local/custom endpoints (e.g. Ollama, vLLM).

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

pub const DEFAULT_MODEL: &str = "gpt-4o";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Wire format for tool calls.
///
/// Cloud OpenAI / LM Studio / vLLM with function-calling templates use
/// [`ToolProtocol::Native`] (the `tools` request parameter + `tool_calls` on
/// the assistant message). Many local models (older Ollama builds, raw
/// llama.cpp, plain Mistral checkpoints) don't honor that parameter; for those
/// we fall back to [`ToolProtocol::JsonBlock`], which describes tools in the
/// system prompt and parses fenced JSON blocks from the model's text output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolProtocol {
    /// OpenAI native function calling (`tools` + `tool_calls`).
    Native,
    /// JSON code blocks embedded in the assistant's text content. Each block
    /// is a single JSON object of the form `{"tool": "name", "args": {...}}`.
    /// Multiple blocks per turn are supported. Both fenced (```json ... ```)
    /// and bare object forms are accepted by the parser.
    JsonBlock,
}

pub struct OpenAiCognition {
    client: reqwest::blocking::Client,
    api_key: String,
    base_url: String,
    model: String,
    max_tokens: u32,
    tool_protocol: ToolProtocol,
    /// Accumulated messages (OpenAI chat format).
    messages: Vec<Value>,
    /// tool_call_id → Intent ID correlation.
    correlations: HashMap<IntentId, String>,
    /// tool_call_ids from the last assistant message that need responses.
    last_tool_call_ids: Vec<String>,
    /// Synthetic id → Intent for the JsonBlock protocol (no native ids).
    last_json_block_ids: Vec<String>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

impl OpenAiCognition {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| Error::Other("OPENAI_API_KEY is not set".into()))?;
        let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into());
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        Self::new(api_key, base_url, model)
    }

    pub fn new(api_key: String, base_url: String, model: String) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| Error::Other(format!("reqwest client build: {e}")))?;
        Ok(OpenAiCognition {
            client,
            api_key,
            base_url,
            model,
            max_tokens: 4096,
            tool_protocol: ToolProtocol::Native,
            messages: Vec::new(),
            correlations: HashMap::new(),
            last_tool_call_ids: Vec::new(),
            last_json_block_ids: Vec::new(),
            total_input_tokens: 0,
            total_output_tokens: 0,
        })
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Switch to the JSON-block tool protocol for local models without native
    /// function calling. See [`ToolProtocol`].
    pub fn with_tool_protocol(mut self, protocol: ToolProtocol) -> Self {
        self.tool_protocol = protocol;
        self
    }

    pub fn tool_protocol(&self) -> ToolProtocol {
        self.tool_protocol
    }
}

impl Cognition for OpenAiCognition {
    fn step(&mut self, ctx: &CognitionContext<'_>) -> Result<CognitionStep> {
        // 1. First turn: seed with system + user message.
        //    Subsequent turns: send tool results for all outstanding tool_call_ids.
        if self.messages.is_empty() {
            let system_prompt = match self.tool_protocol {
                ToolProtocol::Native => build_system_prompt(ctx.writ),
                ToolProtocol::JsonBlock => build_system_prompt_jsonblock(ctx.writ, ctx.tools),
            };
            self.messages.push(json!({
                "role": "system",
                "content": system_prompt,
            }));
            let user_content = build_opening_user_message(ctx);
            self.messages.push(json!({
                "role": "user",
                "content": user_content,
            }));
        } else {
            match self.tool_protocol {
                ToolProtocol::Native => {
                    let tool_messages =
                        build_tool_results(ctx, &self.last_tool_call_ids, &mut self.correlations);
                    self.messages.extend(tool_messages);
                }
                ToolProtocol::JsonBlock => {
                    let user_summary = build_tool_results_jsonblock(
                        ctx,
                        &self.last_json_block_ids,
                        &mut self.correlations,
                    );
                    self.messages.push(json!({
                        "role": "user",
                        "content": user_summary,
                    }));
                }
            }
        }

        // 2. Build request.
        let mut req_body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": self.messages,
        });
        if matches!(self.tool_protocol, ToolProtocol::Native) {
            let tools_payload = build_tools_payload(ctx.tools);
            if !tools_payload.is_empty() {
                req_body["tools"] = json!(tools_payload);
            }
        }

        // 3. POST.
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .map_err(|e| Error::Other(format!("openai request failed: {e}")))?;

        let status = resp.status();
        let resp_json: Value = resp
            .json()
            .map_err(|e| Error::Other(format!("openai response parse: {e}")))?;
        if !status.is_success() {
            return Err(Error::Other(format!(
                "openai API error {status}: {resp_json}"
            )));
        }

        // 4. Usage — accumulate totals and capture this turn's usage.
        let mut step_usage = crate::CognitionUsage::default();
        if let Some(usage) = resp_json.get("usage") {
            let input = usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output = usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            self.total_input_tokens += input;
            self.total_output_tokens += output;
            step_usage.input_tokens = input;
            step_usage.output_tokens = output;
        }

        // 5. Parse the first choice.
        let choice = resp_json
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| Error::Other("openai: no choices in response".into()))?;

        let message = choice
            .get("message")
            .ok_or_else(|| Error::Other("openai: no message in choice".into()))?;

        // Append assistant message to history.
        self.messages.push(message.clone());

        let finish_reason = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // 6. Extract tool_calls as Intents.
        let mut intents: Vec<Intent> = Vec::new();
        let mut new_tool_call_ids: Vec<String> = Vec::new();
        let mut new_json_block_ids: Vec<String> = Vec::new();
        let text_content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match self.tool_protocol {
            ToolProtocol::Native => {
                if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        let tc_id = tc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| Error::Other("tool_call missing id".into()))?
                            .to_string();
                        let function = tc
                            .get("function")
                            .ok_or_else(|| Error::Other("tool_call missing function".into()))?;
                        let name = function
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| Error::Other("tool_call function missing name".into()))?
                            .to_string();
                        let args_str = function
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let args: Value = serde_json::from_str(args_str)
                            .map_err(|e| Error::Other(format!("tool_call args parse: {e}")))?;

                        let mut nonce = [0u8; 16];
                        for (i, b) in tc_id.as_bytes().iter().take(16).enumerate() {
                            nonce[i] = *b;
                        }

                        let intent = Intent::new(IntentBody {
                            parent_commit: None,
                            author: format!("openai:{}", self.model),
                            kind: IntentKind::Act,
                            target: name,
                            args,
                            rationale: text_content.clone(),
                            nonce,
                        })?;

                        self.correlations.insert(intent.id, tc_id.clone());
                        new_tool_call_ids.push(tc_id);
                        intents.push(intent);
                    }
                }
            }
            ToolProtocol::JsonBlock => {
                let parsed = parse_json_blocks(&text_content);
                for (idx, block) in parsed.iter().enumerate() {
                    let synth_id = format!("jb_{}_{}", ctx.step_index, idx);
                    let mut nonce = [0u8; 16];
                    for (i, b) in synth_id.as_bytes().iter().take(16).enumerate() {
                        nonce[i] = *b;
                    }
                    let intent = Intent::new(IntentBody {
                        parent_commit: None,
                        author: format!("openai:{}", self.model),
                        kind: IntentKind::Act,
                        target: block.tool.clone(),
                        args: block.args.clone(),
                        rationale: text_content.clone(),
                        nonce,
                    })?;
                    self.correlations.insert(intent.id, synth_id.clone());
                    new_json_block_ids.push(synth_id);
                    intents.push(intent);
                }
            }
        }

        self.last_tool_call_ids = new_tool_call_ids;
        self.last_json_block_ids = new_json_block_ids;

        let final_answer =
            if intents.is_empty() && (finish_reason == "stop" || finish_reason.is_empty()) {
                if text_content.is_empty() {
                    None
                } else {
                    Some(text_content)
                }
            } else {
                None
            };

        Ok(CognitionStep {
            intents,
            final_answer,
            usage: step_usage,
        })
    }
}

// ---------- helpers ----------------------------------------------------------

fn build_system_prompt(writ: &Writ) -> String {
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
        "You are a cognition process operating inside the Thymos runtime.\n\
         \n\
         You do not take actions directly. You emit function calls that describe \
         PROPOSED actions. The runtime evaluates policy against a bounded Capability \
         Writ, and either commits the effect or rejects the proposal.\n\
         \n\
         Constraints:\n\
         - You may only call tools matching your Writ scope: [{scopes_str}].\n\
         - Budget: {tokens} tokens, {calls} tool calls, ~{usd} USD (millicents), {time}ms wall-clock.\n\
         - If a proposal is rejected, you will see the reason in the tool response. Adjust accordingly.\n\
         - When done, reply with plain text and no function calls to signal completion.",
        tokens = b.tokens,
        calls = b.tool_calls,
        usd = b.usd_millicents,
        time = b.wall_clock_ms,
    )
}

fn build_opening_user_message(ctx: &CognitionContext<'_>) -> String {
    let world_summary = if ctx.world.resources.is_empty() {
        "(empty)".into()
    } else {
        let mut lines = Vec::new();
        for (key, state) in &ctx.world.resources {
            let v = serde_json::to_string(&state.value).unwrap_or_else(|_| "<unprintable>".into());
            lines.push(format!(
                "  {}:{} v{} = {}",
                key.kind, key.id, state.version, v
            ));
        }
        lines.join("\n")
    };
    format!(
        "Task: {}\n\nCurrent world state:\n{}\n\nProceed.",
        ctx.task, world_summary
    )
}

fn build_tools_payload(tools: &ToolRegistry) -> Vec<Value> {
    let mut out = Vec::new();
    for name in tools.names() {
        if let Ok(tool) = tools.get(name) {
            out.push(json!({
                "type": "function",
                "function": {
                    "name": tool.meta().name,
                    "description": tool.description(),
                    "parameters": tool.input_schema(),
                }
            }));
        }
    }
    out
}

fn build_tool_results(
    ctx: &CognitionContext<'_>,
    expected_tool_call_ids: &[String],
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

    // For every tool_call_id, produce a tool message.
    let mut messages = Vec::with_capacity(expected_tool_call_ids.len());
    for tc_id in expected_tool_call_ids {
        let matching_intent_id = correlations
            .iter()
            .find(|(_, v)| *v == tc_id)
            .map(|(k, _)| *k);

        let content = match matching_intent_id.and_then(|id| outcomes.remove(&id)) {
            Some(HistoryOutcome::Committed(output)) => {
                format!(
                    "Committed. Observation:\n{}",
                    serde_json::to_string_pretty(&output).unwrap_or_default()
                )
            }
            Some(HistoryOutcome::Rejected(reason)) => {
                format!("Rejected by runtime. Reason: {reason:?}")
            }
            Some(HistoryOutcome::Failed(error)) => {
                format!("Execution failed after staging. Error: {error}")
            }
            None => "Proposal was not executed this turn (runtime deferred or suspended).".into(),
        };

        messages.push(json!({
            "role": "tool",
            "tool_call_id": tc_id,
            "content": content,
        }));
    }

    correlations.retain(|_, v| !expected_tool_call_ids.contains(v));
    messages
}

enum HistoryOutcome {
    Committed(serde_json::Value),
    Rejected(RejectionReason),
    Failed(String),
}

// ---------- JSON-block tool protocol (local-model fallback) ------------------

/// One parsed `{"tool": ..., "args": ...}` directive from the model's reply.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JsonBlockCall {
    pub tool: String,
    pub args: Value,
}

/// Build the system prompt for the JsonBlock tool protocol. Tools and their
/// JSON Schemas are inlined in the system message because we cannot pass them
/// as a structured `tools` field (the model wouldn't honor it).
fn build_system_prompt_jsonblock(writ: &Writ, tools: &ToolRegistry) -> String {
    let mut scopes: Vec<String> = writ
        .body
        .tool_scopes
        .iter()
        .map(|p| p.tool.clone())
        .collect();
    scopes.sort();
    let scopes_str = scopes.join(", ");
    let b = &writ.body.budget;

    let mut tool_lines = Vec::new();
    for name in tools.names() {
        if let Ok(t) = tools.get(name) {
            let schema_str =
                serde_json::to_string(&t.input_schema()).unwrap_or_else(|_| "{}".into());
            tool_lines.push(format!(
                "  - {name}: {desc}\n      args schema: {schema}",
                name = t.meta().name,
                desc = t.description(),
                schema = schema_str,
            ));
        }
    }
    let tools_block = tool_lines.join("\n");

    format!(
        "You are a cognition process operating inside the Thymos runtime.\n\
         \n\
         You do not take actions directly. To call a tool, emit one or more \
         fenced JSON blocks in your reply. Each block must be a single JSON \
         object with this exact shape:\n\
         \n\
         ```json\n\
         {{\"tool\": \"<tool_name>\", \"args\": {{...}}}}\n\
         ```\n\
         \n\
         You may emit multiple JSON blocks in one turn — each becomes a \
         proposed action. Plain prose around the blocks is allowed but only \
         the JSON blocks are executed. When you are done, reply with text and \
         no JSON blocks.\n\
         \n\
         Constraints:\n\
         - Writ scope: [{scopes_str}].\n\
         - Budget: {tokens} tokens, {calls} tool calls, ~{usd} USD (millicents), {time}ms wall-clock.\n\
         - Rejected proposals come back as a numbered list. Do not retry an \
         identical (tool, args) pair after a rejection.\n\
         \n\
         Available tools:\n\
         {tools_block}",
        tokens = b.tokens,
        calls = b.tool_calls,
        usd = b.usd_millicents,
        time = b.wall_clock_ms,
    )
}

/// Build a single `user` message summarizing the previous turn's outcomes.
/// JsonBlock has no native tool-message role, so we deliver outcomes as
/// regular user text the model can read.
fn build_tool_results_jsonblock(
    ctx: &CognitionContext<'_>,
    expected_block_ids: &[String],
    correlations: &mut HashMap<IntentId, String>,
) -> String {
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

    let mut lines = Vec::with_capacity(expected_block_ids.len() + 1);
    lines.push("Previous turn results:".to_string());
    for (idx, block_id) in expected_block_ids.iter().enumerate() {
        let matching_intent_id = correlations
            .iter()
            .find(|(_, v)| *v == block_id)
            .map(|(k, _)| *k);

        let line = match matching_intent_id.and_then(|id| outcomes.remove(&id)) {
            Some(HistoryOutcome::Committed(output)) => {
                let pretty = serde_json::to_string_pretty(&output).unwrap_or_else(|_| "<>".into());
                format!("  [{n}] committed → {pretty}", n = idx + 1)
            }
            Some(HistoryOutcome::Rejected(reason)) => {
                format!("  [{n}] REJECTED → {reason:?}", n = idx + 1)
            }
            Some(HistoryOutcome::Failed(error)) => {
                format!("  [{n}] EXECUTION FAILED → {error}", n = idx + 1)
            }
            None => format!("  [{n}] not executed", n = idx + 1),
        };
        lines.push(line);
    }
    lines.push(
        "Continue. Emit JSON blocks for the next actions, or reply with plain text to finish."
            .into(),
    );
    correlations.retain(|_, v| !expected_block_ids.contains(v));
    lines.join("\n")
}

/// Extract every `{"tool": ..., "args": ...}` JSON object from the model's
/// reply. Accepts both fenced ```json blocks and bare JSON objects.
/// Tolerates surrounding prose and malformed siblings.
pub(crate) fn parse_json_blocks(text: &str) -> Vec<JsonBlockCall> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        // Find the matching closing brace, respecting strings + escapes.
        let mut depth: i32 = 0;
        let mut in_str = false;
        let mut esc = false;
        let mut end = i;
        let mut found = false;
        for (j, &b) in bytes.iter().enumerate().skip(i) {
            if esc {
                esc = false;
                continue;
            }
            if b == b'\\' && in_str {
                esc = true;
                continue;
            }
            if b == b'"' {
                in_str = !in_str;
                continue;
            }
            if in_str {
                continue;
            }
            if b == b'{' {
                depth += 1;
            } else if b == b'}' {
                depth -= 1;
                if depth == 0 {
                    end = j + 1;
                    found = true;
                    break;
                }
            }
        }
        if !found {
            break;
        }
        let candidate = &text[i..end];
        if let Ok(v) = serde_json::from_str::<Value>(candidate) {
            let tool = v.get("tool").and_then(|t| t.as_str()).map(str::to_string);
            if let Some(tool) = tool {
                let args = v.get("args").cloned().unwrap_or(json!({}));
                out.push(JsonBlockCall { tool, args });
            }
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_blocks_extracts_fenced_blocks() {
        let text = "Here we go.\n\
                    ```json\n\
                    {\"tool\": \"echo\", \"args\": {\"msg\": \"hi\"}}\n\
                    ```\n\
                    Then another:\n\
                    ```json\n\
                    {\"tool\": \"add\", \"args\": {\"a\": 1, \"b\": 2}}\n\
                    ```\n";
        let blocks = parse_json_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].tool, "echo");
        assert_eq!(blocks[0].args["msg"], "hi");
        assert_eq!(blocks[1].tool, "add");
        assert_eq!(blocks[1].args["a"], 1);
    }

    #[test]
    fn parse_json_blocks_ignores_non_tool_objects() {
        let text = "{\"unrelated\": true}\n{\"tool\":\"x\",\"args\":{}}";
        let blocks = parse_json_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].tool, "x");
    }

    #[test]
    fn parse_json_blocks_handles_strings_with_braces() {
        let text = r#"{"tool":"shell","args":{"cmd":"echo \"} hi {\""}}"#;
        let blocks = parse_json_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].tool, "shell");
    }

    #[test]
    fn parse_json_blocks_returns_empty_on_plain_text() {
        let text = "Done. No more actions needed.";
        let blocks = parse_json_blocks(text);
        assert!(blocks.is_empty());
    }
}
