# Design: UI Surfaces & Programmability

Status: **Draft / design** · Scope: how humans *and* machines drive and observe
OpenThymos. Nothing here changes the runtime — every surface is a **client** of
the existing HTTP/SSE API and the ledger. The runtime stays the single source of
truth; surfaces only *observe* and *propose*.

---

## 0. The one idea every surface must show

OpenThymos is not "another agent runner." Its differentiator is **governance you
can watch and prove**. So every surface is organized around the same spine:

```
Intent ──▶ Proposal ──▶ [ permit | deny | require-approval ] ──▶ Commit ──▶ Ledger ──▶ Replay ✓
                              the boundary, visible
```

A generic agent UI shows "the agent did things." A *Thymos* UI shows **what was
proposed, what the runtime allowed and why, what it refused, and proof the record
is intact.** If a screen doesn't surface a verdict, a writ, or the ledger, it's
off-brand.

Shared visual language (already live in the CLI, reuse everywhere):
`◆ Intent` · `▸ Proposal` · `✓ Commit` (green) · `✕ Rejected` (red) ·
`⏸ Suspended` (amber) · violet brand, star-cyan accents.

---

## 1. Data sources (what every surface reads)

All three surfaces are built from endpoints that already exist:

| Need | Endpoint |
|------|----------|
| Liveness · live-vs-mock · ledger backend | `GET /health` |
| Create work | `POST /runs` |
| Operator-truth run state | `GET /runs/:id/execution` |
| **Live updates** | `GET /runs/:id/execution/stream` (SSE) |
| Ledger entries (the trail) | `GET /audit/entries?run_id=` |
| Verify + fold | `GET /runs/:id/replay` |
| World projection | `GET /runs/:id/world` |
| Clear a gate | `POST /runs/:id/approvals/:channel` |
| Run history | `GET /runs` |

No surface needs new server work to reach **v1**. (The one fix that unblocked
live UIs — the execution SSE stream closing on terminal status — is already in.)

---

## 2. Surface A — TUI governance cockpit (`thymos watch`)

The most on-brand surface for a CLI-first runtime, and the cheapest to build
(ratatui + the SSE client we already have). A full-screen, live cockpit.

```
┌ OPEN THYMOS ── watch ────────────────────────── ●live  anthropic  ledger:sqlite ┐
│ RUNS                          │ run 9e88… · "map the repo and summarize"         │
│ ▸ 9e88  ✓ commit  step 4/16   │ ◆ intent    step4  grep "fn main"               │
│   a17c  ⏸ approval ops        │ ▸ proposal  permit  [WritAuthority]  writ ab12…  │
│   4f01  ✓ done   12 commits   │ ✓ commit    grep    14ms  → world c9f2…          │
│   2bd0  ✕ failed              │ ▸ proposal  DENY    effect ceiling: External>RW  │
│                               │ ⏸ proposal  approval ops "irreversible: delete"  │
│ ─ writ ────────────────────── │ ───────────────────────────────────────────────│
│ budget  tok 12k/100k  ▓▓░░░   │ [a]pprove  [d]eny  [enter] audit  [w] world      │
│ scopes  fs_read grep kv_*     │ commits 4  rejections 1  pending 1               │
└───────────────────────────────┴─────────────────────────────────────────────────┘
```

- **Left:** run list (live status glyphs) + the selected run's **writ** — budget
  burn-down bars, granted scopes, effect ceiling. *This is the unique panel:* you
  watch authority being spent.
- **Right:** the live governance feed (the SSE snapshots), each line a verdict.
- **Inline action:** `a`/`d` resolves a suspended proposal via the approvals
  endpoint — human-in-the-loop without leaving the terminal.
- **`enter`** drops into the existing `thymos audit` render for the run.

Build: new `thymos-tui` crate (or `thymos watch` behind a feature), ~1 screen,
reuses `thymos-client` + the SSE stream. No server changes.

---

## 3. Surface B — VS Code sidebar (grow `clients/vscode`)

For developers who live in the editor. A `thymos` activity-bar view with three
tree sections + one webview.

```
THYMOS ▾
├─ ● runtime   localhost:3001 · anthropic · sqlite
├─ RUNS
│  ├─ ✓ map the repo…            9e88  4 commits
│  ├─ ⏸ deploy staging           a17c  needs: ops ▸ [Approve] [Deny]
│  └─ ✕ refactor auth            2bd0
├─ THIS RUN  (selected)
│  ├─ ◆ intent  grep "fn main"
│  ├─ ▸ permit  [WritAuthority]
│  ├─ ✓ commit  grep → c9f2…
│  └─ ✕ deny    effect ceiling
└─ [ Open Audit ]  [ New Run… ]
```

