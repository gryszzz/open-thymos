//! Provider presets — name → OpenAI-compatible endpoint.
//!
//! Almost every model in the wild is served behind an OpenAI-compatible Chat
//! Completions API (the hosted clouds and every local runtime) or the native
//! Anthropic API. This registry maps a short, memorable provider name (`groq`,
//! `openrouter`, `ollama`, …) to the base URL, the environment variable(s) that
//! hold its API key, a sensible default model, and whether the provider supports
//! native function-calling (otherwise we drive it with the JSON-block protocol).
//!
//! The key never travels over the wire: a client names a provider, and the
//! **server** reads the corresponding key from its own environment. This keeps
//! the cognition boundary intact — cognition still only proposes; it gains no
//! authority — while making "use any model" a one-liner.

/// One OpenAI-compatible provider preset.
#[derive(Clone, Copy, Debug)]
pub struct ProviderPreset {
    /// Canonical id, e.g. `"groq"`.
    pub id: &'static str,
    /// Human-readable label for listings, e.g. `"Groq"`.
    pub label: &'static str,
    /// OpenAI-compatible base URL (the `/v1` root).
    pub base_url: &'static str,
    /// Environment variables checked in order for the API key. Empty means a
    /// local runtime that needs no key.
    pub api_key_envs: &'static [&'static str],
    /// A reasonable default model id when the caller doesn't specify one.
    pub default_model: &'static str,
    /// True if the provider implements native OpenAI tool-calling. False routes
    /// the adapter through the JSON-block tool protocol (robust for local /
    /// smaller models that lack reliable function-calling).
    pub native_tools: bool,
    /// Whether this is a local runtime (host loopback, no key required).
    pub local: bool,
    /// Alternate names that resolve to this preset.
    pub aliases: &'static [&'static str],
}

impl ProviderPreset {
    /// Does this provider require an API key (cloud) vs. run keyless (local)?
    pub fn requires_key(&self) -> bool {
        !self.api_key_envs.is_empty()
    }
}

