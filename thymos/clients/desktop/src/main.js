// OpenThymos Desktop — webview client.
//
// This file ONLY observes and proposes: it calls the runtime's HTTP/SSE API.
// It never executes effects. A chat message is a `POST /runs`; the "reply" is
// the streamed governance feed (intents, verdicts, commits) from the runtime.
// See docs/rfcs/desktop-app.md.

const invoke = window.__TAURI__?.core?.invoke;

// Resolve the runtime address from the Tauri host (falls back for browser dev).
let BASE = "http://127.0.0.1:3001";
async function resolveBase() {
  try {
    if (invoke) BASE = await invoke("runtime_addr");
  } catch (_) {}
}

const $ = (id) => document.getElementById(id);
const api = (path) => `${BASE}${path}`;

async function getJSON(path) {
  const r = await fetch(api(path));
  if (!r.ok) throw new Error(`${r.status} ${path}`);
  return r.json();
}
async function postJSON(path, body) {
  const r = await fetch(api(path), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!r.ok) throw new Error(`${r.status} ${path}`);
  return r.json().catch(() => ({}));
}

/* ---------- tabs ---------- */
document.querySelectorAll(".tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    $(`tab-${btn.dataset.tab}`).classList.add("active");
    if (btn.dataset.tab === "runs") loadRuns();
    if (btn.dataset.tab === "providers") loadHealth();
    if (btn.dataset.tab === "tools") loadTools();
  });
});

/* ---------- runtime supervision + health ---------- */
async function refreshStatus() {
  let running = false;
  try { running = invoke ? await invoke("runtime_running") : false; } catch (_) {}
  let health = null;
  try { health = await getJSON("/health"); } catch (_) {}
  const up = !!health && health.status === "ok";
  $("dot").className = "dot " + (up ? "dot-on" : "dot-off");
  $("statusText").textContent = up
    ? "runtime: live"
    : running
    ? "runtime: starting…"
    : "runtime: stopped";
  if (health) {
    $("provider").textContent = `provider: ${health.default_provider}${
      health.cognition_live ? "" : " (mock)"
    }`;
    $("ledger").textContent = `ledger: ${health.ledger}`;
  }
}

$("startBtn").addEventListener("click", async () => {
  try {
    if (invoke) await invoke("start_runtime");
    // Poll briefly until /health answers.
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 400));
      await refreshStatus();
      if ($("dot").classList.contains("dot-on")) break;
    }
  } catch (e) { alert("Could not start runtime: " + e); }
});
$("stopBtn").addEventListener("click", async () => {
  try { if (invoke) await invoke("stop_runtime"); } catch (_) {}
  await refreshStatus();
});

/* ---------- chat = a governed run ---------- */
const feed = $("chatFeed");
function pushLine(cls, text) {
  const div = document.createElement("div");
  div.className = `line ${cls}`;
  div.textContent = text;
  feed.appendChild(div);
  feed.scrollTop = feed.scrollHeight;
  return div;
}

// Map a runtime log entry to a glyph + css class (the shared visual language).
function glyphFor(entry) {
  const p = entry.phase, lvl = entry.level;
  if (lvl === "error") return ["✕", "deny"];
  if (lvl === "warning") return ["⏸", "suspend"];
  if (p === "intent") return ["◆", "intent"];
  if (p === "proposal") return ["▸", "permit"];
  if (p === "result" || p === "execution")
    return lvl === "success" ? ["✓", "commit"] : ["·", "sys"];
  return ["·", "sys"];
}

let currentStream = null;
let renderedUpTo = 0;

function renderSnapshot(s) {
  // Snapshot carries the full log; render only newly-arrived entries.
  (s.log || []).forEach((e) => {
    if (e.idx < renderedUpTo) return;
    renderedUpTo = e.idx + 1;
    const [g, cls] = glyphFor(e);
    const detail = e.detail ? `  ${e.detail}` : "";
    pushLine(cls, `${g} ${e.title}${detail}`);
  });
  if (s.status === "waiting_approval") showApproval(s.run_id);
  if (["completed", "failed", "cancelled"].includes(s.status) && s.final_answer)
    pushLine("sys", `— ${s.status}: ${s.final_answer}`);
}

