//! Keyless HTTP integration tests for the cognition adapters.
//!
//! These spin a tiny local HTTP stub, point a real adapter at it, and assert
//! request/response handling and error robustness — no API key, no network
//! egress, runnable in CI. The OpenAI-compatible adapter here also covers the
//! ~15 presets (groq, openrouter, together, deepseek, …), which all share it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use thymos_cognition::{
    anthropic::AnthropicCognition, openai::OpenAiCognition, Cognition, CognitionContext,
    CognitionStep,
};
use thymos_core::{
    crypto::{generate_signing_key, public_key_of},
    error::Result,
    world::World,
    writ::{Budget, DelegationBounds, EffectCeiling, TimeWindow, ToolPattern, Writ, WritBody},
};
use thymos_tools::ToolRegistry;

/// Serve one fixed HTTP response to every connection. Returns the base URL
/// (with the `/v1` suffix the OpenAI adapter expects).
fn stub(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Drain the (small, localhost) request; we don't need its content.
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            let reason = if (200..300).contains(&status) { "OK" } else { "ERR" };
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
        }
    });
    format!("http://{addr}/v1")
}

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
            tool_scopes: vec![ToolPattern::exact("*")],
            budget: Budget {
                tokens: 10_000,
                tool_calls: 8,
                wall_clock_ms: 30_000,
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

fn step_against(base_url: &str) -> Result<CognitionStep> {
    let mut c = OpenAiCognition::new("dummy-key".into(), base_url.into(), "stub-model".into())
        .expect("adapter constructs with a dummy key (no network until step)");
    let writ = fixture_writ();
    let tools = ToolRegistry::new();
    let world = World::default();
    let ctx = CognitionContext {
        task: "adapter smoke test",
        writ: &writ,
        world: &world,
        tools: &tools,
        since_last: Vec::new(),
        step_index: 0,
    };
    c.step(&ctx)
}

#[test]
fn openai_adapter_parses_final_answer_and_usage() {
    let body = r#"{"id":"x","object":"chat.completion","model":"stub-model",
        "choices":[{"index":0,"finish_reason":"stop",
            "message":{"role":"assistant","content":"hello from the stub"}}],
        "usage":{"prompt_tokens":7,"completion_tokens":5,"total_tokens":12}}"#;
    let url = stub(200, body);
    let step = step_against(&url).expect("a content response must parse");
    assert!(step.intents.is_empty(), "no tool calls → no intents");
    assert_eq!(step.final_answer.as_deref(), Some("hello from the stub"));
    assert_eq!(step.usage.input_tokens, 7);
    assert_eq!(step.usage.output_tokens, 5);
}

#[test]
fn openai_adapter_errors_cleanly_on_5xx() {
    // A provider 5xx must surface as an Error, never a panic.
    let url = stub(500, r#"{"error":{"message":"upstream overloaded"}}"#);
    let err = step_against(&url).expect_err("5xx must be an error");
    assert!(
        format!("{err}").contains("500"),
        "error should mention the status: {err}"
    );
}

#[test]
fn openai_adapter_errors_cleanly_on_malformed_body() {
    // A 200 with a non-JSON body must error at parse, not panic.
    let url = stub(200, "<html>gateway error</html>");
    let err = step_against(&url).expect_err("malformed body must error");
    assert!(
        format!("{err}").to_lowercase().contains("parse"),
        "error should be a parse failure: {err}"
    );
}

// ----- Anthropic adapter (now base_url-overridable) -----

fn anthropic_step(base_url: &str) -> Result<CognitionStep> {
    let mut c = AnthropicCognition::with_api_key("dummy-key".into())
        .unwrap()
        .with_base_url(base_url)
        // Don't burn retry backoff in tests; fatal statuses don't retry anyway.
        .with_max_retries(0);
    let writ = fixture_writ();
    let tools = ToolRegistry::new();
    let world = World::default();
    let ctx = CognitionContext {
        task: "adapter smoke test",
        writ: &writ,
        world: &world,
        tools: &tools,
        since_last: Vec::new(),
        step_index: 0,
    };
    c.step(&ctx)
}

#[test]
fn anthropic_adapter_parses_text_answer_and_usage() {
    let body = r#"{"id":"msg","type":"message","role":"assistant","model":"claude",
        "stop_reason":"end_turn",
        "content":[{"type":"text","text":"hi from the anthropic stub"}],
        "usage":{"input_tokens":9,"output_tokens":4}}"#;
    let url = stub(200, body);
    let step = anthropic_step(&url).expect("a text response must parse");
    assert!(step.intents.is_empty());
    assert_eq!(step.final_answer.as_deref(), Some("hi from the anthropic stub"));
    assert_eq!(step.usage.input_tokens, 9);
    assert_eq!(step.usage.output_tokens, 4);
}

#[test]
fn anthropic_adapter_errors_cleanly_on_fatal_status() {
    // A 400 is fatal (not retried) and must surface as a clean error.
    let url = stub(400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#);
    let err = anthropic_step(&url).expect_err("4xx must be an error");
    let msg = format!("{err}");
    assert!(
        msg.contains("400") || msg.to_lowercase().contains("invalid_request"),
        "error should reflect the fatal status: {msg}"
    );
}
