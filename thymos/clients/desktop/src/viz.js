// Mind — the runtime's neural map. A pure network: the conversation's run as
// lifecycle lanes (intent → proposal → grant/rejected → execution → commit →
// answer), with the provider, the tools actually used, and the replay verdict
// wired to the events they touched. No logo, no cage, no decoration — every
// node and edge is a real runtime record, read from the runtime's own
// endpoints (`/runs/{id}/execution`, `/audit/entries`, `/health`,
// `/runs/{id}/replay`); three.js is vendored (no egress).
import * as THREE from "./vendor/three.module.js";

const invoke = window.__TAURI__?.core?.invoke;
let BASE = "http://127.0.0.1:3001";
(async () => { try { if (invoke) BASE = await invoke("runtime_addr"); } catch (_) {} })();

const VIOLET = 0x7c5cff, CYAN = 0x45e0ff, GREEN = 0x46d39a, RED = 0xff6b8a,
      AMBER = 0xffc24b, BLUE = 0x6ab0ff, DIM = 0x8a7fe0, ORANGE = 0xff9d5c;

// Node taxonomy — the lifecycle types a run can produce, with the question
// each answers for the operator.
const TYPES = {
  intent:    { color: CYAN,   label: "Intent",     sum: "Cognition declared what it wants to do — no side effects yet." },
  proposal:  { color: VIOLET, label: "Proposal",   sum: "Compiler + policy checks resolved authority, budget, and risk." },
  grant:     { color: AMBER,  label: "Grant",      sum: "Suspended — waiting for (or resolved by) an operator approval." },
  rejected:  { color: ORANGE, label: "Rejected",   sum: "The runtime refused this proposal — authority or policy said no. Governed, not broken: grant the tool (or change policy) and retry." },
  message:   { color: 0x9aa6ff, label: "You",      sum: "A message you sent. Each one starts its own governed run." },
  answer:    { color: 0xd9bcff, label: "Answer",   sum: "The model's final reply for this run, recorded once the governed loop finished." },
  execution: { color: BLUE,   label: "Execution",  sum: "The runtime executed a governed tool contract." },
  commit:    { color: GREEN,  label: "Commit",     sum: "An authorized action mutated world state — appended to the ledger." },
  error:     { color: RED,    label: "Error",      sum: "Something failed here. The raw detail is preserved below." },
  system:    { color: DIM,    label: "System",     sum: "Runtime housekeeping for this run." },
  provider:  { color: CYAN,   label: "Provider",   sum: "The cognition provider answering this run. It proposes; it never executes." },
  tool:      { color: BLUE,   label: "Tool",       sum: "A governed capability this run actually invoked." },
  replay:    { color: GREEN,  label: "Replay",     sum: "The ledger chain re-verified end-to-end — this outcome is reproducible." },
};

// Map a live execution-session log entry to a node type.
function classifyLog(e) {
  if (e.level === "error") return "error";
  const p = e.phase || "";
  if (p === "intent") return "intent";
  if (p === "proposal") {
    // Both rejections and approval-requests arrive as proposal/warning —
    // distinguish them so a blocked path reads as Rejected, not as a grant.
    if (e.level !== "warning") return "proposal";
    return /reject/i.test(e.title || "") ? "rejected" : "grant";
  }
  if (p === "execution") return "execution";
  if (p === "result") return e.level === "success" ? "commit" : "system";
  return "system";
}

// Map a ledger/audit entry kind to a node type (fallback source for runs
// restored from disk whose live session log is gone).
function classifyAudit(en) {
  const k = (en.kind || "").toLowerCase();
  if (k.includes("commit")) return "commit";
  if (k.includes("reject")) return "rejected";
  if (k.includes("approval") || k.includes("suspend")) return "grant";
  if (k.includes("delegation")) return "proposal";
  if (k.includes("skill")) return "proposal";
  if (k.includes("root")) return "intent";
  return "system";
}

let renderer, scene, camera, world, nodesGroup, glowTex;
let inited = false, running = false, raf = 0, loadedOnce = false;
// Stable 2D navigation: the graph is planar (z=0), so we pan + zoom instead of
// orbiting. Nothing auto-moves — nodes stay put so they're easy to click.
let panX = 0, panY = 0, targetPanX = 0, targetPanY = 0;
let drag = null;
let nodeMeshes = [], raycaster, pointer, dragMoved = false;

