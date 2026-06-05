---
layout: default
title: Getting Started
eyebrow: 5 minutes · easiest path first
subtitle: Start the Rust runtime once, then attach from CLI, VS Code, terminal, or web.
permalink: /getting-started/
---

## The fast mental model

You do **not** start a separate agent for every interface.

You start the **OpenThymos runtime**, then attach to it from:

- the web console
- the CLI
- the VS Code sidebar
- the interactive shell / system terminal

All of those clients observe the same run state when they point at the same backend.

## 1. Start the OpenThymos runtime

```bash
git clone https://github.com/gryszzz/open-thymos.git
cd open-thymos/thymos
cargo run -p thymos-server
```

Default behavior:

- server runs on `http://localhost:3001`
- mock cognition is available, so you can test the full flow with no API key
- runs are exposed through HTTP plus live SSE streams

Check it:

```bash
curl http://localhost:3001/health
curl http://localhost:3001/ready
```

## 2. Choose your interface

### Option A: Web console

From the repo root:

```bash
npm install
npm run dev
```

Open:

`http://localhost:3000/runs`

Validate the exported GitHub Pages site plus the markdown docs before you push:

```bash
npm run site:check
```

Preview the static Pages export locally:

```bash
npm run pages:preview
```

The public site is served from `https://gryszzz.github.io/open-thymos/`.

Use this when you want:

- the clearest onboarding path
- the live execution console
- the execution log
- world replay and branching

### Option B: CLI

In another terminal:

```bash
cd thymos
cargo run -p thymos-cli -- run "Inspect the repo and explain what Thymos is" --provider mock --follow
```

Use this when you want:

- terminal-first workflow
- live execution follow mode
- run inspection, diffing, resume, and cancel commands

### Option C: VS Code sidebar

Build the extension:

```bash
cd thymos/clients/vscode
npm install
npm run compile
```

Then open the extension in VS Code's Extension Development Host and point it at:

`http://localhost:3001`

Use this when you want:

- editor-native run visibility
- approval review inside VS Code
- a shared console without leaving your coding flow

## 3. Submit your first task

No model key required:

```bash
cd thymos
cargo run -p thymos-cli -- run "Map the repo and summarize the execution runtime" --provider mock --follow
```

What you should see:

1. a run is created
2. the execution session starts updating
3. the runtime emits intent / proposal / execution / result events
4. the run finishes with a final answer

## 4. Switch to a real model

Thymos keeps the same runtime and tool model. You only swap the proposer — and
you just set a key. **Any run that doesn't specify its own `cognition` block now
uses the provider you configured** (instead of silently falling back to mock).

The CLI cooperates: `thymos run` defaults to `--provider auto`, which sends **no**
provider override, so your runs use whatever the server resolved. Once the server
has a key, the same `thymos run "…"` uses the real model with no CLI change. Pass
`--provider mock` to force the deterministic mock for a single run.

### Anthropic

```bash
ANTHROPIC_API_KEY=... cargo run -p thymos-server
```

### OpenAI

```bash
OPENAI_API_KEY=... cargo run -p thymos-server
```

### Local OpenAI-compatible backend

```bash
OPENAI_BASE_URL=http://localhost:1234/v1 OPENAI_API_KEY=local cargo run -p thymos-server
```

### Any other model — presets

Almost every model out there is served behind an OpenAI-compatible API. Name a
**preset** and set its key; Thymos fills in the endpoint. Run `thymos providers`
to list them all with their key env var and an example model.

```bash
# Hosted — set the provider + its key, then start:
THYMOS_DEFAULT_PROVIDER=groq       GROQ_API_KEY=...       cargo run -p thymos-server
THYMOS_DEFAULT_PROVIDER=openrouter OPENROUTER_API_KEY=... cargo run -p thymos-server
THYMOS_DEFAULT_PROVIDER=deepseek   DEEPSEEK_API_KEY=...   cargo run -p thymos-server
THYMOS_DEFAULT_PROVIDER=gemini     GEMINI_API_KEY=...     cargo run -p thymos-server

# Local — no key, just have the runtime running:
THYMOS_DEFAULT_PROVIDER=ollama THYMOS_DEFAULT_MODEL=llama3.2 cargo run -p thymos-server
```

Built-in presets: `openai`, `groq`, `openrouter`, `together`, `deepseek`,
`mistral`, `xai` (grok), `fireworks`, `nvidia`, `cerebras`, `gemini`,
`perplexity`, `huggingface` — plus local `ollama`, `lmstudio`, `vllm`,
`llamacpp`, `localai`. Or point at **any** OpenAI-compatible URL directly:

```bash
thymos run "..." --provider groq --model llama-3.3-70b-versatile
thymos run "..." --provider openai --base-url https://your-host/v1 --model your-model
```

