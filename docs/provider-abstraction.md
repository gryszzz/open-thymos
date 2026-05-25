---
layout: default
title: Provider Abstraction
eyebrow: Cognition boundary
subtitle: Providers propose intents; they do not define execution authority.
permalink: /provider-abstraction/
---

# Provider Abstraction

OpenThymos treats providers as replaceable cognition sources. A provider may be
hosted, local, deterministic, stochastic, streamed, or mock. The runtime
contract remains the same.

## Provider Contract

The core contract is `Cognition::step`:

```text
(task, writ, world, tools, recent history, step index) -> CognitionStep
```

`CognitionStep` contains:

- zero or more intents
- optional final answer

The provider never receives authority to execute tools.

## Supported Provider Classes

The current provider selector includes:

- Anthropic
- OpenAI
- local OpenAI-compatible servers
- LM Studio
- Hugging Face Router
- mock cognition

All adapters must emit the same protocol object: `Intent`.

## Provider-Neutral Runtime

The following runtime behavior must not vary by provider:

- writ validation
- tool scope checks
- budget checks
- policy evaluation
- proposal status
- tool execution path
- commit construction
- ledger append semantics
- replay verification

A provider may influence which intent is proposed. It may not influence whether
authority exists.

## Streaming Providers

Streaming providers may emit token-level events for operator visibility. These
events are not runtime authority. The runtime waits for a structured cognition
step before submitting intents through the compiler.

## Mock Provider

The mock provider exists for deterministic tests and demos. It should be used
when the test concerns runtime semantics rather than provider behavior.

## Provider Drift

Provider APIs, model defaults, and tool-use syntax change over time. Provider
drift must not corrupt historical replay because replay does not call
providers. Historical runs are reconstructed from ledger data.

## Provider Swap Latency

Provider swapping should be measured as adapter selection and initialization
overhead. It must not include changes to runtime semantics. See
[benchmarks.md](benchmarks.md).

## Adapter Requirements

Provider adapters should:

- normalize provider output into typed intents
- preserve provider errors as runtime-visible failures
- avoid hidden tool calls
- avoid provider-specific policy logic
- expose enough configuration for reproducible run metadata
- fail closed when required credentials are missing

## Security Rule

Provider credentials are not capability writs. Possessing an API key to a
model provider does not grant tool authority inside OpenThymos.
