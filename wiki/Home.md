# Thymos Wiki

Thymos is a unified AI execution runtime, framework, and sandbox for coding agents.

It lets users start work from the CLI, VS Code, a system terminal, or a web console while staying attached to the **same backend run**, the **same execution flow**, and the **same live execution log**.

## Start here

- [[Getting Started]]
- [[Interfaces]]
- [[Architecture]]
- [[API Overview]]
- [[FAQ]]

## The short version

Thymos runs tasks through:

`Intent -> Proposal -> Commit`

The model proposes.
The runtime decides.
Approved capabilities execute.
The results are observed, committed, and replayable.

Capabilities can be built as Rust contracts, JSON manifests, or MCP bridge
tools.

The system keeps carrying work forward until the task is complete, blocked, or cancelled.
