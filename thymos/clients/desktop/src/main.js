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
    if (btn.dataset.tab === "skills") loadSkills();
    if (btn.dataset.tab === "tools") loadTools();
    if (btn.dataset.tab === "backups") loadBackups();
    if (btn.dataset.tab === "advanced") loadAdvanced();
    if (btn.dataset.tab === "audit" && !$("auditRunId").value.trim()) loadExplorer();
  });
});

/* ---------- Advanced tab: live runtime inspection (all real state) ---------- */
async function loadAdvanced() {
  // Runtime card: health truth + where this app is pointed.
  const rt = $("advRuntime");
  if (rt) {
    let health = null;
    try { health = await getJSON("/health"); } catch (_) {}
    let appVer = "";
    try { appVer = await window.__TAURI__?.app?.getVersion?.(); } catch (_) {}
    rt.innerHTML = health
      ? `<div class="state-grid">` +
        `<span class="dim">runtime</span><span><span class="badge ok">live</span> <span class="dim">${escapeHtml(BASE)}</span></span>` +
        `<span class="dim">mode</span><span>${escapeHtml(health.mode || "?")}</span>` +
        `<span class="dim">ledger</span><span>${escapeHtml(health.ledger || "?")}</span>` +
        (appVer ? `<span class="dim">app version</span><span>v${escapeHtml(appVer)}</span>` : "") +
        `</div>`
      : `<div class="hint">Runtime not running — press <b>Start runtime</b> (top right).</div>`;
  }
  // Model & routing card: the provider this runtime answers with.
  const pv = $("advProvider");
  if (pv) {
    let cfg = null, health = null;
    try { if (invoke) cfg = await invoke("get_provider_config"); } catch (_) {}
    try { health = await getJSON("/health"); } catch (_) {}
    const provider = cfg?.provider || health?.default_provider || "—";
    const live = !!health?.cognition_live;
    pv.innerHTML =
      `<div class="state-grid">` +
      `<span class="dim">provider</span><span>${escapeHtml(provider)} ` +
      `<span class="badge ${live ? "ok" : "bad"}">${live ? "live model" : "mock"}</span></span>` +
      `<span class="dim">default model</span><span>${escapeHtml(cfg?.model || "(provider default)")}</span>` +
      (cfg?.base_url ? `<span class="dim">base url</span><span>${escapeHtml(cfg.base_url)}</span>` : "") +
      (cfg ? `<span class="dim">api key</span><span>${cfg.key_set ? "stored locally" : "not set"}</span>` : "") +
      `</div>`;
    if (cfg?.provider) fillModelList(cfg.provider);
  }
  // Authority card: what new chats are allowed to do, right now.
  const au = $("advAuthority");
  if (au) {
    const scopes = effectiveScopes();
    const skills = selectedSkills();
    au.innerHTML =
      `<div class="state-grid">` +
      `<span class="dim">mode</span><span>${escapeHtml(currentMode())}</span>` +
      `<span class="dim">grants</span><span>${scopes.length ? scopes.map(escapeHtml).join(" · ") : "everything"}</span>` +
      `<span class="dim">active skills</span><span>${skills.length ? skills.map(escapeHtml).join(" · ") : "none"}</span>` +
      `<span class="dim">risk gate</span><span>high-risk tools pause for approval</span>` +
      `</div>`;
  }
  // Ledger card: the real on-disk chain this runtime writes.
  const lp = $("advLedgerPath");
  if (lp && invoke) {
    try { lp.textContent = (await invoke("ledger_path")) || "(runtime not started yet)"; }
    catch (_) { lp.textContent = "—"; }
  }
}
$("refreshAdvanced")?.addEventListener("click", loadAdvanced);