Keys are read **server-side** — only the provider *name* travels over the wire,
never a key. Cognition still just proposes intents; the runtime governs every
effect regardless of which model you pick.

Confirm which provider is active (no more guessing whether you're on mock):

```bash
curl http://localhost:3001/health
# { "status": "ok", "mode": "reference", "default_provider": "anthropic", ... }
```

Resolution order for the default provider:

1. `THYMOS_DEFAULT_PROVIDER` — `anthropic`, `openai`, `mock`, or **any preset
   name** (`groq`, `openrouter`, `gemini`, `ollama`, … — run `thymos providers`),
   with optional `THYMOS_DEFAULT_MODEL`.
2. Otherwise the first key found — `ANTHROPIC_API_KEY`, then `OPENAI_API_KEY`.
3. Otherwise `mock`.

A run can still override per-request by sending a `cognition` block to
`POST /runs` or `--provider` on the CLI. See
[`thymos/.env.example`](https://github.com/gryszzz/open-thymos/blob/main/thymos/.env.example)
for the full set of runtime variables.

## 5. Load programmable capabilities

Capabilities are registered on the server, so every surface sees the same
available tools.

```bash
mkdir -p tools
cat > tools/repo_word_count.json <<'JSON'
{
  "name": "repo_word_count",
  "version": "0.1.0",
  "description": "Count words in a repository file",
  "effect_class": "read",
  "risk_class": "low",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string" }
    },
    "required": ["path"]
  },
  "executor": {
    "kind": "shell",
    "command_template": "wc -w {path}"
  }
}
JSON

cd thymos
THYMOS_TOOL_MANIFEST_DIRS=../tools cargo run -p thymos-server
```

Read the full capability model in
[Programmable Capabilities]({{ '/programmable-capabilities' | relative_url }}).

## 6. Understand what you are looking at

Every run follows the same structure:

### Intent

The model declares what it wants to do next.

### Proposal

The runtime compiles and policy-checks that action under the active writ.

### Execution

The tool runs for real and the runtime observes the result.

### Result

The run records a commit, rejection, suspension, failure, or completion event.

## 7. Production-shaped mode

For persistent, safer runtime behavior:

```bash
cargo build --release -p thymos-worker

THYMOS_RUNTIME_MODE=production \
THYMOS_BIND_ADDR=0.0.0.0:3001 \
THYMOS_LEDGER_PATH=thymos-ledger.db \
THYMOS_DB_PATH=thymos-runs.db \
THYMOS_GATEWAY_DB_PATH=thymos-gateway.db \
THYMOS_MARKETPLACE_DB_PATH=thymos-marketplace.db \
THYMOS_ALLOWED_ORIGINS=https://your-console.example.com \
THYMOS_MAX_CONCURRENT_RUNS_GLOBAL=100 \
THYMOS_MAX_CONCURRENT_RUNS_PER_TENANT=20 \
THYMOS_TOOL_MANIFEST_DIRS=../tools \
THYMOS_TOOL_FABRIC=worker \
THYMOS_WORKER_BIN=$PWD/target/release/thymos-worker \
cargo run -p thymos-server
```

Use this when you want:

- file-backed run history
- worker-backed tool execution
- startup-loaded programmable capabilities
- explicit browser origin policy
- deploy-time concurrency tuning
- a more production-shaped runtime boundary

## 8. Postgres ledger backend (optional)

The durable ledger defaults to SQLite. To run it on Postgres instead, build the
server with the `postgres` feature and point it at a database:

```bash
# Bring up a local Postgres (or use your own); from thymos/:
docker compose --profile postgres up -d postgres

THYMOS_POSTGRES_URL=postgres://thymos:thymos@localhost:5432/thymos \
  cargo run -p thymos-server --features postgres
```

The startup log prints `ledger: postgres (synchronous blocking facade) at …`
when it connects. Notes:

- Without the `postgres` feature, `THYMOS_POSTGRES_URL` is ignored — the server
  logs a note and stays on SQLite. The **default build compiles no Postgres
  dependencies**.
- Replay is byte-identical across backends: the same trajectory yields the same
  content-addressed chain on SQLite and Postgres (this is asserted by the gated
  `postgres_integration` tests).

## Where to go next

- [Interfaces]({{ '/interfaces' | relative_url }}) — pick the surface that fits you
- [Coding Agent]({{ '/coding-agent' | relative_url }}) — understand the autonomous coding loop
- [Programmable Capabilities]({{ '/programmable-capabilities' | relative_url }}) — extend the runtime safely
- [Architecture]({{ '/architecture' | relative_url }}) — see how the shared runtime is built
- [API Reference]({{ '/api-reference' | relative_url }}) — drive it over HTTP
