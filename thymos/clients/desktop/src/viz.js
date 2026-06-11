// Mind — an immersive 3D view of a run's reasoning: the OpenThymos logo
// suspended in a rotating wireframe cage, with the governed ledger DAG (commits,
// rejections, suspensions) as glowing nodes orbiting it. Pure client: reads
// `/audit/entries` from the local runtime; three.js is vendored (no egress).
import * as THREE from "./vendor/three.module.js";

const invoke = window.__TAURI__?.core?.invoke;
let BASE = "http://127.0.0.1:3001";
(async () => { try { if (invoke) BASE = await invoke("runtime_addr"); } catch (_) {} })();

const VIOLET = 0x7c5cff, CYAN = 0x45e0ff, GREEN = 0x46d39a, RED = 0xff6b8a, AMBER = 0xffc24b;
let nodeMeshes = [], raycaster, pointer, dragMoved = false;
function colorFor(kind) {
  const k = (kind || "").toLowerCase();
  if (k.includes("commit")) return GREEN;
  if (k.includes("reject")) return RED;
  if (k.includes("approval") || k.includes("suspend")) return AMBER;
  if (k.includes("delegation")) return VIOLET;
  if (k.includes("root")) return CYAN;
  return VIOLET;
}

let renderer, scene, camera, world, cage, nodesGroup, glowTex;
let inited = false, running = false, raf = 0, loadedOnce = false;
let targetRotY = 0, targetRotX = 0.25, curRotY = 0, curRotX = 0.25;
let drag = null;
// Neural animation state: per-node halos to pulse, the chain as a curve, a
// signal that travels it, and a live-refresh timer so the graph grows as a run
// streams.
let pulses = [], chainCurve = null, signal = null, clock = 0;
let currentRunId = "", refreshTimer = 0, lastCount = -1;

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
  // Click (not drag) on a node → open the inspector.
  el.addEventListener("click", (e) => {
    if (dragMoved) return;
    const hit = nodeAt(e);
    if (hit) inspectNode(hit);
  });
  // Hover → quick tooltip + pointer cursor.
  el.addEventListener("pointermove", (e) => {
    if (drag) return;
    const hit = nodeAt(e);
    el.style.cursor = hit ? "pointer" : "grab";
    if (hit) showTip(e, hit); else hideTip();
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
    o.material?.dispose?.();
  }
  nodeMeshes = [];
}

/* ---------- inspectable nodes ---------- */
// Raycast the pointer event against node meshes; return the hit entry or null.
function nodeAt(e) {
  if (!raycaster || !nodeMeshes.length) return null;
  const rect = renderer.domElement.getBoundingClientRect();
  pointer.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
  raycaster.setFromCamera(pointer, camera);
  const hits = raycaster.intersectObjects(nodeMeshes, false);
  return hits.length ? hits[0].object.userData : null;
}

// Human-readable one-liner for an entry kind.
function summaryFor(en) {
  const k = (en.kind || "").toLowerCase();
  if (k.includes("root")) return "Trajectory root — the run began.";
  if (k.includes("commit")) return "Commit — an authorized action that mutated world state.";
  if (k.includes("rejection") || k.includes("reject")) return "Rejected — a proposal the runtime refused.";
  if (k.includes("approval") || k.includes("pending")) return "Suspended — waiting for human approval.";
  if (k.includes("delegation") || k.includes("deleg")) return "Delegation — authority handed to a child run.";
  if (k.includes("skill")) return "Skill bound — a skill narrowed this run's authority.";
  return en.kind || "Ledger entry";
}

function inspectNode(en) {
  const box = document.getElementById("mindInspector");
  if (!box) return;
  const id = en.commit_id || en.id || "";
  const detail = en.detail ? (typeof en.detail === "string" ? en.detail : JSON.stringify(en.detail, null, 2)) : "";
  box.hidden = false;
  box.innerHTML =
    `<div class="mi-head"><span class="mi-kind">${escHtml(en.kind || "entry")}</span>` +
    `<button class="mi-close" title="close">×</button></div>` +
    `<div class="mi-sum">${escHtml(summaryFor(en))}</div>` +
    `<dl class="mi-fields">` +
    (en.seq != null ? `<dt>seq</dt><dd>${en.seq}</dd>` : "") +
    (id ? `<dt>id</dt><dd class="mono">${escHtml(String(id).slice(0, 24))}…</dd>` : "") +
    `</dl>` +
    (detail ? `<pre class="mi-detail">${escHtml(detail).slice(0, 1200)}</pre>` : "");
  box.querySelector(".mi-close").onclick = () => { box.hidden = true; };
}

