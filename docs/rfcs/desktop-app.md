# Design: OpenThymos Desktop

Status: **Draft / design** · Scope: a downloadable, consumer-installable desktop
app — "one place to install OpenThymos" for people who will never open a
terminal. Like every surface in
[UI Surfaces & Programmability](ui-surfaces-and-programmability.md), the desktop
app is a **client**: it observes the ledger and *proposes* runs through the
existing HTTP/SSE API. It changes **nothing** in the runtime. The runtime stays
the single source of truth; the app only starts work, watches verdicts, and
clears gates.

This RFC follows the design style of the UI-surfaces RFC rather than the
protocol template, because there are **no ledger, replay, writ, or policy
changes** here — it is packaging and a GUI over surfaces that already exist.

---

## 0. The one rule the GUI must not break

> **Cognition proposes. The runtime governs. The ledger records.**

A pretty desktop app is the easiest place to accidentally blur that line — to
let a button "just do the thing" outside the `Intent → Proposal → Commit`
pipeline. It must not. Concretely:

- The app **never** executes a tool, mutates world state, or spends budget
  itself. It only calls `POST /runs`, streams `/execution/stream`, and clears
  gates via `POST /runs/:id/approvals/:channel`.
- Every screen that shows an action shows its **verdict** (`permit` / `deny` /
  `require-approval`), its **writ**, and a path to the **ledger** + **replay ✓**.
  If a screen hides the boundary, it is off-brand and wrong.
- **No phone-home.** The app ships with zero analytics/telemetry beacons
  (CLAUDE.md core value). The only network egress is (a) to the local runtime it
  manages, and (b) to the LLM provider the *user* configured, with the *user's*
  key, read server-side.

---

## 1. Why a desktop app at all (and why Tauri)

The CLI, TUI, and VS Code surfaces all assume a developer. "Normal people" — an
auditor, a compliance reviewer, a solo operator, someone evaluating the project
— need **install → open → it works**, with no `cargo`, no `$ANTHROPIC_API_KEY`,
no `docker run`.

**Tauri** (chosen): a Rust host process + a system-webview UI.

- The Rust side **embeds or supervises `thymos-server`** in-process / as a child,
  so the app *is* the runtime — no separate install step. (`thymos-client` and
  `thymos-server` are already libraries; the app links them.)
- Tiny native installers (`.dmg` / `.msi` / `.AppImage`, ~10MB) → a **real
  download**, signable and notarizable.
- The webview reuses the same render model as the VS Code audit webview and the
  planned web operator surface — one UI codebase across three surfaces.
- Matches the project's Rust + governance ethos; no 100MB Electron runtime.

Rejected: **Electron** (shares more TS with the web surface but ~100MB bundles,
HTTP-only to the runtime, heavier egress surface); **a hosted web app only**
(no "install once, runs locally, your keys never leave your box" story, which is
the whole point for the privacy-conscious audience).

---

## 2. Feature map — what's real now vs. what needs a backend

The request named eleven things. **Honesty is the credibility signal** (CLAUDE.md):
some are wired today; several are *not built in the runtime at all* and must not
ship as buttons-over-nothing. This table is the contract.

