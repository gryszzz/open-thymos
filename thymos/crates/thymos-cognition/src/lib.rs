//! Thymos Cognition Gateway.
//!
//! The Cognition Gateway is the sole producer of `Intent`s in a Thymos run.
//! Any process that implements [`Cognition`] can drive the runtime — including
//! a deterministic mock for tests, a rule-based planner, or a language-model
//! adapter such as [`anthropic::AnthropicCognition`].
//!
//! The trait contract is intentionally narrow: on each step the gateway
//! receives the current context (task, writ, world projection, tool surface,
//! history since the last call) and returns a batch of Intents plus an
//! optional final answer. The runtime is responsible for submitting those
//! Intents through the IPC Triad and feeding the typed outcomes back on the
//! next step.
//!
//! Cognition **never** mutates state, **never** executes tools, and **never**
//! persists to the ledger. Those are runtime responsibilities.

pub mod anthropic;
pub mod context;
pub mod mock;
pub mod openai;
pub mod presets;

use serde::{Deserialize, Serialize};
use thymos_core::{
    commit::Observation, error::Result, intent::Intent, proposal::RejectionReason, world::World,
    writ::Writ,
};
use thymos_tools::ToolRegistry;

/// Context passed to [`Cognition::step`].
pub struct CognitionContext<'a> {
    pub task: &'a str,
    pub writ: &'a Writ,
    pub world: &'a World,
    pub tools: &'a ToolRegistry,
    /// History items produced since the previous call to `step` (empty on
    /// the first call).
    pub since_last: Vec<HistoryItem>,
    /// Zero on the first step; increments after each step.
    pub step_index: u32,
}

/// Typed feedback for cognition. These are NOT ledger entries — they are a
/// projection of ledger events the runtime thinks this cognition instance
/// should see. The runtime is free to omit, redact, or reorder items (for
/// instance, to respect a memory-view policy).
#[derive(Debug, Clone)]
pub enum HistoryItem {
    /// A previously-emitted Intent was committed; `observation` is the tool
    /// output as recorded in the ledger.
    Committed {
        intent: Intent,
        observation: Observation,
    },
    /// A previously-emitted Intent was rejected at the compiler boundary.
    /// Cognition should adjust its next Intent accordingly.
    Rejected {
        intent: Intent,
        reason: RejectionReason,
    },
    /// A previously-emitted Intent made it past proposal time but failed
    /// during execution or commit. Cognition should recover and try again.
    Failed { intent: Intent, error: String },
}

/// Token / cost usage a cognition gateway reports for one `step`. The runtime
/// debits these against the writ budget so that model spend — not just tool
/// calls — is bounded by capability. Adapters that cannot price a request leave
/// `usd_millicents` at zero; the token dimensions remain enforceable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usd_millicents: u64,
}

impl CognitionUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// One turn of cognition output.
#[derive(Debug, Clone, Default)]
pub struct CognitionStep {
    /// Intents to submit through the IPC Triad, in order. Empty means
    /// "terminate" — the runtime will stop the loop after this step.
    pub intents: Vec<Intent>,
    /// Optional natural-language result. Set when cognition has concluded
    /// the task. May accompany an empty `intents` list.
    pub final_answer: Option<String>,
    /// Token/cost usage incurred producing this step. Defaults to zero for
    /// gateways that don't report it (e.g. the deterministic mock).
    pub usage: CognitionUsage,
}

/// Cognition produces Intents. That is the entire contract.
pub trait Cognition: Send {
    fn step(&mut self, ctx: &CognitionContext<'_>) -> Result<CognitionStep>;
}

/// Convenience: Cognition for a boxed trait object.
impl Cognition for Box<dyn Cognition> {
    fn step(&mut self, ctx: &CognitionContext<'_>) -> Result<CognitionStep> {
        (**self).step(ctx)
    }
}

// ---- Streaming / Async Cognition ------------------------------------------

/// A token-level event emitted during streaming cognition. The runtime can
/// forward these over SSE to the client for real-time display.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CognitionEvent {
    /// A text token fragment from the model's response.
    Token { text: String },
    /// The model is beginning a tool-use block.
    ToolUseStart { tool: String, id: String },
    /// Incremental JSON fragment for the tool arguments being generated.
    ToolUseArgDelta { id: String, delta: String },
    /// The tool-use block is complete; arguments are fully formed.
    ToolUseDone { id: String },
    /// The model finished its turn. Intents count + final_answer summary.
    TurnComplete {
        intents_count: usize,
        final_answer: Option<String>,
    },
    /// An error occurred during streaming.
    Error { message: String },
}

