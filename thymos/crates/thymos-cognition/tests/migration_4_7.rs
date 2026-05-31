//! Regression harness for the Opus 4.6 → 4.7 migration.
//!
//! These tests DO NOT make network calls — they exercise the Anthropic
//! adapter's public surface and the `CognitionConfig` schema to prove the
//! migration preserves behavior without regressions.
//!
//! Covers:
//!   * Default model id is Opus 4.7.
//!   * `with_model` accepts both full ids and aliases.
//!   * Prompt-cache prefix is on by default and can be toggled.
//!   * Extended thinking is opt-in; default config has it disabled.
//!   * CognitionConfig (de)serialization is backward-compatible with the
//!     pre-4.7 JSON shape (old clients that don't send the new fields keep
//!     working).
//!   * Legacy model ids still resolve to themselves (so pinned callers work).

use thymos_cognition::{
    anthropic::{AnthropicCognition, DEFAULT_MODEL, LEGACY_MODEL_4_6},
    CognitionConfig, CognitionProvider,
};

#[test]
fn default_model_is_opus_4_7() {
    assert_eq!(DEFAULT_MODEL, "claude-opus-4-7");
}

#[test]
fn legacy_constant_still_points_at_4_6() {
    assert_eq!(LEGACY_MODEL_4_6, "claude-opus-4-6");
}

#[test]
fn adapter_builder_accepts_short_and_full_ids() {
    // We can't call from_env() in tests (no API key), but with_api_key is fine
    // — no network hits until .step() is called.
    let c = AnthropicCognition::with_api_key("test-key".into()).unwrap();
    let _ = c.with_model("opus");
    let c2 = AnthropicCognition::with_api_key("test-key".into()).unwrap();
    let _ = c2.with_model("claude-opus-4-7");
    let c3 = AnthropicCognition::with_api_key("test-key".into()).unwrap();
    let _ = c3.with_model("opus-4.6");
}

#[test]
fn adapter_builder_exposes_all_new_knobs() {
    let c = AnthropicCognition::with_api_key("test-key".into()).unwrap();
    let _c = c
        .with_model("opus")
        .with_max_tokens(8192)
        .with_thinking(4096)
        .with_max_internal_messages(64);

    let c2 = AnthropicCognition::with_api_key("test-key".into()).unwrap();
    let _c2 = c2.without_cache_prefix();
}

#[test]
fn cognition_config_default_is_mock_with_cache_on() {
    let cfg = CognitionConfig::default();
    assert_eq!(cfg.provider, CognitionProvider::Mock);
    assert!(cfg.cache_prefix, "cache_prefix must default to true");
    assert!(cfg.thinking_budget_tokens.is_none());
    assert!(cfg.model.is_none());
    assert!(cfg.max_tokens.is_none());
}