function showApproval(runId) {
  if (document.querySelector(`.approval-row[data-run="${runId}"]`)) return;
  const row = document.createElement("div");
  row.className = "approval-row";
  row.dataset.run = runId;
  row.innerHTML = `<span class="q">⏸ approval required — channel</span>`;
  const chan = document.createElement("input");
  chan.value = "ops";
  chan.style.maxWidth = "120px";
  const approve = document.createElement("button");
  approve.textContent = "Approve";
  const deny = document.createElement("button");
  deny.textContent = "Deny";
  deny.className = "ghost";
  const act = async (decision) => {
    try {
      await postJSON(`/runs/${runId}/approvals/${chan.value}`, { decision });
      pushLine("sys", `— ${decision} (${chan.value})`);
      row.remove();
    } catch (e) { alert("Approval failed: " + e); }
  };
  approve.onclick = () => act("approve");
  deny.onclick = () => act("deny");
  row.append(chan, approve, deny);
  feed.appendChild(row);
  feed.scrollTop = feed.scrollHeight;
}

$("composer").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const task = $("taskInput").value.trim();
  if (!task) return;
  $("taskInput").value = "";
  pushLine("you", `you ▸ ${task}`);
  try {
    const { run_id } = await postJSON("/runs", { task });
    pushLine("sys", `— run ${run_id}`);
    if (currentStream) currentStream.close();
    renderedUpTo = 0;
    currentStream = new EventSource(api(`/runs/${run_id}/execution/stream`));
    currentStream.onmessage = (m) => {
      try { renderSnapshot(JSON.parse(m.data)); } catch (_) {}
    };
    currentStream.onerror = () => { currentStream && currentStream.close(); };
  } catch (e) {
    pushLine("deny", `✕ could not start run: ${e}. Is the runtime live?`);
  }
});

/* ---------- runs history ---------- */
async function loadRuns() {
  const el = $("runsList");
  el.innerHTML = "<div class='hint'>loading…</div>";
  try {
    const runs = await getJSON("/runs");
    el.innerHTML = "";
    const list = Array.isArray(runs) ? runs : runs.runs || [];
    if (!list.length) { el.innerHTML = "<div class='hint'>no runs yet</div>"; return; }
    list.forEach((r) => {
      const glyph = r.status === "completed" ? "✓" : r.status === "failed" ? "✕" : "▸";
      const cls = r.status === "completed" ? "commit" : r.status === "failed" ? "deny" : "permit";
      const commits = r.summary?.commits ?? "";
      const div = document.createElement("div");
      div.className = "item";
      div.innerHTML =
        `<span class="glyph ${cls}">${glyph}</span>` +
        `<b>${escapeHtml(r.task || "")}</b>` +
        `<span class="meta">${r.trajectory_id?.slice(0, 8) || ""}` +
        `${commits !== "" ? " · " + commits + " commits" : ""}</span>`;
      div.style.cursor = "pointer";
      div.onclick = () => openAudit(r.trajectory_id);
      el.appendChild(div);
    });
  } catch (e) { el.innerHTML = `<div class='hint'>could not load runs: ${e}</div>`; }
}
$("refreshRuns").addEventListener("click", loadRuns);

/* ---------- providers: live truth + connect any model ---------- */
async function loadHealth() {
  const el = $("providerCard");
  try {
    const h = await getJSON("/health");
    el.innerHTML =
      `<div><b>Default provider:</b> ${h.default_provider} ` +
      `<span class="badge ${h.cognition_live ? "ok" : "bad"}">` +
      `${h.cognition_live ? "live" : "mock — no key set"}</span></div>` +
      `<div><b>Mode:</b> ${h.mode}</div>` +
      `<div><b>Ledger backend:</b> ${h.ledger}</div>`;
  } catch (e) {
    el.innerHTML = `<div class='hint'>runtime not reachable — start it from the top bar (${e})</div>`;
  }
  await loadProviderForm();
}
$("refreshHealth").addEventListener("click", loadHealth);

