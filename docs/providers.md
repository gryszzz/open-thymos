---
layout: default
title: Cognition Providers
eyebrow: Bring your own model
subtitle: Swap the proposer, keep the same runtime, tools, and execution model.
permalink: /providers/
---

Thymos cognition is a single trait — `Cognition::step` — with multiple built-in
adapters. The important part is that the runtime does not change when you swap
providers. The same execution session, tool model, approvals, and operator
surfaces stay intact; only the proposer changes.

## Selector

Every `POST /runs` body takes a `cognition` block:

```json
{
  "task": "...",
  "cognition": {
    "provider": "huggingface",
    "model": "Qwen/Qwen2.5-Coder-32B-Instruct"
  }
}
```

Valid `provider` values: `anthropic`, `openai`, `lmstudio`, `huggingface`,
`local`, `mock`.

| Field                  | Notes                                                                          |
|------------------------|--------------------------------------------------------------------------------|
| `model`                | Provider-specific model id; falls back to env var, then provider default.      |
| `max_tokens`           | Cap on the response.                                                           |
| `base_url`             | Override the endpoint (any OpenAI-compatible API).                             |
| `thinking_budget_tokens` | Anthropic only — extended thinking budget. Must be `< max_tokens`.           |
| `cache_prefix`         | Anthropic only — toggle the system+tools prompt-cache breakpoint.              |

---

## Anthropic

Frontier reasoning + tool use. Default for serious runs.

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

```json
{ "provider": "anthropic", "model": "opus" }
```

Aliases: `opus`, `sonnet`, `haiku`, `opus-4.6`. Prompt caching, extended
thinking (4.7+), retry-on-transient, and full `stop_reason` handling are on
by default.

---

## OpenAI

```bash
export OPENAI_API_KEY=sk-...
```

```json
{ "provider": "openai", "model": "gpt-4o" }
```

Set `OPENAI_BASE_URL` (or pass `base_url`) to point at any OpenAI-compatible
gateway (Azure, Together, OpenRouter).

---

## LM Studio — local, free

Run a coding model on your laptop. LM Studio exposes an OpenAI-compatible
server; Thymos picks it up with one config line.

1. Open LM Studio, load a coding model (e.g. `qwen2.5-coder-32b-instruct`).
2. Start the local server (default port `1234`).
3. Hit Thymos with:

```json
{
  "provider": "lmstudio",
  "model": "qwen2.5-coder-32b-instruct"
}
```

Defaults: `base_url=http://localhost:1234/v1`, no auth required. Override with
`LMSTUDIO_BASE_URL`, `LMSTUDIO_MODEL`, `LMSTUDIO_API_KEY` env vars or via
`base_url`/`model` in the cognition block.

Most LM Studio builds ignore the `model` argument and serve whichever model is
loaded — that is fine, just pass any string.

---

## Hugging Face Router — hosted, free tier

The HF Router is an OpenAI-compatible serverless endpoint that fronts every
model on the Hub. One token, any model.

```bash
export HF_TOKEN=hf_...
# or HUGGINGFACE_API_KEY=hf_...
```

```json
{
  "provider": "huggingface",
  "model": "Qwen/Qwen2.5-Coder-32B-Instruct"
}
```

Defaults: `base_url=https://router.huggingface.co/v1`, default model
`Qwen/Qwen2.5-Coder-32B-Instruct`. Override with `HF_BASE_URL`, `HF_MODEL`, or
the cognition block.

Recommended models for the coding agent surface:

| Model                                          | Why                                              |
|------------------------------------------------|--------------------------------------------------|
| `Qwen/Qwen2.5-Coder-32B-Instruct`              | Strong tool following, low cost, default         |
| `meta-llama/Llama-3.3-70B-Instruct`            | General reasoning, broad availability            |
| `deepseek-ai/DeepSeek-V3`                      | Long-context coding work                         |
| `mistralai/Mixtral-8x22B-Instruct-v0.1`        | Cheap inference, good Rust/Python coverage       |

---

## Local — Ollama / vLLM / llama.cpp

Same shape as LM Studio, different default port. Use this preset for any
OpenAI-compatible server you run yourself.

```json
{
  "provider": "local",
  "base_url": "http://localhost:11434/v1",
  "model": "llama3"
}
```

---

## Mock

Deterministic, in-process. Used for tests and demos where you don't want to
talk to a real model.

```json
{ "provider": "mock" }
```

---

## Picking a provider

| You want…                                  | Pick                          |
|--------------------------------------------|-------------------------------|
| Best tool following on hard refactors      | `anthropic` (Opus 4.7)        |
| Free, on-laptop, no network                | `lmstudio`                    |
| Hosted but free / very cheap               | `huggingface`                 |
| Existing OpenAI workflow                   | `openai`                      |
| Self-hosted GPU box                        | `local` (Ollama / vLLM)       |
| CI / fast deterministic test               | `mock`                        |

The runtime, ledger, policy, and tool surface do not change. Only the
proposer does.

## Confirming which provider resolved

A run can override the provider for that run via its own `cognition` block, so
the configured default is not always the whole story. To see what the server
actually resolved as its default, check `/health`:

```bash
curl http://localhost:3001/health
# default_provider: "mock" | "anthropic" | "openai" | ...
# cognition_live:   false when the default provider is mock
```

`cognition_live` is the honest signal: when it is `false`, any run that omits
its own `cognition` block is answered by the deterministic mock, not a real
model.
