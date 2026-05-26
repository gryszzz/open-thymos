---
layout: default
title: Interfaces
eyebrow: Same runtime, different entry points
subtitle: Pick CLI, VS Code, terminal, or web. OpenThymos keeps one backend execution state.
permalink: /interfaces/
---

## The rule

The interfaces are **clients**, not separate runtimes.

If they point at the same Thymos server, they can all observe the same run.
They also share the same registered capability set, writ scope model, approval
state, and replayable ledger.

That means a task can begin in one place and be followed in another:

- start from the CLI
- inspect from the web console
- approve from the VS Code sidebar
- continue from a terminal shell
- replay the same trajectory later from any client

## Web console

Best for:

- first-time onboarding
- operator visibility
- live execution logs
- world replay and branching

The web console is the easiest way to understand what the runtime is doing because it exposes the execution session directly.

## CLI

Best for:

- terminal-first users
- automation and scripts
- inspecting runs from the shell
- follow mode during active execution

The CLI is especially useful when you want to launch a task quickly and stay close to the repo.

## VS Code sidebar

Best for:

- editor-native workflows
- reviewing approvals and diffs
- watching live runtime state while coding

The sidebar is not a separate assistant. It is a view into the same backend run.

## Interactive shell / system terminal

Best for:

- shell users who want a persistent Thymos session
- terminal-based review of approvals
- mixing ad hoc commands with runtime-driven runs

The shell is useful when you want the same runtime semantics without leaving a terminal workflow.

## What stays shared across all of them

- run id
- trajectory id
- execution phase
- registered capability names
- operator state
- live execution log
- approvals
- final answer
- world state

## What changes by interface

- how you submit tasks
- how you read the run
- how you review approvals
- how visual or text-heavy the experience is

## Recommended onboarding path

1. Start with the web console to understand the flow.
2. Use the CLI once you know the run model.
3. Add the VS Code sidebar if you want editor-native approval and monitoring.
4. Use the shell when you want a fully terminal-first setup.