// Animation + graph state.
let pulses = [], chainCurve = null, signal = null, clock = 0;
let currentRunId = "", refreshTimer = 0, lastCount = -1, lastFilterKey = "";
let spawnQueue = [];           // meshes animating in
let activeMesh = null;         // newest lifecycle node while the run is live
let runStatus = "";            // running | waiting_approval | completed | failed
let searchQ = "";
const filters = { message: true, intent: true, proposal: true, grant: true, rejected: true, execution: true, commit: true, answer: true, error: true, system: false };
// When the operator types a run id, Mind pins to it; otherwise it follows
// whatever the chat is currently doing.
let pinnedRun = false;

function radialTexture(inner) {
  const c = document.createElement("canvas");
  c.width = c.height = 128;
  const g = c.getContext("2d");
  const grad = g.createRadialGradient(64, 64, 0, 64, 64, 64);
  grad.addColorStop(0, inner);
  grad.addColorStop(1, "rgba(0,0,0,0)");
  g.fillStyle = grad;
  g.fillRect(0, 0, 128, 128);
  return new THREE.CanvasTexture(c);
}

// A small always-facing text label for context nodes.
function labelSprite(text, cssColor) {
  const c = document.createElement("canvas");
  c.width = 256; c.height = 56;
  const g = c.getContext("2d");
  g.font = "600 26px -apple-system, 'Segoe UI', sans-serif";
  g.textAlign = "center";
  g.fillStyle = cssColor;
  g.fillText(String(text).slice(0, 18), 128, 38);
  const tex = new THREE.CanvasTexture(c);
  const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, opacity: 0.9 }));
  s.scale.set(2.4, 0.53, 1);
  return s;
}
const hex = (n) => "#" + n.toString(16).padStart(6, "0");

function init() {
  const container = document.getElementById("mindCanvas");
  renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  container.appendChild(renderer.domElement);

  scene = new THREE.Scene();
  camera = new THREE.PerspectiveCamera(55, 1, 0.1, 500);
  camera.position.set(0, 0, 13);
  glowTex = radialTexture("rgba(124,92,255,0.9)");

  // Flat, front-facing graph. Pure network: no logo, no cage, no decoration —
  // only the run's real events and what connects them, held still.
  world = new THREE.Group();
  scene.add(world);

  nodesGroup = new THREE.Group();
  world.add(nodesGroup);

  // Interaction: drag to PAN, wheel to zoom, click a node to inspect it.
  // No rotation, no auto-motion — clicking is precise.
  raycaster = new THREE.Raycaster();
  pointer = new THREE.Vector2();
  const el = renderer.domElement;
  el.addEventListener("pointerdown", (e) => { drag = { x: e.clientX, y: e.clientY }; dragMoved = false; });
  window.addEventListener("pointerup", () => { drag = null; });
  window.addEventListener("pointermove", (e) => {
    if (!drag) return;
    if (Math.abs(e.clientX - drag.x) + Math.abs(e.clientY - drag.y) > 3) dragMoved = true;
    // Pan in the view plane; scale by zoom so it tracks the cursor 1:1.
    const k = camera.position.z / 520;
    targetPanX += (e.clientX - drag.x) * k;
    targetPanY -= (e.clientY - drag.y) * k;
    drag = { x: e.clientX, y: e.clientY };
  });
  el.addEventListener("wheel", (e) => {
    e.preventDefault();
    camera.position.z = Math.max(6, Math.min(30, camera.position.z + e.deltaY * 0.012));
  }, { passive: false });
  // Click (not drag) on a node → open the inspector + center it.
  el.addEventListener("click", (e) => {
    if (dragMoved) return;
    const hit = nodeAt(e);
    if (hit) { inspectNode(hit.userData); focusOn(hit); }
  });
  // Double-click empty space → reset the view to fit.
  el.addEventListener("dblclick", (e) => {
    if (nodeAt(e)) return;
    targetPanX = 0; targetPanY = 0; camera.position.z = 13;
  });
  // Hover → quick tooltip + pointer cursor.
  el.addEventListener("pointermove", (e) => {
    if (drag) return;
    const hit = nodeAt(e);
    el.style.cursor = hit ? "pointer" : "grab";
    if (hit) showTip(e, hit.userData); else hideTip();
  });
  el.addEventListener("pointerleave", hideTip);

  new ResizeObserver(resize).observe(container);
  inited = true;
}

function resize() {
  if (!renderer) return;
  const c = document.getElementById("mindCanvas");
  const w = c.clientWidth || 800, h = c.clientHeight || 500;
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}

