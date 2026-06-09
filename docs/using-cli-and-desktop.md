# Using the terminal and the desktop together (with a real AI)

OpenThymos has two human surfaces — the **CLI** (`thymos`) and the **desktop
app** — and they are *not* separate apps with separate state. They are both
**clients of one `thymos-server`**. Whoever owns `http://localhost:3001` owns the
runtime, the provider/key, and the durable ledger; both surfaces read and drive
*the same* truth.

This is verifiable: start a run in the terminal (`thymos run "…"`) and it appears
in the desktop's **Runs** tab; approve a suspended run from the desktop and the
terminal's `thymos audit` shows who approved it. Same ledger, same runs, same
verdicts.

---

## 1. One runtime, two surfaces

```
        ┌────────────────────────────────────────────┐
        │            thymos-server  :3001             │
        │   runtime · provider/key · DURABLE ledger   │
        └──────────────▲──────────────────▲───────────┘
                       │ HTTP/SSE          │ HTTP/SSE
              ┌────────┴───────┐   ┌───────┴────────┐
              │   Desktop app  │   │  CLI  (thymos) │
              │  (webview)     │   │  THYMOS_URL→3001│
              └────────────────┘   └────────────────┘
```

- The CLI defaults to `http://localhost:3001` (override with `--url` or
  `THYMOS_URL`).
- The desktop **adopts** a server already listening on 3001 instead of starting
  a second one. So either of these works, and the *other* surface just joins:
  - **Open the desktop app** → it starts the server (with a durable ledger in the
    OS app-data dir) → then run `thymos …` in a terminal and it's the same runtime.
  - **Start `thymos-server` yourself** → open the desktop app → it adopts your
    server.
- **Rule of thumb:** one server owns 3001. Don't run two; they'd both want the
  port. Let one own it and point everything else at it.

---

## 2. Connect a real model (both surfaces inherit it)

The provider and API key are **server-side** — set once, used by every surface.

- **Desktop:** Providers tab → pick a provider (Claude, OpenAI, Ollama / LM
  Studio local, or any OpenAI-compatible adapter) → paste a key → it restarts the
  runtime with that provider. `thymos run …` in a terminal now uses the same model.
- **CLI / manual server:** start the server with a key in its environment, e.g.
  `ANTHROPIC_API_KEY=sk-… thymos-server`. The desktop adopts it and chats use it.

Keys never cross to the model with authority and never leave your machine; only
the provider *name* crosses the wire. `GET /health` (status bar in the app,
`thymos health` in the terminal) reports `cognition_live: true` once a real model
is wired (and `mock` until then — same deterministic mock on both surfaces).

---

## 3. How the agent responds to an action

With a real model, a task runs the governed loop — **the same on both surfaces**,
because it's the same runtime:

1. **You** send a task (desktop chat, or `thymos run "…"`).
2. **Cognition proposes.** The model emits *intents* (tool calls). It never
   executes anything itself — it has no execution authority by type.
3. **The runtime governs each intent** → a verdict:
   - **✓ permit** — within the writ's scope, effect ceiling, budget, time window,
     and policy. The tool runs; a **commit** lands on the ledger.
   - **✕ deny** — out of scope / over budget / policy `Deny`. Recorded as a
     *rejection*; nothing executes.
   - **⏸ require-approval** — irreversible or policy-flagged. The run **suspends**
     and waits for a human.
4. **You see it stream** as it happens: `◆ intent  ▸ proposal  ✓ commit`
   (or `✕ rejected`, `⏸ suspended`), then the final answer.
5. **Approvals are cross-surface.** A suspended run can be cleared from *either*
   side and the other sees it:
   - Desktop: the **Approve / Deny** card.
   - Terminal: `thymos approve <run-id> <channel>` (add `--deny` to refuse).
6. **Everything is on the shared ledger.** Inspect the same trail from either
   surface — the desktop **Audit** tab or `thymos audit <run-id>` — and verify it
   with `thymos replay <run-id>` (or the app's replay badge). Replay re-folds the
   ledger independently; it never calls the model or re-runs a tool.

So the agent's "response to an action" is always: **propose → the runtime decides
→ the result + verdict are recorded and streamed back.** It cannot act outside a
permitted, recorded proposal — that's the guarantee, and it holds identically
whether you drove the run from the terminal or the app.

---

## 4. Quick recipe

```bash
# Terminal A — own the runtime with a real model + a durable ledger
ANTHROPIC_API_KEY=sk-… THYMOS_LEDGER_PATH=~/.thymos/ledger.db thymos-server

# Open the desktop app — it ADOPTS the server above (shared runtime + ledger).

# Terminal B — drive the same runtime; the app shows these runs live
thymos run "map this repo and open a PR" --follow --scopes fs_read,grep
thymos audit <run-id>     # the full governance trail (same as the app's Audit tab)
thymos replay <run-id>    # verify the ledger folds to its world
```

The desktop app and the CLI are windows onto one governed runtime — pick whichever
fits the moment; the runs, the ledger, the approvals, and the proof are shared.