/// Async streaming cognition trait. Implementations yield `CognitionEvent`s
/// through a channel as the model generates tokens, then return the final
/// `CognitionStep` from the future.
///
/// This allows the runtime to:
///   1. Forward token events to clients in real-time (SSE)
///   2. Still get the structured `CognitionStep` for the IPC Triad
#[cfg(feature = "async")]
#[async_trait::async_trait]
pub trait StreamingCognition: Send {
    /// Stream one step of cognition. Events (tokens, tool-use deltas) are sent
    /// through `event_tx`. The returned `CognitionStep` is the fully-parsed
    /// result of the turn.
    async fn step_streaming(
        &mut self,
        ctx: &CognitionContext<'_>,
        event_tx: tokio::sync::mpsc::Sender<CognitionEvent>,
    ) -> Result<CognitionStep>;
}

/// Adapter: wrap any sync `Cognition` as `StreamingCognition` by emitting
/// a single `TurnComplete` event (no token-level streaming).
#[cfg(feature = "async")]
pub struct NonStreamingAdapter<C: Cognition>(pub C);

#[cfg(feature = "async")]
#[async_trait::async_trait]
impl<C: Cognition + Send> StreamingCognition for NonStreamingAdapter<C> {
    async fn step_streaming(
        &mut self,
        ctx: &CognitionContext<'_>,
        event_tx: tokio::sync::mpsc::Sender<CognitionEvent>,
    ) -> Result<CognitionStep> {
        // `block_in_place` lets the synchronous `Cognition::step` call do
        // blocking work without pinning the reactor and still preserves the
        // borrowed `&ctx` (which `spawn_blocking`'s `'static` bound would
        // reject). Some callers and tests run on Tokio's current-thread
        // runtime, though, where `block_in_place` panics. Fall back to a
        // direct call in that environment instead of crashing.
        let step_result = match tokio::runtime::Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| self.0.step(ctx))
            }
            _ => self.0.step(ctx),
        };

        match &step_result {
            Ok(step) => {
                let _ = event_tx
                    .send(CognitionEvent::TurnComplete {
                        intents_count: step.intents.len(),
                        final_answer: step.final_answer.clone(),
                    })
                    .await;
            }
            Err(e) => {
                let _ = event_tx
                    .send(CognitionEvent::Error {
                        message: format!("{e}"),
                    })
                    .await;
            }
        }
        step_result
    }
}

// ── Multi-model selector ─────────────────────────────────────────────────────

/// Provider identifier for multi-model cognition.
///
/// On the wire this is just a string, so **any** provider name works — the
/// native adapters (`anthropic`, `openai`, `mock`) plus every entry in the
/// [`presets`] registry (`groq`, `openrouter`, `ollama`, …) and any future name.
/// Unknown names resolve through the preset registry at build time. This is what
/// makes "use any model" a one-liner without per-provider wire changes.
#[derive(Clone, Debug, PartialEq)]
pub enum CognitionProvider {
    Anthropic,
    Openai,
    /// A local/custom OpenAI-compatible endpoint (Ollama, vLLM, llama.cpp).
    Local,
    /// LM Studio's OpenAI-compatible local server (default :1234/v1).
    Lmstudio,
    /// Hugging Face Router (serverless inference, OpenAI-compatible).
    Huggingface,
    Mock,
    /// Any other provider name, resolved against the [`presets`] registry when
    /// cognition is built (e.g. `groq`, `openrouter`, `gemini`, `deepseek`).
    Named(String),
}

impl CognitionProvider {
    /// The canonical wire/label string for this provider.
    pub fn as_str(&self) -> &str {
        match self {
            CognitionProvider::Anthropic => "anthropic",
            CognitionProvider::Openai => "openai",
            CognitionProvider::Local => "local",
            CognitionProvider::Lmstudio => "lmstudio",
            CognitionProvider::Huggingface => "huggingface",
            CognitionProvider::Mock => "mock",
            CognitionProvider::Named(s) => s,
        }
    }

    /// Parse a provider name (case-insensitive). Native names map to their
    /// dedicated adapters; everything else becomes [`CognitionProvider::Named`]
    /// and is resolved against the preset registry at build time.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "anthropic" => CognitionProvider::Anthropic,
            "openai" => CognitionProvider::Openai,
            "local" => CognitionProvider::Local,
            "lmstudio" => CognitionProvider::Lmstudio,
            "huggingface" => CognitionProvider::Huggingface,
            "mock" => CognitionProvider::Mock,
            other => CognitionProvider::Named(other.to_string()),
        }
    }
}

impl Serialize for CognitionProvider {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CognitionProvider {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(CognitionProvider::from_name(&s))
    }
}

