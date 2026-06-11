// Mind — an immersive 3D view of a run's active cognition: the OpenThymos
// logo suspended in a rotating wireframe cage, orbited by the run's real
// lifecycle — intents, proposals, grants, executions, commits, errors — plus a
// context ring of what the run actually used (provider, tools, replay
// verdict). Pure client: reads the runtime's own endpoints (`/runs/{id}/
// execution`, `/audit/entries`, `/health`, `/runs/{id}/replay`); three.js is
// vendored (no egress). Nothing here is decorative: every node carries the
// runtime record it represents, and clicking it shows that record.
import * as THREE from "./vendor/three.module.js";

const invoke = window.__TAURI__?.core?.invoke;
let BASE = "http://127.0.0.1:3001";
(async () => { try { if (invoke) BASE = await invoke("runtime_addr"); } catch (_) {} })();

const VIOLET = 0x7c5cff, CYAN = 0x45e0ff, GREEN = 0x46d39a, RED = 0xff6b8a,
      AMBER = 0xffc24b, BLUE = 0x6ab0ff, DIM = 0x8a7fe0;

// Node taxonomy — the lifecycle types a run can produce, with the question
// each answers for the operator.
const TYPES = {
  intent:    { color: CYAN,   label: "Intent",     sum: "Cognition declared what it wants to do — no side effects yet." },
  proposal:  { color: VIOLET, label: "Proposal",   sum: "Compiler + policy checks resolved authority, budget, and risk." },
  grant:     { color: AMBER,  label: "Grant",      sum: "Suspended — waiting for (or resolved by) an operator approval." },
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
  if (p === "proposal") return e.level === "warning" ? "grant" : "proposal";
  if (p === "execution") return "execution";
  if (p === "result") return e.level === "success" ? "commit" : "system";
  return "system";
}

// Map a ledger/audit entry kind to a node type (fallback source for runs
// restored from disk whose live session log is gone).
function classifyAudit(en) {
  const k = (en.kind || "").toLowerCase();
  if (k.includes("commit")) return "commit";
  if (k.includes("reject")) return "error";
  if (k.includes("approval") || k.includes("suspend")) return "grant";
  if (k.includes("delegation")) return "proposal";
  if (k.includes("skill")) return "proposal";
  if (k.includes("root")) return "intent";
  return "system";
}

let renderer, scene, camera, world, cage, nodesGroup, glowTex;
let inited = false, running = false, raf = 0, loadedOnce = false;
let targetRotY = 0, targetRotX = 0.25, curRotY = 0, curRotX = 0.25;
let drag = null;
let nodeMeshes = [], raycaster, pointer, dragMoved = false;