function clearNodes() {
  while (nodesGroup.children.length) {
    const o = nodesGroup.children.pop();
    o.geometry?.dispose?.();
    o.material?.map?.dispose?.();
    o.material?.dispose?.();
  }
  nodeMeshes = [];
  pulses = [];
  chainCurve = null;
  signal = null;
  spawnQueue = [];
  activeMesh = null;
}

/* ---------- inspectable nodes ---------- */
function nodeAt(e) {
  if (!raycaster || !nodeMeshes.length) return null;
  const rect = renderer.domElement.getBoundingClientRect();
  pointer.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
  raycaster.setFromCamera(pointer, camera);
  const hits = raycaster.intersectObjects(nodeMeshes, false);
  return hits.length ? hits[0].object : null;
}

// Smoothly pan the clicked node to the center of the view.
function focusOn(mesh) {
  targetPanX = -mesh.position.x;
  targetPanY = -mesh.position.y;
}

function inspectNode(nd) {
  const box = document.getElementById("mindInspector");
  if (!box) return;
  const t = TYPES[nd.type] || TYPES.system;
  const when = nd.time ? new Date(nd.time).toLocaleTimeString() : "";
  const detail = nd.detail
    ? (typeof nd.detail === "string" ? nd.detail : JSON.stringify(nd.detail, null, 2))
    : "";
  const rows = [
    when ? `<dt>time</dt><dd>${escHtml(when)}</dd>` : "",
    nd.step != null ? `<dt>step</dt><dd>${nd.step + 1}</dd>` : "",
    nd.tool ? `<dt>tool</dt><dd class="mono">${escHtml(nd.tool)}</dd>` : "",
    nd.run ? `<dt>run</dt><dd class="mono">${escHtml(String(nd.run).slice(0, 12))}…</dd>` : "",
    nd.id ? `<dt>id</dt><dd class="mono">${escHtml(String(nd.id).slice(0, 24))}…</dd>` : "",
    nd.seq != null ? `<dt>seq</dt><dd>${nd.seq}</dd>` : "",
  ];
  box.hidden = false;
  box.innerHTML =
    `<div class="mi-head"><span class="mi-kind" style="color:${hex(t.color)}">${escHtml(t.label)}</span>` +
    `<button class="mi-close" title="close">×</button></div>` +
    (nd.title ? `<div class="mi-title">${escHtml(nd.title)}</div>` : "") +
    `<div class="mi-sum">${escHtml(nd.sum || t.sum)}</div>` +
    `<dl class="mi-fields">${rows.join("")}</dl>` +
    (detail ? `<pre class="mi-detail">${escHtml(detail).slice(0, 1200)}</pre>` : "") +
    (nd.run ? `<button class="mi-audit" type="button">Open in Audit →</button>` : "");
  box.querySelector(".mi-close").onclick = () => { box.hidden = true; };
  // Cross-link to the Audit narrative for this run (main.js exposes the hook).
  const a = box.querySelector(".mi-audit");
  if (a) a.onclick = () => { window.thymosOpenAudit?.(nd.run); };
}

let tipEl = null;
function showTip(e, nd) {
  if (!tipEl) tipEl = document.getElementById("mindTip");
  if (!tipEl) return;
  const t = TYPES[nd.type] || TYPES.system;
  tipEl.hidden = false;
  tipEl.textContent = `${t.label}${nd.title ? " · " + nd.title : ""} — click to inspect`;
  const rect = renderer.domElement.getBoundingClientRect();
  tipEl.style.left = (e.clientX - rect.left + 12) + "px";
  tipEl.style.top = (e.clientY - rect.top + 12) + "px";
}
function hideTip() { if (tipEl) tipEl.hidden = true; }
function escHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

/* ---------- graph construction (all real data) ---------- */
const MAX_TIMELINE = 140; // newest entries win; keeps huge sessions smooth

function makeNodeMesh(nd, p, size) {
  const mat = new THREE.MeshBasicMaterial({ color: nd.color, transparent: true, opacity: 1 });
  const node = new THREE.Mesh(new THREE.SphereGeometry(size, 16, 16), mat);
  node.position.copy(p);
  node.userData = nd;
  nodesGroup.add(node);
  nodeMeshes.push(node);
  const halo = new THREE.Sprite(new THREE.SpriteMaterial(
    { map: glowTex, color: nd.color, transparent: true, blending: THREE.AdditiveBlending, opacity: 0.8 }));
  halo.scale.set(1.1, 1.1, 1);
  halo.position.copy(p);
  nodesGroup.add(halo);
  node.userData._halo = halo;
  pulses.push({ halo, phase: nodeMeshes.length * 0.6, err: nd.type === "error" });
  return node;
}

