---
layout: default
title: Programmable Capabilities
eyebrow: Framework layer
subtitle: Extend the OpenThymos runtime with governed capabilities that every client surface can use.
permalink: /programmable-capabilities/
---

# Programmable Capabilities

Capabilities are the effect boundary in OpenThymos. Cognition can request a
capability, but the runtime decides whether that capability may execute under
the active writ, policy trace, budget, effect ceiling, and sandbox profile.

Once registered, a capability is available to every surface attached to the
same server: CLI, VS Code, interactive terminal shell, API client, and web
console.

## Capability Forms

OpenThymos supports three extension paths:

- **Rust contracts** implement `ToolContract` for first-party capabilities that
  need strong validation, custom preconditions, postconditions, deltas, or
  worker integration.
- **JSON manifests** register local shell, HTTP, or no-op capabilities without
  recompiling the runtime.
- **MCP bridges** discover tools from an MCP server and register them into the
  same `ToolRegistry`.

All forms expose a name, version, description, input schema, effect class, and
risk class. Writ tool scopes and policy rules are evaluated before execution.
Manifest tools are validated at load time, loaded in deterministic filename
order, and cannot shadow an already registered built-in capability.

## Manifest Capabilities

Point the server at one or more manifest directories:

```bash
THYMOS_TOOL_MANIFEST_DIRS=./tools cargo run -p thymos-server
```

`THYMOS_TOOL_MANIFEST_DIR` is accepted as a singular alias. Both variables are
comma-separated when multiple directories are needed.

Example manifest:

```json
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
```

Then authorize it in a run:

```json
{
  "task": "Count words in README.md",
  "tool_scopes": ["repo_word_count"],
  "cognition": { "provider": "mock" }
}
```

Manifest tool names must use ASCII letters, digits, `_`, `-`, or `.`. HTTP
manifest capabilities block private and loopback hosts by default; use an
explicit `allowlist` when a capability should call only known domains.

## Sandbox Shape

OpenThymos has several sandbox layers:

- coding tools such as `fs_read`, `fs_patch`, `grep`, `repo_map`, and
  `test_run` are path-confined to allowed roots
- stock `shell` and `http` capabilities can run through `thymos-worker` with
  `THYMOS_TOOL_FABRIC=worker`
- writ scopes, budgets, time windows, effect ceilings, and policy gates apply
  before a capability can reach its executor

Manifest shell and HTTP capabilities are lightweight local extensions. Use them
for low-risk, local automation. They cannot override built-in capabilities, and
HTTP manifests default to SSRF-resistant private-host blocking. For high-risk
execution, implement a Rust `ToolContract` that delegates to the worker fabric
or expose the effect through a hardened external service.

## Framework Contract

A custom Rust capability implements:

- `meta()` for name, version, effect class, and risk class
- `description()` and `input_schema()` for cognition-facing tool shape
- `validate_args()` for typed argument validation
- `check_preconditions()` against the current `World`
- `execute()` to return an observation and structured delta
- `check_postconditions()` against the would-be next world state

The runtime uses that contract in the normal `Intent -> Proposal -> Commit`
cycle, so custom capabilities are replayed, audited, and surfaced like built-in
coding tools.

## Cross-Surface Use

Because the server owns the capability registry, no client needs a separate
plugin model. A capability loaded at startup can be invoked from the CLI, shown
in the web console, approved from VS Code, followed from the terminal shell,
and replayed from the ledger.

Related documents:

- [Secure Tool Fabric](secure-tool-fabric.md)
- [Capability Writs](capability-writs.md)
- [Coding Agent](coding-agent.md)
- [Interfaces](interfaces.md)
