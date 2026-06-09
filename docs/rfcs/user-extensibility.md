# Design: User-Extensible OpenThymos

Status: **Draft / design** · Scope: let a *user* expand what the agent can do —
add capabilities, declare custom tools, and write governance rules — **without
writing Rust and without leaving the governance boundary.** Every user-added
capability is still bound by a writ, checked against an effect ceiling, recorded
on the append-only ledger, and replay-verified. We are mostly *surfacing*
machinery that already exists, plus finishing two config seams.

---

## 0. The one rule

> **You extend OpenThymos *through* the boundary, never around it.**

A generic agent framework lets you "add a tool" and then trusts the model to use
it sanely. OpenThymos lets you add a tool too — but the moment it exists it has a
declared **effect class** (`Pure ≤ Read ≤ Write ≤ External ≤ Irreversible`), it
can only run if a **writ** grants that effect and scope, a **policy** permits it,
every call is a **commit** on the ledger, and **replay** can prove what happened.
So "user can modify what the agent controls" never means "user can bypass
governance." That is the whole value, and this design preserves it.

---

## 1. Current state — built vs. surfaced

Grounded in the code; we are closer than it looks.

| Extension surface | Built today | Gap to "a user can do it" |
|---|---|---|
| **Custom tools (manifests)** | `thymos_tools::ToolManifest` (name, version, `effect_class`, `risk_class`, `input_schema`, `executor`) loaded from `THYMOS_TOOL_MANIFEST_DIRS` / `THYMOS_TOOL_MANIFEST_DIR` via `*_runtime_with_capabilities` | **surface only** — works at startup; no UI/docs, no live add |
| **MCP servers** | `McpBridgeTool::spawn` / `McpBridge::spawn_all` — bridge any MCP server's tools as governed tools | **finish** — no config seam; needs Rust today |
| **Policy-as-code** | `thymos_policy::{JsonPolicySet, SignedPolicyBundle}`, `PolicyEngine.policy_set_hash()` (recorded per commit, replay-checked) | **finish** — server always uses `WritAuthorityPolicy`; no load-from-config, no editor |
| **Marketplace** | `POST /marketplace/packages` (publish/unpublish), signed packages | exists; distribution layer over the above |

So the substrate is real. This RFC defines the *user-facing* seams.

---

## 2. Surface A — Custom tools via manifest *(surface what exists)*

A user declares a tool in JSON; the runtime governs it like a native one.

```jsonc
// ~/.thymos/tools/deploy.json
{
  "name": "deploy_staging",
  "version": "1.0.0",
  "description": "Deploy the current build to staging.",
  "effect_class": "irreversible",          // ⇒ runtime escalates to approval
  "risk_class": "high",
  "input_schema": { "type": "object", "properties": { "ref": { "type": "string" } } },
  "executor": { "kind": "shell", "command": ["./deploy.sh", "{ref}"] }
}
```

- **Already loads** from a directory via `THYMOS_TOOL_MANIFEST_DIRS`. Nothing in
  the runtime changes.
- **Finish:** (1) document it; (2) desktop **Tools → Add tool** form that writes
  a manifest into the user's tool dir and points the runtime at it on restart;
  (3) client-side validation mirroring `ToolManifest` (valid effect class,
  schema is valid JSON-Schema, executor present).
- **Stays governed:** the declared `effect_class` is enforced by the compiler's
  effect-ceiling check *before* the executor runs; an `irreversible` tool a writ
  doesn't grant simply never executes, and (with the compensation gate) prompts
  for approval.

## 3. Surface B — MCP servers *(finish a config seam)*

The biggest expansion of *what it can control*: a user lists MCP servers; each
server's tools become governed Thymos tools (databases, browsers, SaaS, cloud).

```jsonc
// ~/.thymos/mcp.json
{
  "servers": {
    "github":   { "command": ["uvx", "mcp-server-github"], "effect_ceiling": "external" },
    "postgres": { "command": ["uvx", "mcp-postgres"],      "effect_ceiling": "write" }
  }
}
```