| Requested feature | Backend reality today | Verdict for v1 |
|---|---|---|
| **One place to install** | `thymos-server` + `thymos-client` are libs; release.yml builds per-OS binaries | **v1** — Tauri bundles + supervises the server |
| **Provider setup** | Real: preset registry (`thymos_cognition::presets`, ~20 providers), key auto-detect, `GET /health` → `cognition_live`, `thymos providers` | **v1** — GUI key entry + provider picker → server env |
| **Chat sessions** | Real: `POST /runs`, `GET /runs/:id/execution/stream` (SSE), `GET /runs`, `cancel`/`resume`. A "session" = a run/trajectory | **v1** — chat = a governed run, streamed |
| **Tools** | Real: typed tool contracts, sandboxed `shell`/`http`/`fs_*`/`test_run`/`mcp_bridge`, marketplace API (`/marketplace/*`) | **v1 (read/browse + scope picker)**; install-from-marketplace **v1.1** |
| **Logs** | Real: `/runs/:id/events`, `/audit/entries`, `/replay`, world projection | **v1** — event log + audit trail + replay badge |
| **Backups** | Partial: ledger is a SQLite file; `verify_integrity` exists. No export command/UI | **v1 (thin)** — copy DB + run integrity check; restore = point at a file |
| **Profile switching** | **Not a backend concept.** Tenant scoping exists on writs; no "profile" object | **v1 (local-only)**; durable profiles need backend → §4 |
| **Scheduling** | **Not built.** No scheduler; runs are one-shot | **Deferred** → needs `thymos-schedule` (§4). Stub UI must say "not yet". |
| **Message gateways** | **Partial/ambiguous.** There is an *API-key gateway* (`/usage`, `thymos-gateway.db`); inbound chat gateways (Slack/Telegram/email) are **not built** | **Deferred** → §4. Do **not** imply Slack/email works. |
| **Memory management** | **Not built.** Per-run world projection exists; no cross-session memory store | **Deferred** → needs `thymos-memory` (§4) |
| **Skills** | **Not built** as a runtime concept (distinct from tools/marketplace packages) | **Deferred** → needs a definition + backend (§4) |

**The rule:** a feature is in the v1 UI **only** if it maps to a real endpoint
above. Deferred features get a single honest "Coming soon — tracked in
`desktop-app.md` §4" placeholder, never a dead button that pretends to work.

---

## 3. v1 — what ships first (all real today)

A five-tab app, every tab a thin client of an existing endpoint:

```
┌ OPEN THYMOS ─────────────────────────── ●live  anthropic  ledger:sqlite ┐
│ ◆ Chat   ▸ Runs   ⚙ Providers   🧰 Tools   📜 Audit                       │
├──────────────────────────────────────────────────────────────────────────┤
│  you ▸ "summarize the repo and open a PR"                                  │
│  ◆ intent   grep "fn main"                                                 │
│  ▸ permit   [WritAuthority]  writ ab12…   budget 12k/100k ▓▓░░░            │
│  ✓ commit   grep → world c9f2…   14ms                                      │
│  ⏸ approval "irreversible: git push"        [ Approve ]  [ Deny ]          │
└──────────────────────────────────────────────────────────────────────────┘
```