/// Pre-4.7 clients serialize CognitionConfig without the new fields. The new
/// schema MUST deserialize those payloads without error. This is the single
/// most important compatibility guarantee of this migration.
#[test]
fn legacy_cognition_config_json_still_parses() {
    let legacy = serde_json::json!({
        "provider": "anthropic",
        "model": "claude-opus-4-6",
        "max_tokens": 2048
    });
    let cfg: CognitionConfig = serde_json::from_value(legacy).unwrap();
    assert_eq!(cfg.provider, CognitionProvider::Anthropic);
    assert_eq!(cfg.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(cfg.max_tokens, Some(2048));
    // Unset fields should take safe defaults.
    assert!(
        cfg.cache_prefix,
        "cache_prefix must default true when absent"
    );
    assert!(cfg.thinking_budget_tokens.is_none());
}

#[test]
fn new_cognition_config_serializes_with_all_fields() {
    let cfg = CognitionConfig {
        provider: CognitionProvider::Anthropic,
        model: Some("claude-opus-4-7".into()),
        max_tokens: Some(8192),
        base_url: None,
        thinking_budget_tokens: Some(4096),
        cache_prefix: true,
    };
    let v = serde_json::to_value(&cfg).unwrap();
    assert_eq!(v["provider"], "anthropic");
    assert_eq!(v["model"], "claude-opus-4-7");
    assert_eq!(v["max_tokens"], 8192);
    assert_eq!(v["thinking_budget_tokens"], 4096);
    assert_eq!(v["cache_prefix"], true);
}

#[test]
fn cache_prefix_opt_out_roundtrips() {
    let json = serde_json::json!({
        "provider": "anthropic",
        "cache_prefix": false,
    });
    let cfg: CognitionConfig = serde_json::from_value(json).unwrap();
    assert!(!cfg.cache_prefix);
}

#[test]
fn retry_classifier_matches_documented_policy() {
    use thymos_cognition::anthropic::{backoff_delay_ms, is_transient_status};

    // Documented retry surface:
    for s in [429, 500, 502, 503, 504, 529] {
        assert!(is_transient_status(s, None), "expected {s} retryable");
    }
    for s in [400, 401, 403, 404, 422] {
        assert!(!is_transient_status(s, None), "expected {s} fatal");
    }
    assert!(is_transient_status(500, Some("overloaded_error")));
    assert!(!is_transient_status(400, Some("invalid_request_error")));

    // Exponential, capped.
    assert_eq!(backoff_delay_ms(0), 500);
    assert_eq!(backoff_delay_ms(3), 4000);
    assert_eq!(backoff_delay_ms(100), 32_000);
}

#[test]
fn adapter_exposes_max_retries_knob() {
    use thymos_cognition::anthropic::AnthropicCognition;
    let c = AnthropicCognition::with_api_key("k".into()).unwrap();
    let c = c.with_max_retries(5);
    assert_eq!(c.max_retries(), 5);
}

#[cfg(feature = "async")]
mod streaming {
    use thymos_cognition::{
        mock::MockCognition, Cognition, CognitionContext, CognitionEvent, HistoryItem,
        NonStreamingAdapter, StreamingCognition,
    };
    use thymos_core::{
        commit::Observation,
        crypto::{generate_signing_key, public_key_of},
        intent::{Intent, IntentBody, IntentKind},
        world::World,
        writ::{Budget, DelegationBounds, EffectCeiling, TimeWindow, ToolPattern, Writ, WritBody},
    };
    use thymos_tools::ToolRegistry;

    fn fixture_writ() -> Writ {
        let root = generate_signing_key();
        let subject = generate_signing_key();
        Writ::sign(
            WritBody {
                issuer: "root".into(),
                issuer_pubkey: public_key_of(&root),
                subject: "test".into(),
                subject_pubkey: public_key_of(&subject),
                nonce: [0u8; 16],
                parent: None,
                tenant_id: String::new(),
                tool_scopes: vec![ToolPattern::exact("noop")],
                budget: Budget {
                    tokens: 1_000,
                    tool_calls: 4,
                    wall_clock_ms: 10_000,
                    usd_millicents: 0,
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
            },
            &root,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn non_streaming_adapter_emits_turn_complete_on_success() {
        let mock = MockCognition::new(vec![], Some("done".into()));
        let mut adapter = NonStreamingAdapter(mock);
        let writ = fixture_writ();
        let tools = ToolRegistry::new();
        let world = World::default();
        let ctx = CognitionContext {
            task: "t",
            writ: &writ,
            world: &world,
            tools: &tools,
            since_last: Vec::new(),
            step_index: 0,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CognitionEvent>(4);
        let step = adapter.step_streaming(&ctx, tx).await.unwrap();
        assert!(step.intents.is_empty());
        assert_eq!(step.final_answer.as_deref(), Some("done"));

        let evt = rx.recv().await.unwrap();
        match evt {
            CognitionEvent::TurnComplete {
                intents_count,
                final_answer,
            } => {
                assert_eq!(intents_count, 0);
                assert_eq!(final_answer.as_deref(), Some("done"));
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
    }

    /// Cognition that always fails — used to verify the streaming adapter
    /// surfaces an `Error` event before returning the error to the caller,
    /// which is the same path a retry-exhausted Anthropic adapter would take.
    struct FailingCognition;
    impl Cognition for FailingCognition {
        fn step(
            &mut self,
            _ctx: &CognitionContext<'_>,
        ) -> thymos_core::error::Result<thymos_cognition::CognitionStep> {
            Err(thymos_core::error::Error::Other(
                "simulated retry exhaustion".into(),
            ))
        }
    }

    #[tokio::test]
    async fn non_streaming_adapter_emits_error_event_on_failure() {
        let mut adapter = NonStreamingAdapter(FailingCognition);
        let writ = fixture_writ();
        let tools = ToolRegistry::new();
        let world = World::default();
        let ctx = CognitionContext {
            task: "t",
            writ: &writ,
            world: &world,
            tools: &tools,
            since_last: Vec::<HistoryItem>::new(),
            step_index: 0,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CognitionEvent>(4);
        let err = adapter.step_streaming(&ctx, tx).await.unwrap_err();
        assert!(
            format!("{err}").contains("simulated retry exhaustion"),
            "error passed through"
        );

        match rx.recv().await.unwrap() {
            CognitionEvent::Error { message } => {
                assert!(message.contains("simulated retry exhaustion"));
            }
            other => panic!("expected Error event, got {other:?}"),
        }
    }

    // Silence unused-import warnings when async feature is on but nothing
    // drops into this scope directly (e.g. Intent is imported for future tests).
    #[allow(dead_code)]
    fn _keep_imports_live() {
        let _ = |i: Intent| i;
        let _ = |o: Observation| o;
        let _ = |k: IntentKind| k;
        let _ = |b: IntentBody| b;
    }
}