// Populate the connect-a-model form from the host's stored config. The key is
// never returned — we only learn whether one is set, and reflect that in the
// placeholder.
async function loadProviderForm() {
  if (!invoke) {
    $("pfStatus").textContent = "provider editing needs the desktop app";
    $("providerForm")
      .querySelectorAll("input,button")
      .forEach((n) => (n.disabled = true));
    return;
  }
  try {
    const cfg = await invoke("get_provider_config");
    $("pfProvider").value = cfg.provider || "";
    $("pfModel").value = cfg.model || "";
    $("pfBaseUrl").value = cfg.base_url || "";
    $("pfKey").value = "";
    $("pfKey").placeholder = cfg.key_set
      ? "•••••••• stored — blank keeps it"
      : "leave blank to keep current key";
  } catch (_) {}
}

// Preset registry mirror (thymos_cognition::presets) so picking a provider
// auto-fills its endpoint, default model, and which key it needs. Keep in sync
// with crates/thymos-cognition/src/presets.rs.
const PRESETS = {
  anthropic:   { url: "", model: "claude-sonnet-4-6", key: "ANTHROPIC_API_KEY" },
  openai:      { url: "https://api.openai.com/v1", model: "gpt-4o-mini", key: "OPENAI_API_KEY" },
  groq:        { url: "https://api.groq.com/openai/v1", model: "llama-3.3-70b-versatile", key: "GROQ_API_KEY" },
  openrouter:  { url: "https://openrouter.ai/api/v1", model: "openai/gpt-4o-mini", key: "OPENROUTER_API_KEY" },
  together:    { url: "https://api.together.xyz/v1", model: "meta-llama/Llama-3.3-70B-Instruct-Turbo", key: "TOGETHER_API_KEY" },
  deepseek:    { url: "https://api.deepseek.com/v1", model: "deepseek-chat", key: "DEEPSEEK_API_KEY" },
  mistral:     { url: "https://api.mistral.ai/v1", model: "mistral-large-latest", key: "MISTRAL_API_KEY" },
  xai:         { url: "https://api.x.ai/v1", model: "grok-2-latest", key: "XAI_API_KEY" },
  fireworks:   { url: "https://api.fireworks.ai/inference/v1", model: "accounts/fireworks/models/llama-v3p3-70b-instruct", key: "FIREWORKS_API_KEY" },
  nvidia:      { url: "https://integrate.api.nvidia.com/v1", model: "meta/llama-3.3-70b-instruct", key: "NVIDIA_API_KEY" },
  cerebras:    { url: "https://api.cerebras.ai/v1", model: "llama-3.3-70b", key: "CEREBRAS_API_KEY" },
  gemini:      { url: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-2.0-flash", key: "GEMINI_API_KEY" },
  perplexity:  { url: "https://api.perplexity.ai", model: "sonar", key: "PERPLEXITY_API_KEY" },
  huggingface: { url: "https://router.huggingface.co/v1", model: "meta-llama/Llama-3.3-70B-Instruct", key: "HF_TOKEN" },
  ollama:      { url: "http://localhost:11434/v1", model: "llama3.2", key: "" },
  lmstudio:    { url: "http://localhost:1234/v1", model: "local-model", key: "" },
  vllm:        { url: "http://localhost:8000/v1", model: "default", key: "" },
  llamacpp:    { url: "http://localhost:8080/v1", model: "default", key: "" },
  localai:     { url: "http://localhost:8080/v1", model: "gpt-4", key: "" },
  mock:        { url: "", model: "", key: "" },
};

// When a provider is picked, prefill its endpoint/model/key hint so each one is
// one-click. Only fills empty fields, so a typed override is never clobbered.
$("pfProvider")?.addEventListener("input", () => {
  const p = PRESETS[$("pfProvider").value.trim().toLowerCase()];
  if (!p) return;
  $("pfModel").placeholder = p.model || "provider default";
  if (p.url) {
    $("pfBaseUrl").placeholder = p.url;
    if (!$("pfBaseUrl").value) $("pfBaseUrl").value = p.url;
  }
  const local = p.url.startsWith("http://localhost");
  $("pfStatus").textContent = p.key
    ? `needs ${p.key} (read server-side)`
    : local ? "local — no key needed" : "no key needed";
});

$("providerForm").addEventListener("submit", async (e) => {
  e.preventDefault();
  if (!invoke) return;
  const status = $("pfStatus");
  const provider = $("pfProvider").value.trim();
  if (!provider) {
    status.textContent = "pick a provider first";
    return;
  }
  status.textContent = "saving…";
  try {
    await invoke("set_provider_config", {
      provider,
      model: $("pfModel").value,
      baseUrl: $("pfBaseUrl").value,
      apiKey: $("pfKey").value,
    });
    // Apply by restarting the supervised runtime (env is read at spawn).
    const wasRunning = await invoke("runtime_running").catch(() => false);
    if (wasRunning) await invoke("stop_runtime").catch(() => {});
    await invoke("start_runtime").catch(() => {});
    status.textContent = "saved — runtime restarting…";
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 400));
      await refreshStatus();
      if ($("dot").classList.contains("dot-on")) break;
    }
    await loadHealth();
    status.textContent = `now using ${provider}`;
  } catch (err) {
    status.textContent = "save failed: " + err;
  }
});