1. **First-run setup** — pick a provider, paste a key (or "use mock — no key,
   deterministic"). Writes to the supervised server's env; `GET /health` confirms
   `cognition_live`. This is "provider setup," done once.
2. **Chat** — a message starts a run (`POST /runs`); the reply *is* the streamed
   governance feed (intents, verdicts, commits). Suspended `⏸` proposals render
   inline **Approve/Deny** (`POST .../approvals/:channel`). This is the product:
   you watch authority being spent, and you authorize the risky steps.
3. **Runs** — history (`GET /runs`), status glyphs, cancel/resume, open in Audit.
4. **Providers** — the preset list + which key is set + live/mock badge.
5. **Tools** — browse governed tool contracts + the marketplace catalog
   (read-only v1); show each tool's effect class so the writ scope picker is
   honest.
6. **Audit** — the existing audit render (commit chain, rejections, delegations,
   policy decision per commit) + **replay verdict badge**. Reuses the VS Code
   webview render model.
7. **Backups (thin)** — "Back up ledger" copies the SQLite file to a chosen
   location and runs `verify_integrity`, reporting the chain head + entry count;
   "Restore" points the runtime at a chosen ledger file. No new ledger semantics.

Zero server changes are required for everything in §3 (same conclusion the
UI-surfaces RFC reached for its v1).

---

## 4. Deferred features — each needs a real backend first

These are the requested features that **do not exist** and therefore must not
ship as working UI. Each is a small RFC of its own; listing the shape so the
desktop roadmap is honest:

- **`thymos-schedule`** — a scheduler that fires `POST /runs` on a cron/interval
  under a *named, scoped writ*. The schedule itself should be a ledger-recorded
  object so "why did the agent run at 3am" is auditable. Until this exists, the
  Scheduling tab is a labeled placeholder.
- **Message gateways (inbound)** — Slack/Telegram/email/webhook adapters that
  turn an inbound message into a governed run and stream the verdict back. The
  *API-key gateway* (`/usage`) is unrelated plumbing; do not conflate. Needs its
  own RFC (auth, per-channel writs, rate limits, no silent egress).
- **`thymos-memory`** — a cross-session memory store the agent can *propose*
  reads/writes against, governed like any other effect (memory writes are
  commits; reads are tool calls under scope). Per-run world projection is **not**
  this. Must preserve replay determinism.
- **Skills** — needs a definition distinct from tools/marketplace packages
  (likely: a signed bundle of a system prompt + tool scopes + policy defaults).
  Until defined, "Skills" is not in the UI.
- **Durable profiles** — a profile = (provider + default model + default writ
  scopes + tenant + ledger path). v1 can keep these **local to the app** (a
  config file the app owns); a server-side profile/tenant object is a later RFC
  if profiles need to be shared or enforced server-side.

Sequencing recommendation: ship §3 (all real), then `thymos-schedule` (highest
"normal person" value, smallest backend), then `thymos-memory`, then inbound
gateways, then skills.

---

## 5. The download story (honest, no vaporware)

CLAUDE.md: **never vaporware.** A "Download" button in the README must point at a
real artifact or not exist. Plan:

1. Add a `thymos-desktop` Tauri crate/app under `clients/desktop` (Rust host +
   webview UI), supervising `thymos-server`.
2. Extend `.github/workflows/release.yml` (already tag-driven, already builds
   linux/macOS/windows binaries) with a Tauri bundle job that produces
   `.dmg` / `.msi` / `.AppImage` and attaches them to the **GitHub Release**.
   Code-signing/notarization is a prerequisite for a frictionless macOS/Windows
   install and is its own task (needs signing certs as CI secrets).
3. **Only after** the first tagged release carries those assets, add a
   **Download** section to `README.md` linking to
   `releases/latest` (per-OS). Until then: no button, or a clearly-labeled
   "desktop app — in progress, see `docs/rfcs/desktop-app.md`" line. A dead
   download link is exactly the credibility hit this project refuses to take.
4. `STATUS.md` gets a row: desktop app = *scaffolded / unreleased* until assets
   ship, then *released vX.Y.Z*. The README and STATUS must not claim a download
   that the Releases page can't back.

---

## 6. Invariants (don't regress)

- The app is a **read-and-propose client**; it never executes effects, mutates
  projected state, or spends budget outside the runtime's pipeline.
- **No phone-home / no silent egress.** No analytics. Egress is only to the
  user's local runtime and the user's chosen provider with the user's key.
- Replay is unaffected: the app produces no ledger entries of its own and never
  re-runs a tool or calls a provider on the replay path.
- A v1 UI element exists **only** if it maps to a live endpoint in §2; deferred
  features are labeled placeholders, never functional-looking dead ends.
- The **README/STATUS download claim tracks the Releases page** exactly.

---

## 7. Unresolved questions

- **Server lifecycle:** embed `thymos-server` in-process vs. supervise it as a
  child binary the installer also ships? (Child binary = reuse release.yml
  artifacts + crash isolation; in-process = one binary, simpler.)
- **Signing/notarization:** whose certs, stored as which CI secrets? Without
  this, macOS Gatekeeper / Windows SmartScreen make "normal people install" rough.
- **Ledger location & multi-profile storage** on a desktop (one SQLite file per
  profile under the app data dir?).
- **Provider keys at rest:** OS keychain (Keychain / Credential Manager /
  libsecret) vs. the server's existing server-side env handling.
- Reuse the marketing three.js ledger-DAG scene in the Audit tab, or a lighter
  2D graph (same open question as the web surface)?
