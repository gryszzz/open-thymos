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

/// Parse a `Retry-After` header into milliseconds. The header is delta-seconds
/// for rate limits (we ignore the rare HTTP-date form, which returns `None`).
fn parse_retry_after_header(v: &str) -> Option<u64> {
    v.trim().parse::<f64>().ok().map(|s| (s * 1000.0) as u64)
}

/// Some providers (e.g. Groq) put the precise wait only in the JSON error
/// body: `"Please try again in 9.62s"` (per-minute limit) or
/// `"Please try again in 1h16m43.392s"` (per-day limit). Parse the compound
/// `<h>h<m>m<s>s` / `<s>s` form into milliseconds. This is the *accurate*
/// reset; the `Retry-After` header is often a coarse generic value.
fn parse_retry_after_body(body: &str) -> Option<u64> {
    const MARKER: &str = "try again in ";
    let start = body.find(MARKER)? + MARKER.len();
    let dur: String = body[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || matches!(c, 'h' | 'm' | 's'))
        .collect();
    if dur.is_empty() {
        return None;
    }
    let mut total_ms = 0f64;
    let mut num = String::new();
    for c in dur.chars() {
        match c {
            'h' => { total_ms += num.parse::<f64>().unwrap_or(0.0) * 3_600_000.0; num.clear(); }
            'm' => { total_ms += num.parse::<f64>().unwrap_or(0.0) * 60_000.0; num.clear(); }
            's' => { total_ms += num.parse::<f64>().unwrap_or(0.0) * 1_000.0; num.clear(); }
            _ => num.push(c),
        }
    }
    // Bare number (no unit) → seconds, matching the simple "9.62" case.
    if !num.is_empty() {
        total_ms += num.parse::<f64>().unwrap_or(0.0) * 1_000.0;
    }
    (total_ms > 0.0).then_some(total_ms as u64)
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
            let tools_payload = build_tools_payload(ctx.tools, ctx.writ);
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
            .map_err(|e| {
                // reqwest's Display omits the cause ("error sending request for
                // url (…)"); name the failure class so downstream layers can
                // produce a useful message.
                let kind = if e.is_timeout() {
                    " (timeout)"
                } else if e.is_connect() {
                    " (connection failed)"
                } else {
                    ""
                };
                Error::Other(format!("openai request failed{kind}: {e}"))
            })?;

        let status = resp.status();
        // Capture the server's retry hint *before* the body is consumed. On a
        // 429 the budget (e.g. Groq's tokens-per-minute) resets on the
        // provider's wall clock; a fixed local backoff can't clear it, so we
        // surface the server's own wait for the runtime to honor.
        let retry_after_header = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let resp_json: Value = resp
            .json()
            .map_err(|e| Error::Other(format!("openai response parse: {e}")))?;
        if !status.is_success() {
            // Groq's `tool_use_failed`: the model emitted a malformed tool
            // call and the API rejected the whole generation with a 400 —
            // but the text the model was trying to produce is preserved in
            // `error.failed_generation`. Recover it as a plain assistant
            // turn instead of failing the run: no tool executes (a call that
            // can't parse never reaches the runtime), the model just "spoke".
            if status.as_u16() == 400 {
                if let Some(failed) = resp_json
                    .pointer("/error/failed_generation")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    let text = failed.to_string();
                    self.messages
                        .push(json!({ "role": "assistant", "content": text }));
                    return Ok(CognitionStep {
                        intents: vec![],
                        final_answer: Some(text),
                        usage: crate::CognitionUsage::default(),
                    });
                }
            }
            // Prefer the body's precise reset ("try again in 1h16m43s" /
            // "9.62s") — it reflects the actual limit; the Retry-After header is
            // often a coarse generic value (e.g. a flat 60s) that hides a much
            // longer daily-limit reset. Fall back to the header when no body
            // hint is present.
            let hint_ms = parse_retry_after_body(&resp_json.to_string())
                .or_else(|| retry_after_header.as_deref().and_then(parse_retry_after_header));
            let suffix = hint_ms
                .map(|ms| format!(" [retry_after_ms={ms}]"))
                .unwrap_or_default();
            return Err(Error::Other(format!(
                "openai API error {status}: {resp_json}{suffix}"
            )));
        }

        // 4-6. Shared parse: usage, assistant message, tool calls → step.
        self.finish_from_response(&resp_json, ctx)
    }

    fn step_streamed(
        &mut self,
        ctx: &CognitionContext<'_>,
        on_token: &mut dyn FnMut(&str),
    ) -> Option<Result<CognitionStep>> {
        match self.stream_request(ctx, on_token) {
            Ok(resp_json) => Some(self.finish_from_response(&resp_json, ctx)),
            // Couldn't establish/parse the stream → signal fallback to sync.
            Err(_) => None,
        }
    }
}