/// Configuration for selecting a cognition provider at run creation time.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CognitionConfig {
    pub provider: CognitionProvider,
    /// Model name override. Accepts Anthropic aliases (`opus`, `sonnet`,
    /// `haiku`, `opus-4.6`) plus provider-native ids such as `gpt-4o-mini`
    /// or `llama3`.
    #[serde(default)]
    pub model: Option<String>,
    /// Max tokens for the response.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Base URL override (used for Local provider, or custom OpenAI endpoints).
    #[serde(default)]
    pub base_url: Option<String>,
    /// Extended-thinking budget in tokens (Anthropic only). When set, the
    /// adapter enables `thinking` on every turn with this budget. Keep
    /// strictly less than `max_tokens`.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
    /// If false, the Anthropic adapter will NOT place a `cache_control`
    /// breakpoint on the system+tools prefix. Defaults to true (caching on).
    #[serde(default = "default_cache_prefix")]
    pub cache_prefix: bool,
}

fn default_cache_prefix() -> bool {
    true
}

impl Default for CognitionConfig {
    fn default() -> Self {
        CognitionConfig {
            provider: CognitionProvider::Mock,
            model: None,
            max_tokens: None,
            base_url: None,
            thinking_budget_tokens: None,
            cache_prefix: true,
        }
    }
}

/// Build a `Box<dyn Cognition>` from a [`CognitionConfig`].
///
/// Falls back to mock if the requested provider's API key is not set.
pub fn build_cognition(config: &CognitionConfig) -> Box<dyn Cognition> {
    match &config.provider {
        CognitionProvider::Anthropic => match anthropic::AnthropicCognition::from_env() {
            Ok(mut c) => {
                if let Some(m) = &config.model {
                    c = c.with_model(m.as_str());
                }
                if let Some(t) = config.max_tokens {
                    c = c.with_max_tokens(t);
                }
                if let Some(tb) = config.thinking_budget_tokens {
                    c = c.with_thinking(tb);
                }
                if !config.cache_prefix {
                    c = c.without_cache_prefix();
                }
                Box::new(c)
            }
            Err(_) => {
                eprintln!("warn: ANTHROPIC_API_KEY not set, falling back to mock");
                Box::new(mock::MockCognition::new(
                    vec![],
                    Some("no cognition configured".into()),
                ))
            }
        },
        CognitionProvider::Openai => match openai::OpenAiCognition::from_env() {
            Ok(mut c) => {
                if let Some(m) = &config.model {
                    c = c.with_model(m.as_str());
                }
                if let Some(t) = config.max_tokens {
                    c = c.with_max_tokens(t);
                }
                if let Some(u) = &config.base_url {
                    c = c.with_base_url(u.as_str());
                }
                Box::new(c)
            }
            Err(_) => {
                eprintln!("warn: OPENAI_API_KEY not set, falling back to mock");
                Box::new(mock::MockCognition::new(
                    vec![],
                    Some("no cognition configured".into()),
                ))
            }
        },
        CognitionProvider::Local => {
            // Local uses the OpenAI-compatible API with a custom base URL.
            let base_url = config
                .base_url
                .clone()
                .or_else(generic_base_url_env)
                .unwrap_or_else(|| "http://localhost:11434/v1".into());
            let model = config.model.clone().unwrap_or_else(|| "llama3".into());
            // Local endpoints often don't need an API key; use a dummy.
            let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "local".into());
            match openai::OpenAiCognition::new(api_key, base_url, model) {
                Ok(mut c) => {
                    // Local models typically lack native function calling.
                    c = c.with_tool_protocol(openai::ToolProtocol::JsonBlock);
                    if let Some(t) = config.max_tokens {
                        c = c.with_max_tokens(t);
                    }
                    Box::new(c)
                }
                Err(_) => Box::new(mock::MockCognition::new(
                    vec![],
                    Some("local cognition failed to init".into()),
                )),
            }
        }
        CognitionProvider::Lmstudio => {
            // LM Studio exposes an OpenAI-compatible server on :1234 by
            // default. No auth required; the `model` argument is ignored by
            // most LM Studio builds (it serves whichever model is loaded).
            let base_url = config
                .base_url
                .clone()
                .or_else(|| std::env::var("LMSTUDIO_BASE_URL").ok())
                .or_else(generic_base_url_env)
                .unwrap_or_else(|| "http://localhost:1234/v1".into());
            let model = config
                .model
                .clone()
                .or_else(|| std::env::var("LMSTUDIO_MODEL").ok())
                .unwrap_or_else(|| "local-model".into());
            let api_key = std::env::var("LMSTUDIO_API_KEY").unwrap_or_else(|_| "lm-studio".into());
            match openai::OpenAiCognition::new(api_key, base_url, model) {
                Ok(mut c) => {
                    if let Some(t) = config.max_tokens {
                        c = c.with_max_tokens(t);
                    }
                    Box::new(c)
                }
                Err(_) => Box::new(mock::MockCognition::new(
                    vec![],
                    Some("lmstudio cognition failed to init".into()),
                )),
            }
        }
        CognitionProvider::Huggingface => {
            // HF Router is the unified OpenAI-compatible endpoint. The token
            // can come from `HF_TOKEN` (canonical) or `HUGGINGFACE_API_KEY`.
            let base_url = config
                .base_url
                .clone()
                .or_else(|| std::env::var("HF_BASE_URL").ok())
                .or_else(generic_base_url_env)
                .unwrap_or_else(|| "https://router.huggingface.co/v1".into());
            let model = config
                .model
                .clone()
                .or_else(|| std::env::var("HF_MODEL").ok())
                .unwrap_or_else(|| "Qwen/Qwen2.5-Coder-32B-Instruct".into());
            let api_key = std::env::var("HF_TOKEN")
                .or_else(|_| std::env::var("HUGGINGFACE_API_KEY"))
                // The desktop injects the stored key generically; honor it so
                // an HF token entered there isn't silently dropped.
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .unwrap_or_default();
            if api_key.is_empty() {
                eprintln!(
                    "warn: HF_TOKEN / HUGGINGFACE_API_KEY not set; HF Router will reject requests"
                );
            }
            match openai::OpenAiCognition::new(api_key, base_url, model) {
                Ok(mut c) => {
                    if let Some(t) = config.max_tokens {
                        c = c.with_max_tokens(t);
                    }
                    Box::new(c)
                }
                Err(_) => Box::new(mock::MockCognition::new(
                    vec![],
                    Some("huggingface cognition failed to init".into()),
                )),
            }
        }
        CognitionProvider::Named(name) => build_from_preset(name.as_str(), config),
        CognitionProvider::Mock => Box::new(mock::MockCognition::new(
            vec![],
            Some("mock cognition".into()),
        )),
    }
}