function line(a, b, color, opacity) {
  nodesGroup.add(new THREE.Line(
    new THREE.BufferGeometry().setFromPoints([a, b]),
    new THREE.LineBasicMaterial({ color, transparent: true, opacity })));
}

// Build the whole graph: lifecycle helix + context ring + edges.
// `timeline` = unified node records, `ctx` = { provider, live, tools, replayOk }.
function buildGraph(timeline, ctx, newFromIdx) {
  clearNodes();

  // ---- lifecycle lanes: one row per step, events flowing left → right ----
  // This is the runtime map: Intent → Proposal → (Grant/Rejected) →
  // Execution → Commit reads as a horizontal path; steps stack downward.
  const visible = timeline.filter((nd) => filters[nd.type] !== false);
  // Assign each event to its step's lane (system/preamble entries stick to
  // the lane in progress).
  let lane = 0;
  const lanes = [];
  visible.forEach((nd) => {
    if (nd.step != null) lane = nd.step;
    nd._lane = lane;
    (lanes[lane] = lanes[lane] || []).push(nd);
  });
  const laneIds = lanes.map((l, i) => l ? i : -1).filter((i) => i >= 0);
  const laneCount = laneIds.length || 1;
  const rowGap = Math.min(1.9, 11 / laneCount);
  const pts = [];
  const laneStart = new Map(); // lane → first node position (the message spine)
  visible.forEach((nd) => {
    const row = laneIds.indexOf(nd._lane);
    const mates = lanes[nd._lane];
    const col = mates.indexOf(nd);
    const x = (col - (mates.length - 1) / 2) * 1.55;
    const y = ((laneCount - 1) / 2 - row) * rowGap;
    const p = new THREE.Vector3(x, y, 0);
    pts.push(p);
    nd._p = p;
    if (col === 0) laneStart.set(nd._lane, p);
    const big = nd.type === "commit" || nd.type === "error" || nd.type === "rejected" || nd.type === "message";
    const mesh = makeNodeMesh(nd, p, big ? 0.2 : 0.15);
    if (nd.idx != null && newFromIdx != null && nd.idx >= newFromIdx) {
      mesh.scale.set(0.01, 0.01, 0.01);
      spawnQueue.push({ mesh, t0: clock });
    }
    activeMesh = mesh; // ends on the newest visible node
  });

  // Edges within each lane: the governed path of one message, left → right.
  // No traveling "signal" sprite — static synapse lines only (calmer, clearer).
  laneIds.forEach((lid) => {
    const lane = lanes[lid];
    for (let i = 1; i < lane.length; i++) {
      if (lane[i - 1]._p && lane[i]._p) line(lane[i - 1]._p, lane[i]._p, 0x8a7fe0, 0.45);
    }
  });
  // The conversation spine: each message connects to the next, top → bottom.
  for (let i = 1; i < laneIds.length; i++) {
    const a = laneStart.get(laneIds[i - 1]);
    const b = laneStart.get(laneIds[i]);
    if (a && b) line(a, b, 0x6a5fd0, 0.3);
  }
  chainCurve = null;
  signal = null;

  // ---- context ring: provider, tools actually used, replay verdict ----
  const ctxNodes = [];
  if (ctx.provider) {
    ctxNodes.push({
      type: "provider", title: ctx.provider, run: currentRunId,
      sum: `${TYPES.provider.sum} ${ctx.live ? "Answering with a real model." : "Mock — deterministic, offline."}`,
      detail: `provider: ${ctx.provider}\nmode: ${ctx.live ? "live model" : "mock"}`,
    });
  }
  for (const tool of ctx.tools) {
    ctxNodes.push({
      type: "tool", title: tool, tool, run: currentRunId,
      detail: `tool: ${tool}\nEvery invocation was checked against the run's writ before executing.`,
    });
  }
  if (ctx.replayOk) {
    ctxNodes.push({
      type: "replay", title: "replay verified", run: currentRunId,
      detail: "verify: thymos replay " + currentRunId,
    });
  }

  // Context sits on a ring around the conversation's network, each node wired
  // to the lifecycle events it actually touched — the ledger connected to the
  // convo, not to a decoration.
  const RC = 6.4;
  ctxNodes.forEach((nd, i) => {
    const a = (i / Math.max(ctxNodes.length, 1)) * Math.PI * 2 + 0.5;
    nd.color = (TYPES[nd.type] || TYPES.system).color;
    const p = new THREE.Vector3(Math.cos(a) * RC, ((i % 3) - 1) * 1.4, Math.sin(a) * RC * 0.45);
    makeNodeMesh(nd, p, 0.26);
    const lbl = labelSprite(nd.title, hex(nd.color));
    lbl.position.set(p.x, p.y - 0.65, p.z);
    nodesGroup.add(lbl);
    if (nd.type === "tool") {
      // Tools connect to every event that invoked them.
      let edges = 0;
      visible.forEach((tnd, j) => {
        if (tnd.tool === nd.tool && edges < 14) {
          line(p, pts[j], nd.color, 0.16);
          edges++;
        }
      });
    } else if (nd.type === "provider" && pts.length) {
      // The provider feeds cognition: wire it to each step's intent event.
      let edges = 0;
      visible.forEach((tnd, j) => {
        if (tnd.type === "intent" && edges < 10) {
          line(p, pts[j], nd.color, 0.12);
          edges++;
        }
      });
      if (!edges) line(p, pts[0], nd.color, 0.14);
    } else if (nd.type === "replay" && pts.length) {
      // Replay verifies the recorded chain: wire it to the final event.
      line(p, pts[pts.length - 1], nd.color, 0.18);
    }
  });

  applySearchDim();
}