/* ---------- Advanced Mode ---------- */
// Off by default: normal users never see raw runtime data, schemas, writs, or
// commits. The toggle reveals everything tagged `.adv-only` (see the UX
// philosophy doc). Persisted locally.
const ADV_KEY = "thymos.advanced.v1";
(function initAdvanced() {
  const on = (() => { try { return localStorage.getItem(ADV_KEY) === "1"; } catch (_) { return false; } })();
  document.body.classList.toggle("advanced", on);
  const t = $("advToggle");
  if (t) {
    t.checked = on;
    t.addEventListener("change", () => {
      document.body.classList.toggle("advanced", t.checked);
      try { localStorage.setItem(ADV_KEY, t.checked ? "1" : "0"); } catch (_) {}
      // If an advanced-only tab was active and we just hid it, fall back to Chat.
      const active = document.querySelector(".tab.active");
      if (!t.checked && active && active.classList.contains("adv-only")) {
        document.querySelector('.tab[data-tab="chat"]').click();
      }
    });
  }
})();

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
    $("provider").textContent = `provider: ${health.default_provider}`;
    $("ledger").textContent = `ledger: ${health.ledger}`;
    const badge = $("liveBadge");
    if (badge) {
      badge.textContent = health.cognition_live ? "live model" : "mock";
      badge.className = "badge " + (health.cognition_live ? "ok" : "bad");
    }
    // Model name comes from the host's provider config (server-side), not /health.
    if (invoke) {
      try {
        const cfg = await invoke("get_provider_config");
        const m = (cfg && cfg.model) || "";
        $("model").textContent = `model: ${m || "(provider default)"}`;
      } catch (_) { $("model").textContent = "model: —"; }
    }
  } else {
    const badge = $("liveBadge");
    if (badge) { badge.textContent = ""; badge.className = "badge"; }
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

// Governance refusals are the runtime working, not the tool breaking — turn
// the raw reason into plain English with the action that unblocks it. Shared
// by the chat feed and the Audit narrative.
function governanceHint(e) {
  const raw = e.detail || "";
  if (/AuthorityVoid/.test(raw)) {
    const tool = ((e.title || "").match(/for (\S+)/) || [])[1] || "that tool";
    return `${tool} isn't allowed in ${currentMode()} mode. The runtime blocked it (nothing is broken) — switch the mode in the left rail (Auto allows everything; dangerous tools still ask first) and try again.`;
  }
  if (/PolicyDenied/.test(raw)) {
    return "policy denied this action — the decision is recorded in the run's audit trail.";
  }
  if (/BudgetExhausted|budget/.test(raw) && e.level === "error") {
    return "this run's budget (tokens / tool calls / time) ran out — start a fresh message to continue.";
  }
  return "";
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
let activeRunId = null;

// Authority is a SAVED property of the model, not a per-chat choice. The grant
// chips + default skill are configured once (left rail) and persisted; every
// chat on this model inherits them. Empty grant selection lets the server grant
// `*` (all tools).
const GRANTS_KEY = "thymos.grants.v1";
const SKILLS_KEY = "thymos.skills.v1"; // array of active skill names (multi-skill)
const MODEL_KEY = "thymos.chatModel.v1"; // per-chat model override
document.querySelectorAll("#grants .chip").forEach((chip) =>
  chip.addEventListener("click", () => {
    chip.classList.toggle("on");
    try { localStorage.setItem(GRANTS_KEY, JSON.stringify(selectedScopes())); } catch (_) {}
  }));
function selectedScopes() {
  return [...document.querySelectorAll("#grants .chip.on")].map((c) => c.dataset.scope);
}
function savedScopes() {
  try { const a = JSON.parse(localStorage.getItem(GRANTS_KEY) || "null"); return Array.isArray(a) ? a : null; }
  catch (_) { return null; }
}

/* ---------- authority modes (Auto / Edit / Plan / Custom) ---------- */
// The standard AI-tool posture, mapped onto real writ scopes: each mode is a
// set of tools derived from the live registry's effect classes. Auto grants
// everything (the risk gate still pauses dangerous tools for approval).
const MODE_KEY = "thymos.authmode.v1";
let toolsCache = []; // [{name, effect_class}] from the live registry
// Offline fallbacks so modes work before the registry loads.
const FALLBACK_READ = ["fs_read", "grep", "list_files", "repo_map", "kv_get", "memory_recall"];
const FALLBACK_WRITE = ["fs_patch", "test_run", "kv_set", "memory_store"];
function currentMode() {
  try { return localStorage.getItem(MODE_KEY) || "auto"; } catch (_) { return "auto"; }
}
function setMode(m) {
  try { localStorage.setItem(MODE_KEY, m); } catch (_) {}
  renderModeUI();
}
function toolsByEffect(effects) {
  if (!toolsCache.length) {
    return effects.includes("write") ? FALLBACK_READ.concat(FALLBACK_WRITE) : FALLBACK_READ;
  }
  return toolsCache
    .filter((t) => effects.includes(String(t.effect_class || "").toLowerCase()))
    .map((t) => t.name);
}
// The writ scopes a new chat message will request, given the active mode.
// Empty array = the server grants `*` (everything).
function effectiveScopes() {
  switch (currentMode()) {
    case "auto": return [];
    case "plan": return toolsByEffect(["read"]);
    case "edit": return toolsByEffect(["read", "write"]);
    default: return selectedScopes(); // custom = the chips
  }
}
function renderModeUI() {
  const m = currentMode();
  document.querySelectorAll("#modeRow .mode-chip").forEach((b) =>
    b.classList.toggle("on", b.dataset.mode === m));
  const grants = $("grants");
  if (grants) grants.hidden = m !== "custom";
}
document.querySelectorAll("#modeRow .mode-chip").forEach((b) =>
  b.addEventListener("click", () => setMode(b.dataset.mode)));
// Rebuild the grant chips from the LIVE tool registry, so every registered
// tool is grantable — the static HTML chips are only an offline fallback.
// (Previously the chips were a hardcoded subset: tools like repo_map existed
// and showed as registered, but could never be granted — runs rejected them
// with AuthorityVoid and the tool looked broken instead of ungranted.)
function renderGrantChips(names) {
  const wrap = $("grants");
  if (!wrap || !names.length) return;
  const saved = savedScopes();
  const on = new Set(saved ?? selectedScopes());
  [...wrap.querySelectorAll(".chip")].forEach((c) => c.remove());
  names.forEach((n) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "chip" + (on.has(n) ? " on" : "");
    b.dataset.scope = n;
    b.textContent = n;
    b.addEventListener("click", () => {
      b.classList.toggle("on");
      try { localStorage.setItem(GRANTS_KEY, JSON.stringify(selectedScopes())); } catch (_) {}
    });
    wrap.appendChild(b);
  });
}
// Does the current authority mode authorize a tool? Empty scope list means
// the server grants `*` (everything) — mirror the writ's prefix-glob match.
function scopeAuthorizes(name) {
  const scopes = effectiveScopes();
  if (!scopes.length) return true;
  return scopes.some((s) =>
    s.endsWith("*") ? name.startsWith(s.slice(0, -1)) : name === s);
}
// Active skills for the chat — multiple can be on at once; they combine (the
// runtime unions their tools and ANDs their limits, so authority only narrows).
function selectedSkills() {
  return [...document.querySelectorAll("#skillChips .chip.on")].map((c) => c.dataset.skill);
}
function savedSkills() {
  try { const a = JSON.parse(localStorage.getItem(SKILLS_KEY) || "[]"); return Array.isArray(a) ? a : []; }
  catch (_) { return []; }
}
// Restore saved grants + active skills (called at boot and after skills load).
function restoreDefaults() {
  try {
    const saved = JSON.parse(localStorage.getItem(GRANTS_KEY) || "null");
    if (Array.isArray(saved)) {
      document.querySelectorAll("#grants .chip").forEach((c) =>
        c.classList.toggle("on", saved.includes(c.dataset.scope)));
    }
  } catch (_) {}
  const on = savedSkills();
  document.querySelectorAll("#skillChips .chip").forEach((c) =>
    c.classList.toggle("on", on.includes(c.dataset.skill)));
  const cm = $("chatModel");
  if (cm) {
    try { cm.value = localStorage.getItem(MODEL_KEY) || ""; } catch (_) {}
    cm.onchange = () => { try { localStorage.setItem(MODEL_KEY, cm.value.trim()); } catch (_) {} };
  }
}

// Toggle Send/Stop + a live "working" pulse while a run is in flight. The input
// stays enabled (you can draft the next message); a second run is blocked by the
// activeRunId guard, not by disabling the field.
function setBusy(on) {
  $("sendBtn").hidden = on;
  $("chatStop").hidden = !on;
  const regen = $("regenBtn");
  if (regen) regen.disabled = on || !lastTask;
  let w = document.getElementById("workingLine");
  if (on && !w) {
    w = document.createElement("div");
    w.id = "workingLine";
    w.className = "working";
    w.innerHTML = `<span class="dots"><i></i><i></i><i></i></span><span>governing…</span>`;
    feed.appendChild(w);
    feed.scrollTop = feed.scrollHeight;
  } else if (!on && w) {
    w.remove();
  }
}

function renderSnapshot(s) {
  // The busy indicator mirrors the runtime's actual state ("Planning step 2",
  // "Executing fs_patch", "Awaiting approval on high-risk") instead of a
  // static "governing…" that reads as frozen.
  const wl = document.querySelector("#workingLine span:last-child");
  if (wl && s.operator_state) wl.textContent = s.operator_state.toLowerCase() + "…";
  // Snapshot carries the full log; render only newly-arrived entries.
  (s.log || []).forEach((e) => {
    if (e.idx < renderedUpTo) return;
    renderedUpTo = e.idx + 1;
    const [g, cls] = glyphFor(e);
    // Chat is a feed, not a log viewer: keep status lines one-line-ish. The
    // full detail stays in the run's audit trail.
    let detail = e.detail ? `  ${e.detail}` : "";
    if (detail.length > 300) detail = detail.slice(0, 300) + "…";
    pushLine(cls, `${g} ${e.title}${detail}`);
    const hint = governanceHint(e);
    if (hint) pushLine("sys", `   ↳ ${hint}`);
  });
  if (s.status === "waiting_approval") {
    // Show WHAT is being approved: the latest approval request's reason
    // carries the tool + a compact args preview from the compiler.
    const req = [...(s.log || [])].reverse()
      .find((e) => (e.title || "").startsWith("Approval requested"));
    showApproval(s.run_id, s.pending_channel, req?.detail || "");
  }
  if (["completed", "failed", "cancelled"].includes(s.status)) {
    if (s.final_answer) {
      pushAnswer(s.status, s.final_answer);
      // Persist the answer with its ledger link (run/trajectory) for audit+replay.
      addMessage("agent", s.final_answer, {
        status: s.status, run_id: s.run_id, trajectory_id: s.trajectory_id,
      });
    }
    activeRunId = null;
    // Clear the live marker so Mind stops showing this run as in-flight; the
    // finished turn is now a persisted reply it picks up normally.
    if (window.thymosActiveRun === s.run_id) window.thymosActiveRun = null;
    setBusy(false);
  }
}

// The model's answer, with a copy button (typical chat affordance).
function pushAnswer(status, text) {
  const wrap = document.createElement("div");
  wrap.className = "answer";
  const body = document.createElement("div");
  body.className = "answer-body";
  body.textContent = text;
  const copy = document.createElement("button");
  copy.className = "ghost copy";
  copy.textContent = "Copy";
  copy.onclick = async () => {
    try { await navigator.clipboard.writeText(text); copy.textContent = "Copied"; }
    catch (_) { copy.textContent = "—"; }
    setTimeout(() => (copy.textContent = "Copy"), 1200);
  };
  const head = document.createElement("div");
  head.className = "answer-head";
  head.textContent = status === "completed" ? "✓ answer" : `— ${status}`;
  head.appendChild(copy);
  wrap.append(head, body);
  feed.appendChild(wrap);
  feed.scrollTop = feed.scrollHeight;
}

/* ---------- chat history (local-first, persists across restarts) ---------- */
const CHATS_KEY = "thymos.chats.v1";
let chats = [];
let activeChat = null;
const WELCOME_HTML = feed.innerHTML; // captured before any clearing

function loadChats() {
  try { chats = JSON.parse(localStorage.getItem(CHATS_KEY) || "[]"); } catch (_) { chats = []; }
}
function saveChats() { try { localStorage.setItem(CHATS_KEY, JSON.stringify(chats)); } catch (_) {} }
function curChat() { return chats.find((c) => c.id === activeChat); }
// Mind reads the whole conversation through this hook: every message in the
// active chat, with the run each agent reply came from. Lets Mind render the
// full session (messages + each run's governed actions), not one run.
window.thymosSession = function () {
  const c = curChat();
  if (!c) return null;
  return {
    id: c.id,
    title: c.title || "",
    messages: (c.messages || []).map((m) => ({
      role: m.role,
      text: String(m.text || ""),
      run_id: m.run_id || null,
      status: m.status || null,
    })),
  };
};

function renderWelcome() { feed.innerHTML = WELCOME_HTML; }

function renderChatList() {
  const el = $("chatList");
  if (!el) return;
  el.innerHTML = "";
  if (!chats.length) { el.innerHTML = "<div class='hint chat-empty'>no chats yet</div>"; return; }
  chats.forEach((c) => {
    const row = document.createElement("div");
    row.className = "chat-item" + (c.id === activeChat ? " on" : "");
    row.onclick = () => openChat(c.id);
    const t = document.createElement("span");
    t.className = "ci-title";
    t.textContent = c.title || "Untitled";
    const del = document.createElement("button");
    del.className = "ci-del"; del.textContent = "×"; del.title = "delete chat";
    del.onclick = (e) => { e.stopPropagation(); deleteChat(c.id); };
    row.append(t, del);
    el.appendChild(row);
  });
}

function newChatSession(focus = true) {
  if (currentStream) currentStream.close();
  activeRunId = null; renderedUpTo = 0; setBusy(false);
  const c = { id: Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
              title: "New chat", ts: Date.now(), messages: [] };
  chats.unshift(c); activeChat = c.id; saveChats();
  renderChatList(); renderWelcome();
  if (focus) $("taskInput").focus();
}

function openChat(id) {
  if (currentStream) currentStream.close();
  activeRunId = null; renderedUpTo = 0; setBusy(false);
  activeChat = id; renderChatList();
  const c = curChat();
  feed.innerHTML = "";
  if (!c || !c.messages.length) { renderWelcome(); return; }
  c.messages.forEach((m) => {
    if (m.role === "user") pushBubble(m.text);
    else pushAnswer(m.status || "completed", m.text);
  });
  $("taskInput").focus();
}

function deleteChat(id) {
  chats = chats.filter((c) => c.id !== id);
  saveChats();
  if (activeChat === id) activeChat = chats[0] ? chats[0].id : null;
  if (!activeChat) newChatSession(false); else openChat(activeChat);
}

// Append a message to the active chat (creating one if needed) and persist.
function addMessage(role, text, extra) {
  let c = curChat();
  if (!c) { newChatSession(false); c = curChat(); }
  c.messages.push(Object.assign({ role, text }, extra || {}));
  if (role === "user" && (!c.title || c.title === "New chat")) c.title = text.slice(0, 42);
  c.ts = Date.now(); saveChats(); renderChatList();
}

$("newChat")?.addEventListener("click", () => newChatSession());

// Stop: cancel the in-flight run via the real cancel endpoint.
$("chatStop")?.addEventListener("click", async () => {
  if (!activeRunId) return;
  const runId = activeRunId;
  try { await postJSON(`/runs/${runId}/cancel`, {}); pushLine("sys", "— cancel requested"); }
  catch (e) {
    const msg = String(e);
    if (msg.includes("409") || msg.includes("404")) {
      // The run already reached a terminal state — nothing to cancel. Sync
      // the chat with reality instead of surfacing it as a failure.
      pushLine("sys", "— run already finished");
      try { renderSnapshot(await getJSON(`/runs/${runId}/execution`)); } catch (_) {}
      if (activeRunId === runId) { activeRunId = null; setBusy(false); }
    } else {
      pushLine("deny", `✕ cancel failed: ${e}`);
    }
  }
});

function showApproval(runId, channel, why) {
  if (document.querySelector(`.approval-row[data-run="${runId}"]`)) return;
  const row = document.createElement("div");
  row.className = "approval-row";
  row.dataset.run = runId;
  row.innerHTML = `<span class="q">⏸ approval required — channel</span>`;
  if (why) {
    const w = document.createElement("div");
    w.className = "approval-why";
    w.textContent = String(why).slice(0, 400);
    row.appendChild(w);
  }
  const chan = document.createElement("input");
  chan.value = channel || "ops"; // prefilled with the actual pending channel
  chan.style.maxWidth = "120px";
  const approve = document.createElement("button");
  approve.textContent = "Approve";
  const deny = document.createElement("button");
  deny.textContent = "Deny";
  deny.className = "ghost";
  const act = async (decision) => {
    try {
      await postJSON(`/runs/${runId}/approvals/${chan.value}`, { approve: decision === "approve" });
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

let lastTask = null;

// A user message rendered as a chat bubble. Authority (skill + grants) is a
// saved model property shown in the left rail, not per-message clutter.
function pushBubble(text) {
  const wrap = document.createElement("div");
  wrap.className = "bubble you-bubble";
  const body = document.createElement("div");
  body.className = "bubble-body";
  body.textContent = text;
  wrap.appendChild(body);
  feed.appendChild(wrap);
  feed.scrollTop = feed.scrollHeight;
}

// Start a governed run from a task string. Shared by Send and Regenerate.
async function startRun(task) {
  if (!task || activeRunId) return;
  const welcome = feed.querySelector(".welcome");
  if (welcome) welcome.remove();
  // Authority comes from the active skills + the authority mode (left rail).
  const skills = selectedSkills();
  const scopes = effectiveScopes();
  lastTask = task;
  pushBubble(task);
  addMessage("user", task);
  setBusy(true);
  try {
    const body = { task };
    if (skills.length) body.skills = skills;
    if (scopes.length) body.tool_scopes = scopes;
    const model = $("chatModel")?.value.trim();
    if (model) body.model = model; // per-chat model override (else server default)
    // Multi-turn memory: send this chat's recent transcript (minus the
    // message we just appended) so the model can follow the conversation.
    // The server caps and composes it; each run stays its own trajectory.
    const hist = (curChat()?.messages || [])
      .slice(0, -1)
      .filter((m) => m.role === "user" || m.role === "agent")
      .slice(-12)
      .map((m) => ({
        role: m.role === "agent" ? "assistant" : "user",
        text: String(m.text || "").slice(0, 1500),
      }));
    if (hist.length) body.history = hist;
    const { run_id } = await postJSON("/runs", body);
    activeRunId = run_id;
    window.thymosActiveRun = run_id; // Mind opens on the chat's current run
    pushLine("sys", `— run ${String(run_id).slice(0, 8)}`);
    if (currentStream) currentStream.close();
    renderedUpTo = 0;
    openRunStream(run_id);
  } catch (e) {
    pushLine("deny", `✕ could not start run: ${e}. Is the runtime live?`);
    activeRunId = null;
    setBusy(false);
  }
}

// Attach the execution SSE stream for a run. If the stream drops mid-run
// (sleep, runtime restart, proxy hiccup) the chat must never stick in "busy":
// recover the snapshot over plain HTTP, render it, and re-attach while the
// run is still going.
function openRunStream(run_id) {
  currentStream = new EventSource(api(`/runs/${run_id}/execution/stream`));
  currentStream.onmessage = (m) => {
    try { renderSnapshot(JSON.parse(m.data)); } catch (_) {}
  };
  currentStream.onerror = async () => {
    if (currentStream) currentStream.close();
    if (activeRunId !== run_id) return; // run already concluded
    try {
      const snap = await getJSON(`/runs/${run_id}/execution`);
      renderSnapshot(snap); // clears busy state if the run is terminal
      if (activeRunId === run_id) setTimeout(() => {
        if (activeRunId === run_id) openRunStream(run_id);
      }, 1000);
    } catch (_) {
      pushLine("deny", "✕ lost connection to the runtime");
      activeRunId = null;
      setBusy(false);
    }
  };
}

$("composer").addEventListener("submit", (ev) => {
  ev.preventDefault();
  const task = $("taskInput").value.trim();
  if (!task || activeRunId) return;
  $("taskInput").value = "";
  autoGrow($("taskInput"));
  startRun(task);
});

// Enter sends; Shift+Enter inserts a newline (ChatGPT/Claude convention).
function autoGrow(t) {
  if (!t || t.tagName !== "TEXTAREA") return;
  t.style.height = "auto";
  t.style.height = Math.min(t.scrollHeight, 160) + "px";
}
$("taskInput").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    $("composer").requestSubmit();
  }
});
$("taskInput").addEventListener("input", () => autoGrow($("taskInput")));

// Regenerate: re-run the last task (new trajectory, fresh writ).
$("regenBtn")?.addEventListener("click", () => {
  if (activeRunId || !lastTask) return;
  startRun(lastTask);
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
      div.onclick = () => openAudit(r.run_id);
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
  await loadCatalog();
}
$("refreshHealth").addEventListener("click", loadHealth);

// Provider catalog: every supported provider at a glance — local vs cloud, its
// default model + endpoint, and whether it needs a key. The active provider is
// highlighted with its real key status. Click a row to prefill the form below.
async function loadCatalog() {
  const el = $("providerCatalog");
  if (!el || typeof PRESETS === "undefined") return;
  let active = "";
  let keySet = false;
  try { active = (await getJSON("/health")).default_provider || ""; } catch (_) {}
  try { if (invoke) keySet = !!(await invoke("get_provider_config")).key_set; } catch (_) {}
  el.innerHTML = "";
  for (const [id, p] of Object.entries(PRESETS)) {
    const local = (p.url || "").startsWith("http://localhost") || (!p.url && id === "mock");
    const isActive = id === active;
    const needsKey = !!p.key;
    const keyBadge = isActive
      ? (needsKey
          ? `<span class="badge ${keySet ? "ok" : "bad"}">${keySet ? "key set" : "no key"}</span>`
          : `<span class="badge ok">no key needed</span>`)
      : `<span class="badge">${needsKey ? "needs key" : "no key"}</span>`;
    const host = (p.url || "").replace(/^https?:\/\//, "");
    const div = document.createElement("div");
    div.className = "item" + (isActive ? " active" : "");
    div.style.cursor = "pointer";
    div.innerHTML =
      `<span class="glyph ${isActive ? "commit" : "sys"}">${isActive ? "◆" : "◈"}</span>` +
      `<b>${id}</b><span class="tag">${local ? "local" : "cloud"}</span>${keyBadge}` +
      `<span class="meta">${p.model || "—"}${host ? " · " + host : ""}</span>`;
    div.onclick = () => {
      const inp = $("pfProvider");
      inp.value = id;
      inp.dispatchEvent(new Event("input"));
      inp.scrollIntoView({ behavior: "smooth", block: "center" });
      inp.focus();
    };
    el.appendChild(div);
  }
}

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

// Common models per provider — suggestions for the Model fields (provider form,
// chat, skill). "Discover models" replaces these with the provider's live list;
// these give every provider a sensible menu without a round-trip, and manual
// entry always works. (Kept conservative; not authoritative.)
const MODELS = {
  anthropic: ["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5",
    "claude-opus-4-1", "claude-3-7-sonnet-latest", "claude-3-5-haiku-latest"],
  openai: ["gpt-4o", "gpt-4o-mini", "o3-mini", "o1", "gpt-4.1", "gpt-4-turbo"],
  groq: ["llama-3.3-70b-versatile", "llama-3.1-8b-instant", "mixtral-8x7b-32768", "qwen-2.5-32b"],
  gemini: ["gemini-2.0-flash", "gemini-2.0-flash-lite", "gemini-1.5-pro", "gemini-1.5-flash"],
  deepseek: ["deepseek-chat", "deepseek-reasoner"],
  mistral: ["mistral-large-latest", "mistral-small-latest", "codestral-latest"],
  xai: ["grok-2-latest", "grok-2-vision-latest", "grok-beta"],
  openrouter: ["anthropic/claude-sonnet-4-6", "openai/gpt-4o", "google/gemini-2.0-flash", "meta-llama/llama-3.3-70b-instruct"],
  together: ["meta-llama/Llama-3.3-70B-Instruct-Turbo", "Qwen/Qwen2.5-72B-Instruct-Turbo", "deepseek-ai/DeepSeek-V3"],
  perplexity: ["sonar", "sonar-pro", "sonar-reasoning"],
  ollama: ["llama3.2", "llama3.1", "qwen2.5", "mistral", "phi3", "gemma2", "deepseek-r1"],
  lmstudio: ["local-model"],
};

// Populate the shared model datalist with a provider's common models (fallback
// to its single default). Used by every Model field via list="modelList".
function fillModelList(provider) {
  const dl = $("modelList");
  if (!dl) return;
  const id = (provider || "").trim().toLowerCase();
  const list = MODELS[id] || (PRESETS[id] && PRESETS[id].model ? [PRESETS[id].model] : []);
  dl.innerHTML = list.map((m) => `<option value="${escapeHtml(m)}"></option>`).join("");
}

// When a provider is picked, prefill its endpoint/model/key hint + model menu.
$("pfProvider")?.addEventListener("input", () => {
  const id = $("pfProvider").value.trim().toLowerCase();
  const p = PRESETS[id];
  if (!p) return;
  fillModelList(id);
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

// Resolve the base URL: the typed field, else the provider's preset default.
function resolveBaseUrl() {
  const url = $("pfBaseUrl").value.trim();
  if (url) return url;
  const p = $("pfProvider").value.trim().toLowerCase();
  return (typeof PRESETS !== "undefined" && PRESETS[p] && PRESETS[p].url) || "";
}

// Test connection — host pings the provider's /models with the SAVED key.
$("pfTest")?.addEventListener("click", async () => {
  if (!invoke) return;
  const provider = $("pfProvider").value.trim();
  if (!provider) { $("pfStatus").textContent = "pick a provider first"; return; }
  $("pfStatus").textContent = "testing…";
  try {
    const r = await invoke("test_provider", { provider, baseUrl: resolveBaseUrl() });
    $("pfStatus").textContent = "✓ " + r;
  } catch (e) { $("pfStatus").textContent = "✕ " + e + " (Save first if you changed the key)"; }
});

// Discover models — populate the Model field's list with the provider's real models.
$("pfDiscover")?.addEventListener("click", async () => {
  if (!invoke) return;
  const provider = $("pfProvider").value.trim();
  if (!provider) { $("pfStatus").textContent = "pick a provider first"; return; }
  $("pfStatus").textContent = "discovering models…";
  try {
    const models = await invoke("discover_models", { provider, baseUrl: resolveBaseUrl() });
    $("modelList").innerHTML = models.map((m) => `<option value="${escapeHtml(m)}"></option>`).join("");
    $("pfStatus").textContent = models.length
      ? `${models.length} models found — pick one in Model`
      : "no models returned";
  } catch (e) { $("pfStatus").textContent = "✕ " + e + " (Save the key first if needed)"; }
});

/* ---------- tools: the runtime's governed tool contracts ---------- */
// Effect class → the ceiling the governor enforces. This is the thymos-unique
// signal: every tool is tagged with the maximum effect it may ever produce.
const EFFECT_RANK = { pure: 0, read: 1, write: 2, external: 3, irreversible: 4 };
function effectClassName(e) {
  return ["pure", "read", "write", "external", "irreversible"][EFFECT_RANK[e] ?? 1];
}
async function loadTools() {
  const el = $("toolsList");
  el.innerHTML = "<div class='hint'>loading…</div>";
  try {
    const res = await getJSON("/tools");
    const tools = res.tools || (Array.isArray(res) ? res : []);
    el.innerHTML = "";
    if (!tools.length) { el.innerHTML = "<div class='hint'>no tools registered</div>"; return; }
    // Keep the grant chips + mode→scope mapping in sync with the registry.
    toolsCache = tools.map((t) => ({ name: t.name, effect_class: t.effect_class }));
    renderGrantChips(tools.map((t) => t.name).filter(Boolean));
    tools
      .sort((a, b) => (EFFECT_RANK[a.effect_class] ?? 0) - (EFFECT_RANK[b.effect_class] ?? 0))
      .forEach((t) => {
        const eff = effectClassName(t.effect_class);
        // Registered ≠ usable: show whether YOUR current grants authorize it,
        // so a writ rejection never reads as "the tool is broken".
        const granted = scopeAuthorizes(t.name || "");
        const div = document.createElement("div");
        div.className = "item tool-item";
        div.innerHTML =
          `<span class="glyph permit">▣</span>` +
          `<b>${escapeHtml(t.name || "")}</b>` +
          `<span class="effect eff-${eff}" title="effect ceiling enforced by the governor">${eff}</span>` +
          (t.risk_class ? `<span class="risk risk-${escapeHtml(t.risk_class)}">${escapeHtml(t.risk_class)} risk</span>` : "") +
          `<span class="badge ${granted ? "ok" : ""}" title="${granted ? "your current grant chips authorize this tool" : "not in your grant chips — runs will reject it until you grant it (Chat → left rail)"}">${granted ? "granted" : "not granted"}</span>` +
          `<span class="meta">v${escapeHtml(t.version || "1")}</span>`;
        el.appendChild(div);
      });
  } catch (e) { el.innerHTML = `<div class='hint'>could not load tools: ${e}. Is the runtime live?</div>`; }
}
$("refreshTools").addEventListener("click", loadTools);

/* ---------- add a custom tool (governed manifest) ---------- */
// Toggle shell vs http executor fields.
$("tlKind")?.addEventListener("change", () => {
  const http = $("tlKind").value === "http";
  $("tlCmdRow").hidden = http;
  $("tlUrlRow").hidden = !http;
});

$("toolForm")?.addEventListener("submit", async (e) => {
  e.preventDefault();
  if (!invoke) return;
  const status = $("tlStatus");
  const name = $("tlName").value.trim();
  const kind = $("tlKind").value;
  // Validation — fail clearly, never write an invalid manifest.
  if (!/^[a-z][a-z0-9_]*$/.test(name)) {
    status.textContent = "name must be lower_snake_case (letters, digits, _)"; return;
  }
  let schema;
  try { schema = JSON.parse($("tlSchema").value); }
  catch (_) { status.textContent = "input schema must be valid JSON"; return; }
  const exec = kind === "http"
    ? { kind: "http", url_template: $("tlUrl").value.trim(), method: "GET" }
    : { kind: "shell", command_template: $("tlCmd").value.trim() };
  const target = kind === "http" ? exec.url_template : exec.command_template;
  if (!target) { status.textContent = `${kind === "http" ? "URL" : "command"} template is required`; return; }

  const manifest = {
    name,
    version: $("tlVersion").value.trim() || "1.0.0",
    description: $("tlDesc").value.trim(),
    effect_class: $("tlEffect").value,
    risk_class: $("tlRisk").value,
    input_schema: schema,
    executor: exec,
  };
  status.textContent = "saving…";
  try {
    await invoke("save_tool", { manifest });
    // The runtime loads manifests at startup, so restart to register it.
    const wasRunning = await invoke("runtime_running").catch(() => false);
    if (wasRunning) await invoke("stop_runtime").catch(() => {});
    await invoke("start_runtime").catch(() => {});
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 400));
      await refreshStatus();
      if ($("dot").classList.contains("dot-on")) break;
    }
    await loadTools();
    status.textContent = `✓ added “${name}” — now governed by the runtime`;
    $("tlName").value = "";
  } catch (err) {
    status.textContent = "could not add tool: " + err;
  }
});

/* ---------- backups: the real on-disk ledger file ---------- */
$("refreshBackups")?.addEventListener("click", loadBackups);

async function loadBackups() {
  // Live snapshot of the runtime + ledger, as of now.
  const stateEl = $("backupState");
  if (stateEl) {
    let health = null, runs = 0;
    try { health = await getJSON("/health"); } catch (_) {}
    try {
      const r = await getJSON("/runs");
      const list = Array.isArray(r) ? r : r.runs || [];
      runs = (Array.isArray(r) ? r : r).total ?? list.length ?? 0;
    } catch (_) {}
    const now = new Date().toLocaleTimeString();
    if (health) {
      stateEl.innerHTML =
        `<div class="state-grid">` +
        `<span class="dim">runtime</span><span><span class="badge ok">live</span></span>` +
        `<span class="dim">provider</span><span>${escapeHtml(health.default_provider)} ` +
        `<span class="badge ${health.cognition_live ? "ok" : "bad"}">${health.cognition_live ? "real model" : "mock"}</span></span>` +
        `<span class="dim">ledger</span><span>${escapeHtml(health.ledger)}</span>` +
        `<span class="dim">runs recorded</span><span>${runs}</span>` +
        `<span class="dim">mode</span><span>${escapeHtml(health.mode)}</span>` +
        `</div><div class="state-asof">as of ${now}</div>`;
    } else {
      stateEl.innerHTML = `<div class="hint">Runtime not running — press <b>Start runtime</b> (top right).</div>`;
    }
  }
  const pathEl = $("ledgerPath");
  if (!pathEl) return;
  if (!invoke) { pathEl.textContent = "available in the desktop app"; return; }
  try {
    const p = await invoke("ledger_path");
    pathEl.textContent = p || "(runtime not started yet)";
    const copy = $("copyLedgerPath");
    if (copy) copy.onclick = async () => {
      try { await navigator.clipboard.writeText(p); copy.textContent = "Copied"; }
      catch (_) { copy.textContent = "—"; }
      setTimeout(() => (copy.textContent = "Copy path"), 1200);
    };
  } catch (e) { pathEl.textContent = "could not resolve ledger path: " + e; }
}

/* ---------- skills: author + tune authority-narrowing templates ---------- */
async function loadSkills() {
  const el = $("skillsList");
  const chipsEl = $("skillChips");
  try {
    const res = await getJSON("/skills");
    const skills = res.skills || [];
    if (!skills.length) {
      el.innerHTML = "<div class='hint'>no skills yet — create one below</div>";
    } else {
      el.innerHTML = "";
      skills.forEach((s) => {
        const div = document.createElement("div");
        div.className = "item";
        div.style.cursor = "pointer";
        div.title = "load into the form to tune";
        div.innerHTML =
          `<span class="glyph permit">✦</span><b>${escapeHtml(s.name)}</b>` +
          `<span class="meta">v${s.version} · ${escapeHtml(s.title || "")}</span>`;
        div.addEventListener("click", () => editSkill(s.name));
        el.appendChild(div);
      });
    }
    // Multi-select skill chips: any combination can be active at once. When
    // there are no skills, the rail stays clean (no placeholder).
    if (chipsEl) {
      if (!skills.length) {
        chipsEl.innerHTML = "";
      } else {
        chipsEl.innerHTML = "";
        skills.forEach((s) => {
          const chip = document.createElement("button");
          chip.type = "button";
          chip.className = "chip";
          chip.dataset.skill = s.name;
          chip.title = `${s.title || s.name} — narrows authority. Click to toggle.`;
          chip.textContent = `✦ ${s.name}`;
          chip.addEventListener("click", () => {
            chip.classList.toggle("on");
            try { localStorage.setItem(SKILLS_KEY, JSON.stringify(selectedSkills())); } catch (_) {}
          });
          chipsEl.appendChild(chip);
        });
      }
      restoreDefaults(); // re-activate the saved set of skills
    }
  } catch (e) {
    el.innerHTML = `<div class='hint'>could not load skills: ${e}</div>`;
  }
}
$("refreshSkills")?.addEventListener("click", loadSkills);

async function editSkill(name) {
  try {
    const res = await getJSON(`/skills/${encodeURIComponent(name)}`);
    const s = res.skill || {};
    $("skName").value = s.name || "";
    $("skTitle").value = s.title || "";
    $("skInstr").value = s.instructions || "";
    $("skTools").value = (s.tools || []).map((t) => t.tool).join(", ");
    $("skModel").value = (s.model_hint && s.model_hint.model) || "";
    const c = s.ceiling || {};
    $("skRead").checked = !!c.read;
    $("skWrite").checked = !!c.write;
    $("skExternal").checked = !!c.external;
    $("skIrrev").checked = !!c.irreversible;
    $("skStatus").textContent = `editing ${s.name} (v${s.version}) — saving bumps the version`;
  } catch (e) {
    $("skStatus").textContent = "load failed: " + e;
  }
}

$("skClear")?.addEventListener("click", () => {
  ["skName", "skTitle", "skInstr", "skTools"].forEach((id) => ($(id).value = ""));
  $("skRead").checked = true;
  $("skWrite").checked = true;
  $("skExternal").checked = false;
  $("skIrrev").checked = false;
  $("skStatus").textContent = "";
});

// One-click starter templates — prefill a working example so users don't start
// from a blank technical form. They can tweak then save.
const SKILL_TEMPLATES = {
  "repo-reader": { name: "repo-reader", title: "Read & summarize a repo",
    instructions: "Explore the repository read-only and summarize what you find. Do not modify anything.",
    tools: "fs.read, grep", read: true, write: false, external: false, irrev: false },
  "web-research": { name: "web-research", title: "Research on the web",
    instructions: "Research the question using HTTP and cite your sources.",
    tools: "http", read: true, write: false, external: true, irrev: false },
  "safe-editor": { name: "safe-editor", title: "Edit code safely",
    instructions: "Make minimal, well-explained edits. Always read a file before you change it.",
    tools: "fs.read, fs.patch, grep", read: true, write: true, external: false, irrev: false },
};
document.querySelectorAll("#skillForm .starter-row .chip").forEach((b) =>
  b.addEventListener("click", () => {
    const t = SKILL_TEMPLATES[b.dataset.skill];
    if (!t) return;
    $("skName").value = t.name; $("skTitle").value = t.title;
    $("skInstr").value = t.instructions; $("skTools").value = t.tools;
    $("skRead").checked = t.read; $("skWrite").checked = t.write;
    $("skExternal").checked = t.external; $("skIrrev").checked = t.irrev;
    $("skStatus").textContent = "template loaded — tweak and Save";
  }));

const TOOL_TEMPLATES = {
  shell: { name: "run_script", desc: "Run a shell command", effect: "write", risk: "medium",
    kind: "shell", cmd: "./script.sh {arg}", url: "",
    schema: '{ "type": "object", "properties": { "arg": { "type": "string" } } }' },
  http: { name: "call_api", desc: "Call an HTTP API", effect: "external", risk: "medium",
    kind: "http", cmd: "", url: "https://api.example.com/{path}",
    schema: '{ "type": "object", "properties": { "path": { "type": "string" } } }' },
};
document.querySelectorAll("#toolForm .starter-row .chip").forEach((b) =>
  b.addEventListener("click", () => {
    const t = TOOL_TEMPLATES[b.dataset.tool];
    if (!t) return;
    $("tlName").value = t.name; $("tlDesc").value = t.desc;
    $("tlEffect").value = t.effect; $("tlRisk").value = t.risk;
    $("tlKind").value = t.kind; $("tlKind").dispatchEvent(new Event("change"));
    $("tlCmd").value = t.cmd; $("tlUrl").value = t.url; $("tlSchema").value = t.schema;
    $("tlStatus").textContent = "template loaded — tweak and Add tool";
  }));

$("skillForm")?.addEventListener("submit", async (e) => {
  e.preventDefault();
  const name = $("skName").value.trim();
  if (!name) {
    $("skStatus").textContent = "name is required";
    return;
  }
  const tools = $("skTools")
    .value.split(",")
    .map((t) => t.trim())
    .filter(Boolean)
    .map((t) => ({ tool: t }));
  const def = {
    name,
    version: 1,
    title: $("skTitle").value.trim(),
    instructions: $("skInstr").value,
    tools,
    ceiling: {
      read: $("skRead").checked,
      write: $("skWrite").checked,
      external: $("skExternal").checked,
      irreversible: $("skIrrev").checked,
    },
    model_hint: { model: $("skModel").value.trim() || null },
  };
  $("skStatus").textContent = "saving…";
  try {
    const res = await postJSON("/skills", def);
    $("skStatus").textContent = `saved ${name} → v${res.skill?.version ?? ""}`;
    await loadSkills();
  } catch (err) {
    $("skStatus").textContent = "save failed: " + err;
  }
});

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
    // Ledger entries are fetched softly: a run that failed before recording a
    // trajectory (e.g. provider init error) legitimately has none and the
    // endpoint 404s — that must NOT blank the whole view. The narrative below
    // still tells the story.
    let list = [];
    let ledgerMissing = false;
    try {
      const entries = await getJSON(`/audit/entries?run_id=${encodeURIComponent(runId)}`);
      list = Array.isArray(entries) ? entries : entries.entries || [];
    } catch (_) { ledgerMissing = true; }
    trail.innerHTML = "";

    // --- What happened: the run's narrative, from the execution session log.
    // Provider retries, grant requests, rejections, executions, errors — with
    // the same plain-English hints the chat shows.
    try {
      const snap = await getJSON(`/runs/${encodeURIComponent(runId)}/execution`);
      const log = snap?.log || [];
      if (log.length) {
        const head = document.createElement("div");
        head.className = "audit-subhead";
        head.innerHTML = `<b>What happened</b><span class="meta"> · the run's live narrative (status: ${escapeHtml(snap.status || "?")})</span>`;
        trail.appendChild(head);
        log.forEach((e) => {
          const [g, cls] = glyphFor(e);
          const when = e.timestamp_ms ? new Date(e.timestamp_ms).toLocaleTimeString() : "";
          const div = document.createElement("div");
          div.className = "item audit-narrative";
          let detail = e.detail || "";
          if (detail.length > 220) detail = detail.slice(0, 220) + "…";
          div.innerHTML =
            `<span class="glyph ${cls}">${g}</span>` +
            `<b>${escapeHtml(e.title || "")}</b>` +
            (e.tool ? `<span class="meta">${escapeHtml(e.tool)}</span>` : "") +
            `<span class="meta">${escapeHtml(when)}</span>` +
            (detail ? `<div class="audit-detail">${escapeHtml(detail)}</div>` : "");
          const hint = governanceHint(e);
          if (hint) div.innerHTML += `<div class="audit-hint">↳ ${escapeHtml(hint)}</div>`;
          trail.appendChild(div);
        });
      }
    } catch (_) { /* restored runs may have no live session — ledger below */ }

    // --- Ledger chain: the authoritative, hash-chained record.
    const head2 = document.createElement("div");
    head2.className = "audit-subhead";
    head2.innerHTML = `<b>Ledger chain</b><span class="meta"> · authoritative, content-addressed, replay-verified</span>`;
    trail.appendChild(head2);
    if (!list.length) {
      const note = document.createElement("div");
      note.className = "hint";
      note.textContent = ledgerMissing
        ? "No ledger entries for this run — it ended before committing anything (e.g. a provider error or an early rejection). The narrative above is the full story."
        : "No ledger entries yet.";
      trail.appendChild(note);
    }
    list.forEach((e) => {
      const div = document.createElement("div");
      div.className = "item";
      div.innerHTML =
        `<span class="meta">#${e.seq}</span><b>${escapeHtml(e.kind)}</b>` +
        `<span class="meta">${escapeHtml((e.commit_id || e.id || "").slice(0, 12))}</span>`;
      trail.appendChild(div);
    });
    // Replay verifies integrity; only meaningful if there's a chain to fold.
    if (!ledgerMissing) {
      try {
        const rep = await getJSON(`/runs/${runId}/replay`);
        badge.innerHTML =
          `<span class="badge ok">replay ✓ verified</span> ` +
          `<span class="meta">${rep.commits_replayed} commits · ` +
          `${rep.rejected_proposals} rejected · head ${(rep.head_commit || "").slice(0, 12)}</span>`;
      } catch (_) {
        badge.innerHTML = `<span class="badge bad">replay could not verify</span>`;
      }
    }
  } catch (e) { trail.innerHTML = `<div class='hint'>could not load trail: ${e}</div>`; }
}
// Mind's inspector links here ("open in Audit").
window.thymosOpenAudit = openAudit;
$("auditForm").addEventListener("submit", (ev) => {
  ev.preventDefault();
  const q = $("auditSearch")?.value.trim();
  const id = $("auditRunId").value.trim();
  if (q) loadSearch(q);
  else if (id) loadAudit(id);
  else loadExplorer();
});
// Kind chips re-filter the all-activity view in place.
document.querySelectorAll("#auditKinds .chip").forEach((c) =>
  c.addEventListener("click", () => {
    c.classList.toggle("on");
    if (!$("auditRunId").value.trim() && !$("auditSearch")?.value.trim()) loadExplorer();
  }));