// Animation + graph state.
let pulses = [], chainCurve = null, signal = null, clock = 0;
let currentRunId = "", refreshTimer = 0, lastCount = -1, lastFilterKey = "";
let spawnQueue = [];           // meshes animating in
let activeMesh = null;         // newest lifecycle node while the run is live
let runStatus = "";            // running | waiting_approval | completed | failed
let searchQ = "";
const filters = { intent: true, proposal: true, grant: true, execution: true, commit: true, error: true, system: false };

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

  // Starfield.
  const N = 1100, pos = new Float32Array(N * 3);
  for (let i = 0; i < N * 3; i++) pos[i] = (Math.random() - 0.5) * 130;
  const starGeo = new THREE.BufferGeometry();
  starGeo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  scene.add(new THREE.Points(starGeo,
    new THREE.PointsMaterial({ color: 0x6a5fd0, size: 0.13, transparent: true, opacity: 0.75 })));

  // World group (orbited by drag + slow auto-rotation): cage + nodes.
  world = new THREE.Group();
  scene.add(world);

  cage = new THREE.Group();
  world.add(cage);
  cage.add(new THREE.LineSegments(
    new THREE.WireframeGeometry(new THREE.IcosahedronGeometry(3.0, 0)),
    new THREE.LineBasicMaterial({ color: VIOLET, transparent: true, opacity: 0.6 })));
  cage.add(new THREE.LineSegments(
    new THREE.WireframeGeometry(new THREE.DodecahedronGeometry(4.2, 0)),
    new THREE.LineBasicMaterial({ color: CYAN, transparent: true, opacity: 0.28 })));

  nodesGroup = new THREE.Group();
  world.add(nodesGroup);

  // Logo suspended in the cage — a sprite so it always faces the camera.
  const tex = new THREE.TextureLoader().load("logo.png");
  if ("colorSpace" in tex) tex.colorSpace = THREE.SRGBColorSpace;
  const logo = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true }));
  logo.scale.set(2.6, 2.6, 1);
  scene.add(logo);
  const glow = new THREE.Sprite(new THREE.SpriteMaterial(
    { map: glowTex, color: VIOLET, transparent: true, blending: THREE.AdditiveBlending, opacity: 0.85 }));
  glow.scale.set(8, 8, 1);
  scene.add(glow);

  // Interaction: drag to orbit, wheel to zoom, click a node to inspect it.
  raycaster = new THREE.Raycaster();
  pointer = new THREE.Vector2();
  const el = renderer.domElement;
  el.addEventListener("pointerdown", (e) => { drag = { x: e.clientX, y: e.clientY }; dragMoved = false; });
  window.addEventListener("pointerup", () => { drag = null; });
  window.addEventListener("pointermove", (e) => {
    if (!drag) return;
    if (Math.abs(e.clientX - drag.x) + Math.abs(e.clientY - drag.y) > 3) dragMoved = true;
    targetRotY += (e.clientX - drag.x) * 0.006;
    targetRotX += (e.clientY - drag.y) * 0.006;
    targetRotX = Math.max(-1.2, Math.min(1.2, targetRotX));
    drag = { x: e.clientX, y: e.clientY };
  });
  el.addEventListener("wheel", (e) => {
    e.preventDefault();
    camera.position.z = Math.max(7, Math.min(26, camera.position.z + e.deltaY * 0.01));
  }, { passive: false });
  // Click (not drag) on a node → open the inspector + swing it to the front.
  el.addEventListener("click", (e) => {
    if (dragMoved) return;
    const hit = nodeAt(e);
    if (hit) { inspectNode(hit.userData); focusOn(hit); }
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

// Smoothly rotate the world so the clicked node faces the camera.
function focusOn(mesh) {
  const p = mesh.position;
  // Camera looks down -Z; a node at world angle a faces front when the world
  // is rotated so the node lands near +Z.
  const a = Math.atan2(p.x, p.z);
  const want = -a;
  // Take the shortest path from the current rotation.
  let delta = (want - targetRotY) % (Math.PI * 2);
  if (delta > Math.PI) delta -= Math.PI * 2;
  if (delta < -Math.PI) delta += Math.PI * 2;
  targetRotY += delta;
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
    (detail ? `<pre class="mi-detail">${escHtml(detail).slice(0, 1200)}</pre>` : "");
  box.querySelector(".mi-close").onclick = () => { box.hidden = true; };
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

  // ---- lifecycle helix ----
  const visible = timeline.filter((nd) => filters[nd.type] !== false);
  const n = visible.length || 1;
  const R = 5.4, step = Math.min(0.6, 9 / n);
  const pts = [];
  visible.forEach((nd, i) => {
    const a = i * 0.7;
    const p = new THREE.Vector3(Math.cos(a) * R, (i - (n - 1) / 2) * step, Math.sin(a) * R);
    pts.push(p);
    const mesh = makeNodeMesh(nd, p, nd.type === "commit" || nd.type === "error" ? 0.2 : 0.16);
    if (nd.idx != null && newFromIdx != null && nd.idx >= newFromIdx) {
      mesh.scale.set(0.01, 0.01, 0.01);
      spawnQueue.push({ mesh, t0: clock });
    }
    activeMesh = mesh; // ends on the newest visible node
  });

  if (pts.length > 1) {
    // Synapse line along the governed chain + a signal that travels it.
    nodesGroup.add(new THREE.Line(
      new THREE.BufferGeometry().setFromPoints(pts),
      new THREE.LineBasicMaterial({ color: 0x8a7fe0, transparent: true, opacity: 0.5 })));
    chainCurve = new THREE.CatmullRomCurve3(pts);
    signal = new THREE.Sprite(new THREE.SpriteMaterial(
      { map: glowTex, color: runStatus === "failed" ? RED : CYAN, transparent: true,
        blending: THREE.AdditiveBlending, opacity: 0.95 }));
    signal.scale.set(0.9, 0.9, 1);
    nodesGroup.add(signal);
  }

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

  const RC = 7.6;
  ctxNodes.forEach((nd, i) => {
    const a = (i / Math.max(ctxNodes.length, 1)) * Math.PI * 2 + 0.5;
    nd.color = (TYPES[nd.type] || TYPES.system).color;
    const p = new THREE.Vector3(Math.cos(a) * RC, ((i % 3) - 1) * 1.4, Math.sin(a) * RC);
    makeNodeMesh(nd, p, 0.26);
    const lbl = labelSprite(nd.title, hex(nd.color));
    lbl.position.set(p.x, p.y - 0.65, p.z);
    nodesGroup.add(lbl);
    // Tether to the core (the run itself).
    line(new THREE.Vector3(0, 0, 0), p, nd.color, 0.18);
    // Tool nodes also connect to the lifecycle entries that used them.
    if (nd.type === "tool") {
      let edges = 0;
      visible.forEach((tnd, j) => {
        if (tnd.tool === nd.tool && edges < 14) {
          line(p, pts[j], nd.color, 0.14);
          edges++;
        }
      });
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
let prevMaxIdx = -1;

async function loadRun(idRaw) {
  let id = (idRaw || "").trim();
  try {
    if (!id) {
      // Prefer the chat's current run (set by main.js) over "latest overall",
      // so Mind opens on what the user is actually doing.
      id = window.thymosActiveRun || "";
    }
    if (!id) {
      const runs = await (await fetch(`${BASE}/runs`)).json();
      const list = Array.isArray(runs) ? runs : runs.runs || [];
      id = list[0]?.run_id || list[0]?.trajectory_id || "";
    }
    if (!id) return;
    const changedRun = id !== currentRunId;
    if (changedRun) prevMaxIdx = -1;
    currentRunId = id;

    // Live session log (rich: phases, tools, errors, timestamps). Falls back
    // to the ledger's audit entries for runs restored without a session.
    let snap = null;
    try { snap = await (await fetch(`${BASE}/runs/${id}/execution`)).json(); } catch (_) {}
    let timeline = [];
    if (snap?.log?.length > 1) {
      timeline = snap.log.map((e) => ({
        idx: e.idx, type: classifyLog(e), title: e.title, detail: e.detail,
        time: e.timestamp_ms, step: e.step_index, tool: e.tool || null,
        run: id, color: 0,
      }));
    } else {
      const data = await (await fetch(`${BASE}/audit/entries?run_id=${encodeURIComponent(id)}`)).json();
      const entries = Array.isArray(data) ? data : data.entries || [];
      timeline = entries.map((en, i) => ({
        idx: i, type: classifyAudit(en), title: en.kind, detail: en.detail,
        seq: en.seq, id: en.commit_id || en.id, run: id, color: 0,
      }));
    }
    timeline = timeline.slice(-MAX_TIMELINE);
    timeline.forEach((nd) => { nd.color = (TYPES[nd.type] || TYPES.system).color; });

    runStatus = snap?.status || "";

    // Context: provider/mode, the tools this run actually used, replay verdict.
    let health = null, replay = null;
    try { health = await (await fetch(`${BASE}/health`)).json(); } catch (_) {}
    try {
      const r = await fetch(`${BASE}/runs/${id}/replay`);
      if (r.ok) replay = await r.json();
    } catch (_) {}
    const tools = [...new Set(timeline.map((nd) => nd.tool).filter(Boolean))].slice(0, 8);
    const ctx = {
      provider: health?.default_provider || "",
      live: !!health?.cognition_live,
      tools,
      replayOk: !!replay,
    };

    // Rebuild when the run, entry count, or filters changed — live polling
    // otherwise leaves the scene untouched so motion stays continuous.
    const filterKey = JSON.stringify(filters);
    const maxIdx = timeline.length ? timeline[timeline.length - 1].idx : -1;
    if (changedRun || timeline.length !== lastCount || filterKey !== lastFilterKey) {
      lastCount = timeline.length;
      lastFilterKey = filterKey;
      buildGraph(timeline, ctx, changedRun ? null : prevMaxIdx + 1);
      prevMaxIdx = maxIdx;
    }

    const el = document.getElementById("mindRunId");
    if (el && !el.value) el.placeholder = `${id.slice(0, 8)} · ${timeline.length} events`;
    renderRunState(id, snap, health, replay);
  } catch (_) { /* runtime not up yet — the cage still renders */ }
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
  targetRotY += 0.0016;
  curRotY += (targetRotY - curRotY) * 0.08;
  curRotX += (targetRotX - curRotX) * 0.08;
  world.rotation.y = curRotY;
  world.rotation.x = curRotX;
  cage.rotation.y -= 0.0011;
  cage.rotation.z += 0.0007;

  // Pulse each node halo on its own phase (neural firing). Errors burn hotter.
  for (const p of pulses) {
    const amp = p.err ? 0.8 : 0.45;
    const s = 1.1 + amp * (0.5 + 0.5 * Math.sin(clock * 2.4 + p.phase));
    p.halo.scale.set(s, s, 1);
  }
  // New nodes scale in — work appearing as it happens.
  spawnQueue = spawnQueue.filter((sp) => {
    const k = Math.min((clock - sp.t0) / 0.45, 1);
    const e = 1 - Math.pow(1 - k, 3); // ease-out
    sp.mesh.scale.set(e, e, e);
    return k < 1;
  });
  // The newest node breathes while the run is live — "this is where I am".
  if (activeMesh && (runStatus === "running" || runStatus === "waiting_approval")) {
    const s = 1 + 0.5 * (0.5 + 0.5 * Math.sin(clock * 5));
    activeMesh.scale.set(s, s, s);
  }
  // Propagate a signal along the chain: fast while thinking, calm when the
  // run has stabilized, halted (red) on failure.
  if (chainCurve && signal) {
    const speed = runStatus === "running" || runStatus === "waiting_approval" ? 0.14
      : runStatus === "failed" ? 0 : 0.05;
    if (speed > 0) {
      const t = (clock * speed) % 1;
      chainCurve.getPointAt(t, signal.position);
      const s = 0.7 + 0.5 * Math.sin(clock * 6);
      signal.scale.set(s, s, 1);
    } else {
      // Failure: the signal parks on the last node, burning red.
      chainCurve.getPointAt(1, signal.position);
      signal.scale.set(1.2, 1.2, 1);
    }
  }
  renderer.render(scene, camera);
}
function start() {
  if (!running) { running = true; frame(); }
  // Live growth: while visible, re-poll the current run so streamed entries
  // appear in the graph. Cleared on stop to avoid background work.
  if (!refreshTimer) {
    refreshTimer = setInterval(() => {
      if (currentRunId) loadRun(currentRunId);
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
  document.getElementById("mindLoad")?.addEventListener("click", () =>
    loadRun(document.getElementById("mindRunId")?.value));

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