// Search never rebuilds geometry — it just dims non-matching nodes in place.
function applySearchDim() {
  const q = searchQ.trim().toLowerCase();
  for (const m of nodeMeshes) {
    const nd = m.userData;
    const hay = `${nd.type} ${nd.title || ""} ${nd.tool || ""} ${nd.detail || ""}`.toLowerCase();
    const hit = !q || hay.includes(q);
    m.material.opacity = hit ? 1 : 0.12;
    if (nd._halo) nd._halo.material.opacity = hit ? 0.8 : 0.05;
  }
}

/* ---------- data loading (the runtime's own endpoints) ---------- */
// How many recent messages get their full per-run lifecycle fetched. Older
// messages still appear (message + answer nodes) but aren't re-fetched, so a
// long conversation stays light.
const SESSION_DETAIL_RUNS = 8;

// Map one run's execution log → lifecycle nodes for conversation turn `turn`.
function runNodes(snap, runId, turn) {
  const out = [];
  let base = turn * 1000;
  if (snap?.log?.length) {
    for (const e of snap.log) {
      const ty = classifyLog(e);
      if (ty === "system") continue; // housekeeping — keep the map clean
      out.push({
        idx: base++, type: ty, title: e.title, detail: e.detail,
        time: e.timestamp_ms, step: turn, tool: e.tool || null, run: runId, color: 0,
      });
    }
  }
  return out;
}

async function loadRun(idRaw) {
  const pinned = (idRaw || "").trim();
  try {
    // Pinned run id → single-run inspection. Otherwise render the whole
    // conversation: every message and the governed actions each one produced.
    if (pinned) return await renderSingleRun(pinned);
    const session = window.thymosSession?.();
    if (session && session.messages.length) return await renderSession(session);
    // No active chat yet → newest run as a fallback so the tab isn't blank.
    const runs = await (await fetch(`${BASE}/runs`)).json();
    const list = Array.isArray(runs) ? runs : runs.runs || [];
    const id = list[0]?.run_id;
    if (id) return await renderSingleRun(id);
  } catch (_) { /* runtime not up yet — the scene stays empty */ }
}