/* ---------- tools (marketplace catalog, read-only) ---------- */
async function loadTools() {
  const el = $("toolsList");
  el.innerHTML = "<div class='hint'>loading…</div>";
  try {
    const res = await getJSON("/marketplace/search");
    const pkgs = res.packages || res.results || (Array.isArray(res) ? res : []);
    el.innerHTML = "";
    if (!pkgs.length) { el.innerHTML = "<div class='hint'>no tools published</div>"; return; }
    pkgs.forEach((p) => {
      const div = document.createElement("div");
      div.className = "item";
      div.innerHTML =
        `<span class="glyph permit">▣</span><b>${escapeHtml(p.name || "")}</b>` +
        `<span class="meta">${escapeHtml(p.description || p.version || "")}</span>`;
      el.appendChild(div);
    });
  } catch (e) { el.innerHTML = `<div class='hint'>could not load tools: ${e}</div>`; }
}
$("refreshTools").addEventListener("click", loadTools);

/* ---------- audit + replay verdict ---------- */
async function openAudit(runId) {
  document.querySelector('.tab[data-tab="audit"]').click();
  $("auditRunId").value = runId || "";
  if (runId) loadAudit(runId);
}
async function loadAudit(runId) {
  const trail = $("auditTrail");
  const badge = $("replayBadge");
  trail.innerHTML = "<div class='hint'>loading…</div>";
  badge.innerHTML = "";
  try {
    const entries = await getJSON(`/audit/entries?run_id=${encodeURIComponent(runId)}`);
    const list = Array.isArray(entries) ? entries : entries.entries || [];
    trail.innerHTML = "";
    list.forEach((e) => {
      const div = document.createElement("div");
      div.className = "item";
      div.innerHTML =
        `<span class="meta">#${e.seq}</span><b>${escapeHtml(e.kind)}</b>` +
        `<span class="meta">${escapeHtml((e.commit_id || e.id || "").slice(0, 12))}</span>`;
      trail.appendChild(div);
    });
    // Replay always verifies integrity; a 200 means the chain folded cleanly.
    try {
      const rep = await getJSON(`/runs/${runId}/replay`);
      badge.innerHTML =
        `<span class="badge ok">replay ✓ verified</span> ` +
        `<span class="meta">${rep.commits_replayed} commits · ` +
        `${rep.rejected_proposals} rejected · head ${(rep.head_commit || "").slice(0, 12)}</span>`;
    } catch (_) {
      badge.innerHTML = `<span class="badge bad">replay could not verify</span>`;
    }
  } catch (e) { trail.innerHTML = `<div class='hint'>could not load trail: ${e}</div>`; }
}
$("auditForm").addEventListener("submit", (ev) => {
  ev.preventDefault();
  const id = $("auditRunId").value.trim();
  if (id) loadAudit(id);
});

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

/* ---------- boot ---------- */
(async function boot() {
  await resolveBase();
  await refreshStatus();
  setInterval(refreshStatus, 4000);
})();