/* ---------- Ledger Explorer: all activity, grouped by day ---------- */
// The cross-run timeline (RFC: mind-cognitive-graph, stage 1). Every entry is
// a real ledger record; clicking one drills into that run's full trail.
async function loadExplorer() {
  const trail = $("auditTrail");
  const badge = $("replayBadge");
  badge.innerHTML = "";
  trail.innerHTML = "<div class='hint'>loading all activity…</div>";
  try {
    const [entriesRes, runsRes] = await Promise.all([
      getJSON("/audit/entries?limit=300"),
      getJSON("/runs?limit=200"),
    ]);
    const entries = entriesRes.entries || [];
    const runs = runsRes.runs || [];
    const byTraj = {};
    runs.forEach((r) => { if (r.trajectory_id) byTraj[r.trajectory_id] = r; });
    const kinds = new Set([...document.querySelectorAll("#auditKinds .chip.on")]
      .map((c) => c.dataset.kind));
    const shown = entries.filter((e) =>
      [...kinds].some((k) => (e.kind || "").toLowerCase().includes(k)));
    trail.innerHTML = "";
    if (!shown.length) {
      trail.innerHTML = "<div class='hint'>no ledger activity yet — chat with the agent and every governed action lands here</div>";
      return;
    }
    // Newest first, grouped by day.
    shown.sort((a, b) => (b.created_at || 0) - (a.created_at || 0));
    let lastDay = "";
    shown.forEach((e) => {
      const ts = (e.created_at || 0) > 1e12 ? e.created_at : e.created_at * 1000;
      const day = ts ? new Date(ts).toLocaleDateString() : "unknown date";
      if (day !== lastDay) {
        lastDay = day;
        const h = document.createElement("div");
        h.className = "audit-subhead";
        h.innerHTML = `<b>${escapeHtml(day)}</b>`;
        trail.appendChild(h);
      }
      const run = byTraj[e.trajectory_id];
      const div = document.createElement("div");
      div.className = "item";
      div.innerHTML =
        `<span class="meta">#${e.seq}</span><b>${escapeHtml(e.kind || "")}</b>` +
        (run ? `<span class="meta">${escapeHtml(String(run.task || "").slice(0, 60))}</span>` : "") +
        `<span class="meta">${ts ? new Date(ts).toLocaleTimeString() : ""}</span>`;
      if (run) {
        div.style.cursor = "pointer";
        div.title = "open this run's full trail";
        div.onclick = () => { $("auditRunId").value = run.run_id; loadAudit(run.run_id); };
      }
      trail.appendChild(div);
    });
    badge.innerHTML = `<span class="meta">${shown.length} entries · click any to open its run · clear filters with the chips above</span>`;
  } catch (e) { trail.innerHTML = `<div class='hint'>could not load activity: ${e}</div>`; }
}

