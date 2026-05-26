---
layout: default
title: FAQ
eyebrow: Fast answers
subtitle: The questions new users usually ask first.
permalink: /faq/
---

## Is OpenThymos a chatbot?

No. OpenThymos is a Rust execution runtime and framework. A model can drive it,
but the runtime owns execution, sandboxing, logging, approvals, replay, and
completion state.

## Is OpenThymos only for coding?

No, but coding agents are the first reference workload. The current runtime
path is exercised through repository work, file operations, tests, shell
commands, programmable capabilities, and observable execution loops.

## Do the CLI, web app, and VS Code extension each run their own agent?

No. CLI, web, VS Code, and terminal shell clients can all connect to the same
backend runtime and observe the same run.

## What does "agentic" mean here?

It means the runtime keeps working through a task: plan, act, observe, recover, and continue until the task is resolved or blocked.

## What keeps OpenThymos from becoming a free-for-all tool runner?

Signed writs, policy checks, capability scopes, budgets, approvals, typed
contracts, path confinement, worker-backed shell/HTTP isolation, and the
execution loop all constrain what the agent can do.

## Can OpenThymos recover from failures?

Yes. Transient cognition failures can be retried, and staged execution failures are surfaced back into the run so the agent can adapt rather than simply crash.

## Can I use local models?

Yes. LM Studio, Ollama, and other OpenAI-compatible local endpoints can all drive the same runtime.

## Can I add my own capabilities?

Yes. You can implement Rust `ToolContract`s, load JSON manifests through
`THYMOS_TOOL_MANIFEST_DIRS`, or bridge tools from an MCP server.

## What is the "execution session"?

It is the live state object behind a run: status, phase, operator state, counters, final answer, and execution log.

## What is the difference between the cognition stream and the execution session?

The cognition stream shows raw model-side activity such as token streaming. The execution session shows runtime-side truth: what phase the run is in, what the runtime did, and what the current result is.

## Where should a new user start?

Start the backend runtime, open the web console, run one mock task, then move to the CLI or VS Code sidebar once the flow is clear.