- **Approve/Deny as inline tree buttons** on suspended runs — the killer editor
  feature (review an agent's risky action without context-switching).
- **`fs_patch` commits** can offer "Open Diff" against the workspace file.
- **Audit** opens a webview rendering the same governance trail (reuse the render
  model) with the replay verdict badge.
- **CodeLens (stretch):** above a function the agent touched, "governed by writ
  ab12 · committed c9f2 · replay ✓".

Build: TypeScript extension using the SSE stream; the webview reuses the audit
render. No server changes; this is the surface with the most product leverage.

---

## 4. Surface C — Web operator overview

For operators / reviewers who aren't in a terminal, and for **sharing** a run.

```
OPEN THYMOS · operator
┌ fleet ───────────────────────────────────────────────────────────┐
│ runs today 142   commits 1.2k   rejections 38   pending 3 ⏸       │
│ providers: anthropic ●  openai ●  ollama ●        ledger: postgres │
├ timeline ─────────────────────────────────────────────────────────┤
│ ▸▸✓✓✕▸✓⏸✓✓✓✕✓  (governance events, color = verdict)              │
├ run drill-down ───────────────────────────────────────────────────┤
│ writ · budget · scopes  │  ledger DAG  │  replay: verified ✓        │
└───────────────────────────────────────────────────────────────────┘
```

- **Fleet view:** aggregate verdict counters, provider health, ledger backend.
- **Governance timeline:** a scrub-able stream of verdicts across all runs — the
  "what is my agent fleet allowed to do, and what did it try" board.
- **Run drill-down:** writ + ledger DAG (the 3D motif from the marketing site,
  reused as a real ledger graph) + replay status.
- **Approvals queue:** a shared inbox of `⏸` proposals for a team to clear.

Build: served from `thymos-server` (or a static SPA hitting the API). Heaviest of
the three; do it after the TUI proves the data model.

---

## 5. Recommended sequence

1. **TUI `thymos watch`** — cheapest, most on-brand, proves the live data model.
2. **VS Code sidebar** — highest product leverage (inline approvals where devs work).
3. **Web overview** — operators + sharing, once the model is proven.

Each is independently shippable and adds zero runtime risk (read-only clients +
the existing approvals endpoint).

---

## 6. Programmability — machines *and* people

> "Can we make this machine-programmable to take real action, usable by machines
> or with people?" — yes; here's the surface area, all of it already present.

**Machines drive it three ways:**

- **HTTP/SSE API** — `POST /runs` to start work, consume `/execution/stream` for
  live state, `GET /replay` for proof. Any service/agent/cron can drive a governed
  run and get back a verifiable trail. This is the integration boundary.
- **CLI as a scriptable tool** — every command is pipe-friendly and TTY-aware
  (clean plain output when piped). `Run started: <id>` is a stable parse line.
- **Rust SDK** — `thymos-client` for in-process embedding.

**The agent takes real-world action through governed tools** (`thymos tools`):

- `shell` (run commands) · `http` (call any API) · `fs_read`/`fs_patch` (edit
  files) · `test_run` · and **`mcp_bridge`** — connect *any* MCP server, which
  opens the entire MCP ecosystem (databases, browsers, SaaS, cloud) as governed
  tools. Custom tools arrive via signed **manifests** + the marketplace.
- Every call is checked against the writ's **effect ceiling**
  (`Read ≤ Write ≤ External ≤ Irreversible`) *before* it runs — so "real-world
  power" is always bounded by an explicit, signed grant.

**People stay in the loop by design:**

- **Approval gates** — `Irreversible`-class (or policy-flagged) proposals
  *suspend* and wait for a human via `POST /runs/:id/approvals/:channel` (the
  approve/deny in every surface above). M-of-N quorum is supported.
- **Audit for humans** — `thymos audit` / the webview render the whole trail in
  human terms: what was done, under whose authority, which policy decided it.

**Machines + people together:** an automated system starts a run; the agent
proposes a wire transfer / a prod deploy / a destructive migration; the runtime
**suspends** it; a person approves in the TUI, the sidebar, or the web queue; the
ledger records who approved, when, and the replayable result. That hand-off —
autonomous proposal, human authorization, provable record — is the product.

---

## 7. Open questions

- Auth model for the web surface (the API key gateway exists; SSO for operators?).
- Ledger-DAG rendering: reuse the marketing three.js scene as a real graph, or a
  lighter 2D view for the operator board?
- TUI: standalone `thymos-tui` crate vs. `thymos watch` subcommand behind a
  feature flag (leaning subcommand for discoverability).