/* ---------- global search (lexical, over everything recorded) ---------- */
async function loadSearch(q) {
  const trail = $("auditTrail");
  const badge = $("replayBadge");
  badge.innerHTML = "";
  trail.innerHTML = "<div class='hint'>searching…</div>";
  try {
    const res = await getJSON(`/search?q=${encodeURIComponent(q)}`);
    const hits = res.hits || [];
    trail.innerHTML = "";
    if (!hits.length) {
      trail.innerHTML = `<div class='hint'>no matches for “${escapeHtml(q)}”</div>`;
      return;
    }
    hits.forEach((h) => {
      const div = document.createElement("div");
      div.className = "item audit-narrative";
      div.innerHTML =
        `<span class="badge">${escapeHtml(h.field || "")}</span>` +
        `<b>${escapeHtml(String(h.task || h.title || "").slice(0, 60))}</b>` +
        (h.snippet ? `<div class="audit-detail">${escapeHtml(h.snippet)}</div>` : "");
      div.style.cursor = "pointer";
      div.title = "open this run's full trail";
      div.onclick = () => { $("auditRunId").value = h.run_id; $("auditSearch").value = ""; loadAudit(h.run_id); };
      trail.appendChild(div);
    });
    badge.innerHTML = `<span class="meta">${hits.length} matches for “${escapeHtml(q)}” · click any to open its run</span>`;
  } catch (e) { trail.innerHTML = `<div class='hint'>search failed: ${e}</div>`; }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

/* ---------- boot ---------- */
(async function boot() {
  await resolveBase();
  await refreshStatus();
  // Auto-start the runtime so the app just works on launch (idempotent).
  if (invoke && !$("dot").classList.contains("dot-on")) {
    try {
      await invoke("start_runtime");
      for (let i = 0; i < 25; i++) {
        await new Promise((r) => setTimeout(r, 400));
        await refreshStatus();
        if ($("dot").classList.contains("dot-on")) break;
      }
    } catch (_) {}
  }
  // Chat history + saved per-model defaults (skill + grants).
  loadChats();
  restoreDefaults();
  if (chats.length) openChat(chats[0].id); else newChatSession(false);
  loadSkills().catch(() => {}); // fills the skill chips + restores the active set
  // Grant chips + mode→scope mapping come from the live registry.
  getJSON("/tools")
    .then((r) => {
      const tools = r.tools || [];
      toolsCache = tools.map((t) => ({ name: t.name, effect_class: t.effect_class }));
      renderGrantChips(tools.map((t) => t.name).filter(Boolean));
    })
    .catch(() => {}); // offline: the static fallback chips stay
  renderModeUI();
  // Seed the model menu from the configured provider so chat/skill Model fields
  // suggest real models from the start (Discover replaces with the live list).
  if (invoke) {
    try { fillModelList((await invoke("get_provider_config")).provider); } catch (_) {}
  }
  setInterval(refreshStatus, 4000);
})();