/// The full preset table. Base URLs and key env vars follow each provider's
/// published OpenAI-compatible docs.
pub const PRESETS: &[ProviderPreset] = &[
    // ── Hosted, OpenAI-compatible ──────────────────────────────────────────
    ProviderPreset {
        id: "openai",
        label: "OpenAI",
        base_url: "https://api.openai.com/v1",
        api_key_envs: &["OPENAI_API_KEY"],
        default_model: "gpt-4o-mini",
        native_tools: true,
        local: false,
        aliases: &["gpt"],
    },
    ProviderPreset {
        id: "groq",
        label: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        api_key_envs: &["GROQ_API_KEY"],
        default_model: "llama-3.3-70b-versatile",
        native_tools: true,
        local: false,
        aliases: &[],
    },
    ProviderPreset {
        id: "openrouter",
        label: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        api_key_envs: &["OPENROUTER_API_KEY"],
        default_model: "openai/gpt-4o-mini",
        native_tools: true,
        local: false,
        aliases: &["or"],
    },
    ProviderPreset {
        id: "together",
        label: "Together AI",
        base_url: "https://api.together.xyz/v1",
        api_key_envs: &["TOGETHER_API_KEY"],
        default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        native_tools: true,
        local: false,
        aliases: &["togetherai"],
    },
    ProviderPreset {
        id: "deepseek",
        label: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        api_key_envs: &["DEEPSEEK_API_KEY"],
        default_model: "deepseek-chat",
        native_tools: true,
        local: false,
        aliases: &[],
    },
    ProviderPreset {
        id: "mistral",
        label: "Mistral AI",
        base_url: "https://api.mistral.ai/v1",
        api_key_envs: &["MISTRAL_API_KEY"],
        default_model: "mistral-large-latest",
        native_tools: true,
        local: false,
        aliases: &[],
    },
    ProviderPreset {
        id: "xai",
        label: "xAI (Grok)",
        base_url: "https://api.x.ai/v1",
        api_key_envs: &["XAI_API_KEY"],
        default_model: "grok-2-latest",
        native_tools: true,
        local: false,
        aliases: &["grok"],
    },
    ProviderPreset {
        id: "fireworks",
        label: "Fireworks AI",
        base_url: "https://api.fireworks.ai/inference/v1",
        api_key_envs: &["FIREWORKS_API_KEY"],
        default_model: "accounts/fireworks/models/llama-v3p3-70b-instruct",
        native_tools: true,
        local: false,
        aliases: &[],
    },
    ProviderPreset {
        id: "nvidia",
        label: "NVIDIA NIM",
        base_url: "https://integrate.api.nvidia.com/v1",
        api_key_envs: &["NVIDIA_API_KEY"],
        default_model: "meta/llama-3.3-70b-instruct",
        native_tools: true,
        local: false,
        aliases: &["nim"],
    },
    ProviderPreset {
        id: "cerebras",
        label: "Cerebras",
        base_url: "https://api.cerebras.ai/v1",
        api_key_envs: &["CEREBRAS_API_KEY"],
        default_model: "llama-3.3-70b",
        native_tools: true,
        local: false,
        aliases: &[],
    },
    ProviderPreset {
        id: "gemini",
        label: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        api_key_envs: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        default_model: "gemini-2.0-flash",
        native_tools: true,
        local: false,
        aliases: &["google"],
    },
    ProviderPreset {
        id: "perplexity",
        label: "Perplexity",
        base_url: "https://api.perplexity.ai",
        api_key_envs: &["PERPLEXITY_API_KEY"],
        default_model: "sonar",
        // Sonar models are search-grounded and don't expose reliable native
        // tool-calling; drive them through the JSON-block protocol.
        native_tools: false,
        local: false,
        aliases: &["pplx"],
    },
    ProviderPreset {
        id: "huggingface",
        label: "Hugging Face Router",
        base_url: "https://router.huggingface.co/v1",
        api_key_envs: &["HF_TOKEN", "HUGGINGFACE_API_KEY"],
        default_model: "meta-llama/Llama-3.3-70B-Instruct",
        native_tools: true,
        local: false,
        aliases: &["hf"],
    },
    // ── Local runtimes (no key, host loopback) ─────────────────────────────
    ProviderPreset {
        id: "ollama",
        label: "Ollama (local)",
        base_url: "http://localhost:11434/v1",
        api_key_envs: &[],
        default_model: "llama3.2",
        // Local models vary widely in tool-calling reliability; JSON-block is
        // the robust default. Override per-run if your model does native tools.
        native_tools: false,
        local: true,
        aliases: &[],
    },
    ProviderPreset {
        id: "lmstudio",
        label: "LM Studio (local)",
        base_url: "http://localhost:1234/v1",
        api_key_envs: &[],
        default_model: "local-model",
        native_tools: false,
        local: true,
        aliases: &["lm-studio"],
    },
    ProviderPreset {
        id: "vllm",
        label: "vLLM (local)",
        base_url: "http://localhost:8000/v1",
        // vLLM auth is optional; when enabled, set OPENAI_API_KEY (the generic
        // fallback build_from_preset checks) or use `--provider openai`.
        api_key_envs: &[],
        default_model: "default",
        native_tools: false,
        local: true,
        aliases: &[],
    },
    ProviderPreset {
        id: "llamacpp",
        label: "llama.cpp server (local)",
        base_url: "http://localhost:8080/v1",
        api_key_envs: &[],
        default_model: "default",
        native_tools: false,
        local: true,
        aliases: &["llama-cpp", "llama.cpp"],
    },
    ProviderPreset {
        id: "localai",
        label: "LocalAI (local)",
        base_url: "http://localhost:8080/v1",
        api_key_envs: &[],
        default_model: "gpt-4",
        native_tools: false,
        local: true,
        aliases: &[],
    },
];

/// Resolve a provider name (case-insensitive, trimmed) to its preset, matching
/// the canonical id or any alias.
pub fn resolve(name: &str) -> Option<&'static ProviderPreset> {
    let n = name.trim().to_ascii_lowercase();
    PRESETS
        .iter()
        .find(|p| p.id == n || p.aliases.iter().any(|a| *a == n))
}

/// All presets, for listing (`thymos providers`, a `/providers` endpoint, docs).
pub fn all() -> &'static [ProviderPreset] {
    PRESETS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_aliases_are_unique_and_resolvable() {
        let mut seen = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(seen.insert(p.id), "duplicate id/alias: {}", p.id);
            for a in p.aliases {
                assert!(seen.insert(*a), "duplicate id/alias: {a}");
            }
            // Every id resolves back to itself.
            assert_eq!(resolve(p.id).map(|r| r.id), Some(p.id));
        }
    }

    #[test]
    fn resolve_is_case_insensitive_and_alias_aware() {
        assert_eq!(resolve("  GroQ ").unwrap().id, "groq");
        assert_eq!(resolve("grok").unwrap().id, "xai");
        assert_eq!(resolve("google").unwrap().id, "gemini");
        assert!(resolve("definitely-not-a-provider").is_none());
    }

    #[test]
    fn local_presets_need_no_key() {
        for p in PRESETS.iter().filter(|p| p.local) {
            assert!(
                !p.requires_key(),
                "{} is local but lists key envs",
                p.id
            );
        }
    }
}