let tipEl = null;
function showTip(e, en) {
  if (!tipEl) tipEl = document.getElementById("mindTip");
  if (!tipEl) return;
  tipEl.hidden = false;
  tipEl.textContent = `${en.kind || "entry"}${en.seq != null ? " · seq " + en.seq : ""} — click to inspect`;
  const rect = renderer.domElement.getBoundingClientRect();
  tipEl.style.left = (e.clientX - rect.left + 12) + "px";
  tipEl.style.top = (e.clientY - rect.top + 12) + "px";
}
function hideTip() { if (tipEl) tipEl.hidden = true; }
function escHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function placeNodes(entries) {
  clearNodes();
  pulses = [];
  chainCurve = null;
  signal = null;
  const n = entries.length || 1;
  const R = 5.4, step = 0.6;
  const pts = [];
  entries.forEach((e, i) => {
    const a = i * 0.7;
    const y = (i - (n - 1) / 2) * step;
    const p = new THREE.Vector3(Math.cos(a) * R, y, Math.sin(a) * R);
    pts.push(p);
    const col = colorFor(e.kind);
    const node = new THREE.Mesh(
      new THREE.SphereGeometry(0.16, 16, 16),
      new THREE.MeshBasicMaterial({ color: col }));
    node.position.copy(p);
    node.userData = e; // the ledger entry — makes the node inspectable
    nodesGroup.add(node);
    nodeMeshes.push(node);
    const halo = new THREE.Sprite(new THREE.SpriteMaterial(
      { map: glowTex, color: col, transparent: true, blending: THREE.AdditiveBlending, opacity: 0.8 }));
    halo.scale.set(1.1, 1.1, 1);
    halo.position.copy(p);
    nodesGroup.add(halo);
    // Each node fires on its own phase, so the graph pulses like a network.
    pulses.push({ halo, phase: i * 0.6 });
  });
  if (pts.length > 1) {
    // Synapse lines between consecutive entries (the governed chain).
    nodesGroup.add(new THREE.Line(
      new THREE.BufferGeometry().setFromPoints(pts),
      new THREE.LineBasicMaterial({ color: 0x8a7fe0, transparent: true, opacity: 0.5 })));
    // A signal that propagates along the chain — intent → proposal → commit.
    chainCurve = new THREE.CatmullRomCurve3(pts);
    signal = new THREE.Sprite(new THREE.SpriteMaterial(
      { map: glowTex, color: CYAN, transparent: true, blending: THREE.AdditiveBlending, opacity: 0.95 }));
    signal.scale.set(0.9, 0.9, 1);
    nodesGroup.add(signal);
  }
}

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
    currentRunId = id;
    const data = await (await fetch(`${BASE}/audit/entries?run_id=${encodeURIComponent(id)}`)).json();
    const entries = Array.isArray(data) ? data : data.entries || [];
    // Rebuild when the run changed or its entry count grew, so live polling
    // doesn't restart the animation every tick — new entries just grow the graph.
    if (changedRun || entries.length !== lastCount) {
      lastCount = entries.length;
      placeNodes(entries);
    }
    const el = document.getElementById("mindRunId");
    if (el && !el.value) el.placeholder = `${id.slice(0, 8)} · ${entries.length} entries`;
    await renderRunState(id);
  } catch (_) { /* runtime not up yet — the cage still renders */ }
}

// The live strip above the canvas: the run's real status, the runtime's
// current operator state, provider/model, governed counters, and the replay
// verdict — every field read from the runtime, none decorative.
async function renderRunState(id) {
  const box = document.getElementById("mindState");
  if (!box) return;
  let snap = null, health = null, replay = null;
  try { snap = await (await fetch(`${BASE}/runs/${id}/execution`)).json(); } catch (_) {}
  try { health = await (await fetch(`${BASE}/health`)).json(); } catch (_) {}
  try {
    const r = await fetch(`${BASE}/runs/${id}/replay`);
    if (r.ok) replay = await r.json();
  } catch (_) {}
  if (!snap) { box.hidden = true; return; }
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

  // Pulse each node halo on its own phase (neural firing).
  for (const p of pulses) {
    const s = 1.1 + 0.45 * (0.5 + 0.5 * Math.sin(clock * 2.4 + p.phase));
    p.halo.scale.set(s, s, 1);
  }
  // Propagate a signal along the chain.
  if (chainCurve && signal) {
    const t = (clock * 0.07) % 1;
    chainCurve.getPointAt(t, signal.position);
    const s = 0.7 + 0.5 * Math.sin(clock * 6);
    signal.scale.set(s, s, 1);
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

// Wire the Mind tab: lazy-init + render only while visible (saves CPU).
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
});