impl OpenAiCognition {
    /// Shared response → CognitionStep parse, used by both the sync `step` and
    /// the streaming path so they stay identical in behavior.
    fn finish_from_response(
        &mut self,
        resp_json: &Value,
        ctx: &CognitionContext<'_>,
    ) -> Result<CognitionStep> {
        // Usage — accumulate totals and capture this turn's usage.
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

    /// Stream a chat completion (SSE), invoking `on_token` for each text delta,
    /// and return a synthesized non-streaming-shaped response JSON so the shared
    /// `finish_from_response` parser handles it identically to a normal turn.
    /// Errors (including any setup failure) let the caller fall back to sync.
    fn stream_request(
        &self,
        ctx: &CognitionContext<'_>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Value> {
        use std::io::BufRead;

        let mut req_body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": self.messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if matches!(self.tool_protocol, ToolProtocol::Native) {
            let tools_payload = build_tools_payload(ctx.tools, ctx.writ);
            if !tools_payload.is_empty() {
                req_body["tools"] = json!(tools_payload);
            }
        }

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .map_err(|e| Error::Other(format!("openai stream request failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Other(format!("openai stream HTTP {}", resp.status())));
        }

        let mut content = String::new();
        let mut finish_reason = String::new();
        let mut usage: Option<Value> = None;
        // tool_calls accumulated by index: (id, name, arguments-string).
        let mut tcs: Vec<(String, String, String)> = Vec::new();

        // Hard cap so a runaway/never-terminating stream can't pin memory —
        // generous (~4MB of streamed text) but bounded; tripping it aborts the
        // stream and the caller falls back to the sync path.
        const MAX_STREAM_BYTES: usize = 4 * 1024 * 1024;
        let mut seen_bytes = 0usize;
        let reader = std::io::BufReader::new(resp);
        for line in reader.lines() {
            let line = line.map_err(|e| Error::Other(format!("stream read: {e}")))?;
            seen_bytes += line.len();
            if seen_bytes > MAX_STREAM_BYTES {
                return Err(Error::Other("openai stream exceeded size cap".into()));
            }
            let data = match line.strip_prefix("data:") {
                Some(d) => d.trim(),
                None => continue,
            };
            if data == "[DONE]" {
                break;
            }
            let Ok(chunk) = serde_json::from_str::<Value>(data) else { continue };
            if let Some(u) = chunk.get("usage") {
                if !u.is_null() {
                    usage = Some(u.clone());
                }
            }
            let Some(choice) = chunk.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first())
            else { continue };
            if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                finish_reason = fr.to_string();
            }
            let Some(delta) = choice.get("delta") else { continue };
            if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    content.push_str(text);
                    on_token(text);
                }
            }
            if let Some(arr) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in arr {
                    let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    while tcs.len() <= idx {
                        tcs.push((String::new(), String::new(), String::new()));
                    }
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() { tcs[idx].0 = id.to_string(); }
                    }
                    if let Some(f) = tc.get("function") {
                        if let Some(n) = f.get("name").and_then(|v| v.as_str()) {
                            if !n.is_empty() { tcs[idx].1 = n.to_string(); }
                        }
                        if let Some(a) = f.get("arguments").and_then(|v| v.as_str()) {
                            tcs[idx].2.push_str(a);
                        }
                    }
                }
            }
        }

        // Synthesize the standard response shape for the shared parser.
        let mut message = json!({ "role": "assistant", "content": content });
        if !tcs.is_empty() {
            message["tool_calls"] = json!(tcs
                .into_iter()
                .filter(|(_, name, _)| !name.is_empty())
                .map(|(id, name, args)| json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": args },
                }))
                .collect::<Vec<_>>());
        }
        let mut out = json!({
            "choices": [{
                "message": message,
                "finish_reason": if finish_reason.is_empty() { "stop".into() } else { finish_reason },
            }],
        });
        if let Some(u) = usage {
            out["usage"] = u;
        }
        Ok(out)
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
        "You are OpenThymos — a governed AI agent that can help with almost \
         anything: answering questions, reasoning through problems, writing and \
         explaining code, inspecting and editing files, running commands, and \
         fetching data. Be genuinely useful, accurate, and proactive. Write \
         naturally and directly, like a sharp colleague; when you're unsure, \
         say so rather than guessing.\n\
         \n\
         What makes you different is HOW you act, not how much you can do. The \
         creed of this runtime: cognition proposes, the runtime governs, the \
         ledger records. You are the cognition. When a task needs a real-world \
         effect, you don't perform it directly — you emit a function call \
         describing the PROPOSED action. The runtime checks it against a signed \
         Capability Writ (what you're authorized to do), records the decision, \
         and either commits the effect or rejects it — and every committed \
         action is appended to an auditable, replayable ledger. Treat this as a \
         strength: it lets the user trust you with real power. If a proposal is \
         rejected, explain plainly what was blocked and what would unblock it \
         (usually a broader grant) — never pretend it succeeded.\n\
         \n\
         Not every message needs a tool. If the user is greeting you, making \
         small talk, or asking something you can answer from your own knowledge, \
         just reply with plain text and no function calls. Only propose a tool \
         call when the task genuinely requires an external effect (reading or \
         writing files, running a command, fetching data). Never call a tool \
         merely to acknowledge a message.\n\
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

