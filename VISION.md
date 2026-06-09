# OpenThymos — Vision

> **The governed execution runtime for AI agents.**
> Cognition proposes. The runtime governs. The ledger records.

This file is the single source of truth for *what OpenThymos is and why*. Every
other surface — the website, the README, the desktop app, the CLI — speaks this
same message. If a surface drifts from this spine, the surface is wrong.

---

## The one sentence

OpenThymos is the runtime that makes an AI agent's actions **authorized,
auditable, and replayable** — provable, not trust-the-prompt.

## The spine (say it everywhere, verbatim)

**Cognition proposes. The runtime governs. The ledger records.**

A model cannot call a tool, mutate state, spend budget, delegate authority, or
erase history — not by convention, by runtime semantics. Every effect passes
through a typed pipeline bound to a signed capability writ, a policy trace, and
an append-only execution ledger:

```
Intent → Proposal → Writ → Commit → Replay
```

| Stage | Who | Authority |
|---|---|---|
| **Intent** | emitted by cognition | none — content-addressed, no execution rights |
| **Proposal** | compiled by the runtime | bound to a signed **Writ** + policy trace |
| **Commit** | written to the ledger | the only thing that mutates world state |
| **Replay** | folds the ledger | proves what happened — no model, no re-run |

## The one differentiator

**It's the only agent runtime where every action is provable.** Other frameworks
let a model "just do things" and ask you to trust the prompt. OpenThymos turns
every agent action into a countable, signed, replayable record — a proof you can
hand an auditor, reconcile a bill against, or replay after an incident.

---

## Who it's for

- **Agents that touch money, infra, or data** — a signed writ (tool scopes,
  effect ceiling, budget, time window) decides what actually runs, enforced *in
  the compiler*, not by prompt convention.
- **Audit & compliance for AI actions** — every action is an append-only,
  hash-chained ledger entry: what was done, under whose authority, which policy
  permitted it.
- **Post-incident forensics** — deterministic replay reconstructs exact world
  state and rejects compiler/policy drift and unsigned commits.
- **Multi-tenant agent platforms** — tenant-scoped writs and strict-subset
  delegation, so hosted agents can't exceed granted authority.

## How you use it

One runtime, many surfaces — all clients of the same governed API and ledger:

- **CLI** (`thymos`) — scriptable, server, terminal shell.
- **Desktop app** — local-first chat, live runs, the Mind reasoning view, audit +
  replay; connect any model; your keys never leave your machine.
- **HTTP/SSE API + Rust SDK** — drive it from any service.

The CLI and the desktop share one runtime, one ledger, one provider/key, one
approval queue (see `docs/using-cli-and-desktop.md`).

## What you can extend — through the boundary, never around it

A user expands *what the agent can do* without writing Rust, and everything added
stays governed (effect ceiling, writ, ledger, replay):

- **Custom tools** — declare a tool (command/HTTP + effect class + schema); an
  irreversible one auto-requires approval. *(Shipped.)*
- **MCP servers** — connect any MCP server; its tools become governed Thymos
  tools. *(Next.)*
- **Policy-as-code** — write permit / deny / require-approval rules the runtime
  enforces. *(Next.)*

See `docs/rfcs/user-extensibility.md`.

---

## Where it is (honest)

- **Done & proven:** the governed kernel (Intent→Proposal→Commit, signed writs,
  policy traces, append-only ledger, deterministic replay), multi-agent
  delegation, the CLI + desktop surfaces (linked), custom tools, easy install.
- **Next (what finishes the framework):** MCP + policy-as-code extensibility,
  signed desktop installers, the enterprise/metering surface.

Source of truth for current reality is **[STATUS.md](STATUS.md)** (CI-proven vs
gated vs not-built). This file is the *why*; STATUS is the *what's real today*.

## The values that never bend

- **No phone-home, no silent egress.** Telemetry is operator-owned. Never add
  analytics or usage beacons.
- **Replay never trusts a live provider or re-runs a tool.** The ledger is the
  truth.
- **No authority asserted inline.** Cognition is untrusted input; authority is a
  signed writ.
- **Candor over polish.** We state what is proven, what is gated, and what is not
  built. Credibility is the product.
