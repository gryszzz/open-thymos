# OpenThymos Desktop

A downloadable, install-once desktop app for OpenThymos — for people who will
never open a terminal. It is a **client** of the governed-cognition runtime: it
supervises a local `thymos-server`, starts chat sessions as governed runs, and
shows every proposal, verdict, commit, and replay-verification result.

> **It cannot bypass the boundary.** The app never executes a tool, mutates
> world state, or spends budget — all of that stays inside the runtime's
> `Intent → Proposal → Commit` pipeline. The only network egress is to the local
> runtime and (server-side) the provider *you* configured. **No phone-home, no
> analytics.** See [`docs/rfcs/desktop-app.md`](../../../docs/rfcs/desktop-app.md).

## Architecture

- **`src-tauri/`** — the Tauri (Rust) host. It supervises a `thymos-server`
  child process (a bundled sidecar in release builds, or `thymos-server` on
  `PATH` in dev) and exposes `start_runtime` / `stop_runtime` / `runtime_running`
  / `runtime_addr` / `ledger_path` / `get_provider_config` / `set_provider_config`
  commands. It does no governance itself. Provider config is persisted as
  `provider.json` in the app-data dir and injected as env vars (`THYMOS_DEFAULT_*`,
  `OPENAI_API_KEY`/`OPENAI_BASE_URL`, `ANTHROPIC_*`) into the runtime child at
  spawn — the API key is never returned to the webview.
- **`src/`** — the webview UI (plain HTML/CSS/JS, no build step, so it's
  inspectable). Tabs: **Chat** (a message = a governed run, streamed),
  **Runs**, **Providers** (connect any model — Claude, OpenAI, Ollama/LM Studio,
  or any OpenAI-compatible preset/adapter), **Tools**, **Audit** (+ replay
  badge), **Backups**. Every tab is a thin client of an endpoint that already
  exists.

## Status (honest)

- **v1 surfaces wired to real endpoints:** chat/runs, provider/health, tools
  catalog, audit + replay, thin backups, and **in-app provider setup** — the
  Providers tab connects any model (Claude / OpenAI / Ollama / LM Studio / any
  OpenAI-compatible preset or custom base-URL adapter), persisting the choice and
  restarting the runtime to apply it.
- **Deferred (no backend yet, shown as labeled placeholders, never fake
  buttons):** scheduling, inbound message gateways, memory, skills (designed in
  [`docs/rfcs/skills.md`](../../../docs/rfcs/skills.md), not yet implemented). See
  also the desktop RFC §4.
- **Not yet built/verified here:** this is a scaffold. It has **not** been
  `cargo build`/`tauri build`-compiled in this environment (the Tauri toolchain
  + crates were not fetched), and there is **no signed installer / download link
  yet**. The README Download section lands only when the release pipeline
  attaches real signed bundles.

## Develop

Prereqs: Rust, Node, and the [Tauri v2 system deps](https://tauri.app/start/prerequisites/).

```bash
# 1) Have a runtime binary on PATH for dev:
cargo install --path ../../crates/thymos-server   # provides `thymos-server`

# 2) Run the desktop app (hot-reloads the webview):
cd clients/desktop
npm install
npm run icon            # generate icons from thymosG.PNG (first run)
npm run dev
```

Start the runtime from the app's top bar, then send a task in **Chat**.

## Build installers

```bash
npm run build           # -> src-tauri/target/release/bundle/{dmg,msi,appimage}
```

For a frictionless install on macOS/Windows these need **code-signing +
notarization** (certs as CI secrets) — tracked alongside the release-pipeline
work in the RFC §5. The tag-driven `.github/workflows/release.yml` is where the
bundle job and Release-asset upload will be added.