/// Generic base-URL override for any OpenAI-compatible adapter. This is the
/// variable the desktop injects for every non-Anthropic provider, so a custom
/// endpoint (Ollama on another port, a remote LM Studio, a gateway) reaches
/// the preset path instead of being silently dropped.
fn generic_base_url_env() -> Option<String> {
    std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Build an OpenAI-compatible adapter from a named [`presets`] entry. The API
/// key is read **server-side** from the preset's environment variables (never
/// from the wire); local presets need no key. Falls back to mock — with a clear
/// reason — for an unknown provider name or a cloud preset missing its key.
fn build_from_preset(name: &str, config: &CognitionConfig) -> Box<dyn Cognition> {
    let Some(preset) = presets::resolve(name) else {
        eprintln!(
            "warn: unknown cognition provider '{name}', falling back to mock. \
             Run `thymos providers` to list supported names."
        );
        return Box::new(mock::MockCognition::new(
            vec![],
            Some(format!("unknown provider '{name}'")),
        ));
    };

    let base_url = config
        .base_url
        .clone()
        .or_else(generic_base_url_env)
        .unwrap_or_else(|| preset.base_url.to_string());
    let model = config
        .model
        .clone()
        .unwrap_or_else(|| preset.default_model.to_string());

    // Resolve the key from the preset's env vars, then a generic OPENAI_API_KEY,
    // then a keyless dummy for local runtimes.
    let api_key = preset
        .api_key_envs
        .iter()
        .find_map(|e| std::env::var(e).ok())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok());
    let api_key = match api_key {
        Some(k) if !k.trim().is_empty() => k,
        _ if !preset.requires_key() => "local".to_string(),
        _ => {
            eprintln!(
                "warn: {} requires an API key ({}); none set — falling back to mock",
                preset.label,
                preset.api_key_envs.join(" or ")
            );
            return Box::new(mock::MockCognition::new(
                vec![],
                Some(format!("{} API key not set", preset.label)),
            ));
        }
    };

    match openai::OpenAiCognition::new(api_key, base_url, model) {
        Ok(mut c) => {
            // Providers without reliable native function-calling are driven via
            // the JSON-block tool protocol so they can still emit intents.
            if !preset.native_tools {
                c = c.with_tool_protocol(openai::ToolProtocol::JsonBlock);
            }
            if let Some(t) = config.max_tokens {
                c = c.with_max_tokens(t);
            }
            Box::new(c)
        }
        Err(e) => {
            eprintln!("warn: failed to initialize {}: {e}", preset.label);
            Box::new(mock::MockCognition::new(
                vec![],
                Some(format!("{} init failed", preset.label)),
            ))
        }
    }
}