- **Finish:** a `THYMOS_MCP_CONFIG` (path) loaded at startup that calls
  `McpBridge::spawn_all` per server and registers the discovered tools, tagging
  each with the **operator-declared effect class** for that server (a user
  can't let an MCP tool claim a lower effect than the operator assigned). Add a
  desktop **Tools → Connect MCP server** form.
- **Stays governed:** an MCP tool is a tool — it runs only under a writ that
  grants its scope + effect, every call commits to the ledger, replay sees it.
  MCP servers are subprocesses, so they inherit the existing process-isolation
  posture (`thymos-worker` model). Effect class is **operator-assigned**, not
  self-declared by the MCP server.

## 4. Surface C — Policy-as-code *(finish load + editor)*

The governance dial: the user writes rules the runtime enforces.

```jsonc
// ~/.thymos/policy.json  (a JsonPolicySet)
{
  "rules": [
    { "match": { "tool": "http" },     "decision": "require_approval",
      "when": "host not in [api.github.com]", "channel": "net" },
    { "match": { "tool": "fs_patch" }, "decision": "require_approval",
      "when": "path outside ./",        "channel": "fs" },
    { "match": { "effect": "irreversible" }, "decision": "require_approval", "channel": "ops" }
  ]
}
```

- **Finish:** a `THYMOS_POLICY_BUNDLE` (path) loaded at startup into the
  `PolicyEngine` alongside `WritAuthorityPolicy` (authority is always enforced;
  user policy is *additive* — it can further restrict, never widen). A desktop
  **Policy** tab with an editor + a dry-run ("what would this policy do to this
  past run?" — counterfactual replay, a natural follow-on).
- **Stays governed:** `policy_set_hash` is already recorded on every commit and
  checked by replay, so a run is bound to the exact policy that produced it —
  swapping policy can't retroactively rewrite history. Signed bundles
  (`SignedPolicyBundle`) let an operator pin who authored the rules.

---

## 5. Invariants (must hold for every user extension)

- **Effect ceiling precedes execution.** A user tool / MCP tool runs only if the
  writ grants its (operator-assigned) effect class. No self-declared downgrade.
- **Everything commits.** Every user-tool call is a ledger commit; rejections and
  approvals are ledger events. Replay reconstructs them without re-running tools
  or calling a model.
- **Policy is additive-restrictive.** User policy can deny / require-approval;
  it can never grant authority a writ withholds. `WritAuthorityPolicy` always runs.
- **No phone-home.** MCP servers and tool executors are local subprocesses the
  user configured; the only egress is what the user's tools themselves make,
  bounded by `External`/`Irreversible` effect grants. No analytics, ever.
- **Provenance.** Manifests and policy bundles are content-addressed (and may be
  signed); `policy_set_hash` and tool identity are on the commits they governed.

## 6. Security considerations

User-added capability is *more* power, so the governance must visibly hold:
- An `irreversible` user tool defaults to **approval-gated** (compensation gate),
  so "add a deploy tool" can't silently nuke prod.
- MCP effect class is **operator-assigned** in `mcp.json`, not trusted from the
  server's self-description.
- Tool executors run in the existing process-isolation boundary; shell-template
  executors must escape `{arg}` substitution (no shell injection) — a manifest
  loader hardening task to verify.
- Importing a manifest/policy/MCP config from elsewhere is "running someone
  else's code" — the desktop should show the **effect class + risk** prominently
  before enabling, and prefer signed bundles for shared ones.

## 7. Build order

1. **Custom tools (surface)** — smallest, highest "I can do it" payoff; the
   loader already works. Desktop Add-tool form + validation + docs.
2. **MCP config seam** — biggest capability expansion; `McpBridge` exists, wire
   `THYMOS_MCP_CONFIG` + Connect-MCP UI.
3. **Policy bundle load + editor** — the governance dial; `THYMOS_POLICY_BUNDLE`
   + Policy tab, then dry-run/counterfactual replay.

Each is independently shippable, strictly strengthens (never blurs) the
boundary, and gets its own focused PR. Items 2 and 3 touch runtime startup +
policy, so each gets a short implementation note before code.

## 8. Open questions

- Live reload of tools/policy without a runtime restart, or restart-to-apply
  (simpler, matches today's provider-config flow)?
- Per-tenant policy bundles + MCP sets for the multi-tenant/enterprise path.
- Signing UX for shared manifests/policies (tie into the marketplace).
- Effect-class inference for MCP tools that *do* self-describe richly vs. always
  operator-assigned (lean operator-assigned for safety).