// The full conversation as one network: each turn is a lane —
// You → intent → proposal → (grant/rejected) → execution → commit → Answer.
async function renderSession(session) {
  const msgs = session.messages;
  // Pair user messages with the following agent reply (its run).
  const turns = [];
  for (let i = 0; i < msgs.length; i++) {
    if (msgs[i].role !== "user") continue;
    const reply = msgs.slice(i + 1).find((m) => m.role === "agent");
    turns.push({ user: msgs[i], reply: reply ? { ...reply } : null });
  }
  if (!turns.length) return;

  // Live follow: if a run is in flight (its reply isn't in the chat yet), the
  // newest unreplied turn IS that run — attach it so the graph streams the
  // agent's actions as they happen, not just finished turns.
  const liveRun = window.thymosActiveRun;
  if (liveRun) {
    const open = [...turns].reverse().find((t) => !t.reply);
    if (open) open.reply = { run_id: liveRun, status: "running", text: "" };
  }

  // Fetch lifecycle only for the most recent turns that have a run.
  const detailIdx = new Set();
  let budget = SESSION_DETAIL_RUNS;
  for (let i = turns.length - 1; i >= 0 && budget > 0; i--) {
    if (turns[i].reply?.run_id) { detailIdx.add(i); budget--; }
  }
  const snaps = {};
  await Promise.all([...detailIdx].map(async (i) => {
    const rid = turns[i].reply.run_id;
    try { snaps[i] = await (await fetch(`${BASE}/runs/${rid}/execution`)).json(); } catch (_) {}
  }));

  let timeline = [];
  let liveStatus = "";
  turns.forEach((t, i) => {
    timeline.push({
      idx: i * 1000 - 1, type: "message", title: "You",
      detail: t.user.text, step: i, run: t.reply?.run_id || null, color: 0,
    });
    const snap = snaps[i];
    if (snap) {
      timeline = timeline.concat(runNodes(snap, t.reply.run_id, i));
      if (i === turns.length - 1) liveStatus = snap.status || "";
    }
    const st = t.reply?.status || snap?.status || "";
    const terminal = ["completed", "failed", "cancelled"].includes(st);
    // Only finished turns get an Answer/Error node — a live turn ends on its
    // newest action, breathing, until it resolves.
    if (t.reply && terminal) {
      timeline.push({
        idx: i * 1000 + 900,
        type: st === "failed" ? "error" : "answer",
        title: st === "failed" ? "Run failed" : "Answer",
        detail: t.reply.text || snap?.final_answer || "",
        step: i, run: t.reply?.run_id || null, color: 0,
      });
    }
  });
  timeline = timeline.slice(-MAX_TIMELINE);
  timeline.forEach((nd) => { nd.color = (TYPES[nd.type] || TYPES.system).color; });
  runStatus = liveStatus;

  let health = null;
  try { health = await (await fetch(`${BASE}/health`)).json(); } catch (_) {}
  const tools = [...new Set(timeline.map((nd) => nd.tool).filter(Boolean))].slice(0, 8);
  const ctx = { provider: health?.default_provider || "", live: !!health?.cognition_live, tools, replayOk: false };

  // Live-aware signature: includes status so the final Answer node appears the
  // moment the run resolves, and node count so streamed actions re-render.
  const sig = `${session.id}:${timeline.length}:${liveStatus}:${JSON.stringify(filters)}`;
  if (sig !== lastFilterKey) {
    const grew = sig.split(":")[0] === lastFilterKey.split(":")[0];
    lastFilterKey = sig;
    buildGraph(timeline, ctx, grew ? -1 : null); // new turns/actions animate in
  }
  const el = document.getElementById("mindRunId");
  if (el && !el.value) el.placeholder = `${turns.length} messages · ${timeline.length} nodes`;
  renderSessionState(session, turns, health);
  // The always-on activity panel: the latest turn, read top-to-bottom.
  renderActivity(timeline, liveStatus);
}

// Stream the current turn's events into the side panel — full titles + detail,
// newest turn, auto-scrolled — so the agent's work reads without clicking.
function renderActivity(timeline, status) {
  const list = document.getElementById("mindActivityList");
  if (!list) return;
  const lastStep = timeline.reduce((m, n) => Math.max(m, n.step ?? 0), 0);
  const turn = timeline.filter((n) => (n.step ?? 0) === lastStep);
  list.innerHTML = "";
  turn.forEach((nd) => {
    const t = TYPES[nd.type] || TYPES.system;
    const row = document.createElement("div");
    row.className = "ma-row";
    const d = (nd.detail || "").slice(0, 160);
    row.innerHTML =
      `<span class="ma-dot" style="background:${hex(t.color)}"></span>` +
      `<div class="ma-body"><div class="ma-title">${escHtml(t.label)}` +
      (nd.tool ? ` · <span class="ma-tool">${escHtml(nd.tool)}</span>` : "") +
      `</div>` + (d ? `<div class="ma-detail">${escHtml(d)}</div>` : "") + `</div>`;
    row.onclick = () => inspectNode(nd);
    list.appendChild(row);
  });
  if (status === "running" || status === "waiting_approval") {
    const w = document.createElement("div");
    w.className = "ma-row ma-working";
    w.innerHTML = `<span class="ma-dot ma-pulse"></span><div class="ma-body"><div class="ma-title">working…</div></div>`;
    list.appendChild(w);
  }
  list.scrollTop = list.scrollHeight;
}

