---
layout: default
title: Coding Agent
eyebrow: Operator surface
subtitle: Thymos turns coding work into a controlled execution loop instead of a prompt-and-tool guessing game.
permalink: /coding-agent/
---

## What the coding agent really is

The Thymos coding agent is the first major surface built on top of the runtime.

It is not just "an LLM with file tools."

It is a coding workflow where the runtime:

- plans a next step
- chooses an allowed tool
- executes that tool for real
- observes the result
- records the outcome
- retries or adapts when the step fails

## The loop

```
plan -> inspect -> edit -> test -> recover -> finish
```

Typical tools involved:

- `repo_map`
- `list_files`
- `fs_read`
- `grep`
- `fs_patch`
- `test_run`
- `shell`

## Why it keeps progressing

The important part is not that a model can call tools.

The important part is that the runtime keeps working through a task until it reaches a real result.

That means:

- failed test runs become observable runtime failures
- those failures are fed back into the loop
- the next step can repair the work instead of abandoning it

## What the operator sees

From any surface, the operator can see:

- which phase the run is in
- which tool is active
- what was committed
- what was rejected
- what failed
- whether the runtime is recovering or waiting for approval

That visibility is part of the runtime contract, not just debugging output.

## Safety shape

The coding agent is bounded by:

- tool scopes
- budgets
- effect ceilings
- time windows
- approval gates
- typed tool contracts

The model proposes edits. The runtime decides whether they execute.

## Shared across every surface

The coding agent is the same run whether you look at it through:

- the web operator console
- `thymos-cli`
- the interactive shell
- the VS Code sidebar

Those surfaces differ in presentation, not in runtime truth.

## Good first tasks

- "Map this repo and explain the main runtime crates."
- "Find the API client and add a small retry helper."
- "Run the relevant tests and repair any failure."
- "Summarize the last run and explain why it completed."

## What to read after this

- [Getting Started]({{ '/getting-started' | relative_url }})
- [Interfaces]({{ '/interfaces' | relative_url }})
- [Architecture]({{ '/architecture' | relative_url }})
- [API Reference]({{ '/api-reference' | relative_url }})