/// Build the `tools` request payload, advertising **only** the tools the Writ
/// authorizes. Sending schemas for out-of-scope tools wastes tokens (a real
/// problem against per-minute budgets) and invites guaranteed-rejected
/// proposals — the model proposes a tool it can see, then hits `AuthorityVoid`.
/// The advertised surface now matches the actual granted authority.
fn build_tools_payload(tools: &ToolRegistry, writ: &Writ) -> Vec<Value> {
    let mut out = Vec::new();
    for name in tools.names() {
        if !writ.authorizes_tool(name) {
            continue;
        }
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
        // Only inline tools the Writ authorizes — same rationale as the native
        // payload: fewer tokens, no proposals the runtime will only reject.
        if !writ.authorizes_tool(name) {
            continue;
        }
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
        "You are OpenThymos, a capable, friendly AI assistant. Help the user \
         clearly, accurately, and naturally. You run inside a governed runtime: \
         to take a real-world action you don't act directly — you propose it and \
         the runtime checks it against a Capability Writ before it runs.\n\
         \n\
         To call a tool, emit one or more \
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
         Not every message needs a tool. If the user is greeting you, making \
         small talk, or asking something you can answer directly, just reply \
         with plain text and emit no JSON blocks. Only emit a tool block when \
         the task genuinely requires an external effect.\n\
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
    fn retry_after_header_parses_delta_seconds() {
        assert_eq!(parse_retry_after_header("10"), Some(10_000));
        assert_eq!(parse_retry_after_header(" 2.5 "), Some(2_500));
        // HTTP-date form is unsupported → None (caller falls back to body).
        assert_eq!(parse_retry_after_header("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    }

    #[test]
    fn retry_after_body_extracts_groq_wait() {
        let body = r#"{"error":{"message":"Rate limit reached ... Please try again in 9.62s. Need more tokens?","code":"rate_limit_exceeded"}}"#;
        assert_eq!(parse_retry_after_body(body), Some(9_620));
        assert_eq!(parse_retry_after_body("no hint here"), None);
    }

    #[test]
    fn retry_after_body_parses_compound_daily_limit() {
        // Per-day limit resets are reported as h/m/s — must parse to the full
        // duration so the runtime fails fast instead of retrying into a wall.
        let body = r#"{"error":{"message":"...tokens per day (TPD)... Please try again in 1h16m43.392s."}}"#;
        let ms = parse_retry_after_body(body).unwrap();
        // 1h16m43.392s = 4603392 ms (±1s for float).
        assert!((4_603_000..=4_604_000).contains(&ms), "got {ms}");
        assert_eq!(parse_retry_after_body("try again in 2m30s."), Some(150_000));
    }

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