let prevMaxIdx = -1;
async function renderSingleRun(id) {
  const changedRun = id !== currentRunId;
  if (changedRun) prevMaxIdx = -1;
  currentRunId = id;
  let snap = null;
  try { snap = await (await fetch(`${BASE}/runs/${id}/execution`)).json(); } catch (_) {}
  let timeline = [];
  if (snap?.log?.length > 1) {
    timeline = snap.log.map((e) => ({
      idx: e.idx, type: classifyLog(e), title: e.title, detail: e.detail,
      time: e.timestamp_ms, step: e.step_index, tool: e.tool || null, run: id, color: 0,
    }));
  } else {
    const data = await (await fetch(`${BASE}/audit/entries?run_id=${encodeURIComponent(id)}`)).json();
    const entries = Array.isArray(data) ? data : data.entries || [];
    timeline = entries.map((en, i) => ({
      idx: i, type: classifyAudit(en), title: en.kind, detail: en.detail,
      seq: en.seq, id: en.commit_id || en.id, run: id, color: 0,
    }));
  }
  runStatus = snap?.status || "";
  if (snap?.final_answer && ["completed", "failed", "cancelled"].includes(runStatus)) {
    const lastIdx = timeline.length ? (timeline[timeline.length - 1].idx ?? 0) : 0;
    const lastStep = [...timeline].reverse().find((n) => n.step != null)?.step ?? null;
    timeline.push({
      idx: lastIdx + 1, type: runStatus === "completed" ? "answer" : "error",
      title: runStatus === "completed" ? "Answer" : "Run " + runStatus,
      detail: snap.final_answer, time: snap.updated_at_ms, step: lastStep, run: id, color: 0,
    });
  }
  timeline = timeline.slice(-MAX_TIMELINE);
  timeline.forEach((nd) => { nd.color = (TYPES[nd.type] || TYPES.system).color; });
  let health = null, replay = null;
  try { health = await (await fetch(`${BASE}/health`)).json(); } catch (_) {}
  try { const r = await fetch(`${BASE}/runs/${id}/replay`); if (r.ok) replay = await r.json(); } catch (_) {}
  const tools = [...new Set(timeline.map((nd) => nd.tool).filter(Boolean))].slice(0, 8);
  const ctx = { provider: health?.default_provider || "", live: !!health?.cognition_live, tools, replayOk: !!replay };
  const filterKey = JSON.stringify(filters) + ":" + id;
  const maxIdx = timeline.length ? timeline[timeline.length - 1].idx : -1;
  if (changedRun || timeline.length !== lastCount || filterKey !== lastFilterKey) {
    lastCount = timeline.length;
    lastFilterKey = filterKey;
    buildGraph(timeline, ctx, changedRun ? null : prevMaxIdx + 1);
    prevMaxIdx = maxIdx;
  }
  renderRunState(id, snap, health, replay);
}

// Session-level state strip: how the conversation is doing as a whole.
function renderSessionState(session, turns, health) {
  const box = document.getElementById("mindState");
  if (!box) return;
  const last = turns[turns.length - 1];
  const pieces = [
    `<span><span class="ms-label">conversation</span><span class="ms-val">${escHtml(session.title || "untitled")}</span></span>`,
    `<span><span class="ms-label">messages</span><span class="ms-val">${turns.length}</span></span>`,
    health
      ? `<span><span class="ms-label">provider</span><span class="ms-val">${escHtml(health.default_provider || "?")}</span>` +
        ` <span class="badge ${health.cognition_live ? "ok" : "bad"}">${health.cognition_live ? "live" : "mock"}</span></span>`
      : "",
    last?.reply ? `<span><span class="ms-label">last</span><span class="badge ${last.reply.status === "failed" ? "bad" : "ok"}">${escHtml(last.reply.status || "completed")}</span></span>` : "",
  ];
  box.innerHTML = pieces.filter(Boolean).join("");
  box.hidden = false;
}

// The live strip above the canvas: the run's real status, the runtime's
// current operator state, provider/model, governed counters, and the replay
// verdict — every field read from the runtime, none decorative.
function renderRunState(id, snap, health, replay) {
  const box = document.getElementById("mindState");
  if (!box) return;
  if (!snap || !snap.status) { box.hidden = true; return; }
  const st = snap.status || "?";
  const stCls = st === "completed" ? "ok" : (st === "failed" ? "bad" : "warn");
  const c = snap.counters || {};
  const pieces = [
    `<span><span class="ms-label">run</span><code>${escHtml(String(id).slice(0, 8))}</code></span>`,
    `<span><span class="ms-label">status</span><span class="badge ${stCls}">${escHtml(st)}</span></span>`,
    snap.operator_state
      ? `<span><span class="ms-label">now</span><span class="ms-val">${escHtml(snap.operator_state)}</span></span>`
      : "",
    health
      ? `<span><span class="ms-label">provider</span><span class="ms-val">${escHtml(health.default_provider || "?")}</span>` +
        ` <span class="badge ${health.cognition_live ? "ok" : "bad"}">${health.cognition_live ? "live" : "mock"}</span></span>`
      : "",
    `<span><span class="ms-label">commits</span><span class="ms-val">${c.commits ?? 0}</span></span>`,
    `<span><span class="ms-label">rejections</span><span class="ms-val">${c.rejections ?? 0}</span></span>`,
    (c.approvals_pending ?? 0) > 0
      ? `<span class="badge warn">⏸ ${c.approvals_pending} awaiting approval</span>`
      : "",
    replay
      ? `<span><span class="ms-label">replay</span><span class="badge ok">verified · ${replay.commits_replayed ?? 0} commits</span></span>`
      : "",
  ];
  box.innerHTML = pieces.filter(Boolean).join("");
  box.hidden = false;
}

/* ---------- animation ---------- */
function frame() {
  raf = requestAnimationFrame(frame);
  clock += 0.016;
  // Eased pan toward target — the only world motion, and only when you drag.
  panX += (targetPanX - panX) * 0.18;
  panY += (targetPanY - panY) * 0.18;
  world.position.set(panX, panY, 0);

  // Gentle, uniform halo breathing — a calm "alive" shimmer, not flashing.
  // (Errors get a slightly stronger pulse so they read as needing attention.)
  for (const p of pulses) {
    const amp = p.err ? 0.35 : 0.16;
    const s = 1.05 + amp * (0.5 + 0.5 * Math.sin(clock * 1.6 + p.phase));
    p.halo.scale.set(s, s, 1);
  }
  // New nodes scale in — work appearing as it happens.
  spawnQueue = spawnQueue.filter((sp) => {
    const k = Math.min((clock - sp.t0) / 0.45, 1);
    const e = 1 - Math.pow(1 - k, 3); // ease-out
    sp.mesh.scale.set(e, e, e);
    return k < 1;
  });
  // The newest node breathes a little while the run is live — "where I am".
  if (activeMesh && (runStatus === "running" || runStatus === "waiting_approval")) {
    const s = 1 + 0.25 * (0.5 + 0.5 * Math.sin(clock * 3));
    activeMesh.scale.set(s, s, s);
  }
  renderer.render(scene, camera);
}
function start() {
  if (!running) { running = true; frame(); }
  // Always-on: while visible, follow the chat's current run (switching when a
  // new message starts a new run), unless the operator pinned a run id.
  if (!refreshTimer) {
    // Pinned → keep refreshing that run; otherwise re-render the whole
    // conversation (picks up new messages + streaming actions live).
    refreshTimer = setInterval(() => {
      loadRun(pinnedRun ? (document.getElementById("mindRunId")?.value || "") : "");
    }, 1500);
  }
}
function stop() {
  running = false;
  cancelAnimationFrame(raf);
  if (refreshTimer) { clearInterval(refreshTimer); refreshTimer = 0; }
}

/* ---------- wiring: tab, controls, filters, search ---------- */
window.addEventListener("DOMContentLoaded", () => {
  const mindTab = document.querySelector('.tab[data-tab="mind"]');
  if (!mindTab) return;
  mindTab.addEventListener("click", () => {
    if (!inited) init();
    requestAnimationFrame(() => { resize(); start(); });
    if (!loadedOnce) { loadedOnce = true; loadRun(""); }
  });
  document.querySelectorAll('.tab:not([data-tab="mind"])').forEach((t) =>
    t.addEventListener("click", stop));
  // No "Visualize" button — Mind is always on. Typing a run id pins to it;
  // clearing the field resumes following the current chat.
  const runInput = document.getElementById("mindRunId");
  runInput?.addEventListener("change", () => {
    const v = runInput.value.trim();
    pinnedRun = !!v;
    loadRun(v);
  });

  // Legend doubles as a filter bar — click a type to hide/show it.
  document.querySelectorAll(".mind-legend .lg[data-type]").forEach((b) => {
    b.addEventListener("click", () => {
      const ty = b.dataset.type;
      filters[ty] = !filters[ty];
      b.classList.toggle("off", !filters[ty]);
      if (currentRunId) loadRun(currentRunId);
    });
  });
  // Search dims non-matching nodes in place (no geometry rebuild).
  document.getElementById("mindSearch")?.addEventListener("input", (e) => {
    searchQ = e.target.value || "";
    applySearchDim();
  });
});
