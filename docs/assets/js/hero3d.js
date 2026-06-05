/* THYMOS — hero 3D execution-DAG (three.js, CDN, graceful-degrading)
   Renders a rotating node graph (the append-only ledger) with bright
   "commit" pulses flowing along its edges. Self-guards: does nothing if
   the container is absent, WebGL is unavailable, or the CDN fails to load. */

const MOUNT = document.getElementById('hero-canvas');
const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

if (MOUNT) {
  import('https://cdn.jsdelivr.net/npm/three@0.160.0/build/three.module.js')
    .then((THREE) => boot(THREE))
    .catch(() => { /* CDN/WebGL unavailable — hero text stands on its own */ });
}

function boot(THREE) {
  // ----- palette (mirrors site.css) -----
  const AMBER = new THREE.Color('#ffb547');
  const AMBER_SOFT = new THREE.Color('#ffe1a8');
  const CYAN = new THREE.Color('#4de0c4');
  const VIOLET = new THREE.Color('#b998ff');
  const BG = new THREE.Color('#05070b');

  const scene = new THREE.Scene();
  scene.fog = new THREE.Fog(BG, 34, 78);

  const camera = new THREE.PerspectiveCamera(58, mountAspect(), 0.1, 200);
  camera.position.set(0, 0, 46);

  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: 'high-performance' });
  } catch { return; }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  setSize();
  renderer.domElement.setAttribute('aria-hidden', 'true');
  MOUNT.appendChild(renderer.domElement);

  // ----- build the DAG: nodes in a flattened ellipsoid, edges by proximity -----
  const N = 88;
  const RX = 30, RY = 13.5, RZ = 21;
  const nodes = [];
  for (let i = 0; i < N; i++) {
    // even-ish distribution on a sphere, then squash to an ellipsoid
    const u = Math.random(), v = Math.random();
    const theta = 2 * Math.PI * u;
    const phi = Math.acos(2 * v - 1);
    const r = Math.cbrt(Math.random()) * 0.5 + 0.5; // bias outward, keep core sparse
    nodes.push(new THREE.Vector3(
      Math.sin(phi) * Math.cos(theta) * RX * r,
      Math.sin(phi) * Math.sin(theta) * RY * r,
      Math.cos(phi) * RZ * r
    ));
  }

  // proximity edges (capped), build adjacency for the pulse walk
  const D2 = 9.2 * 9.2;
  const edges = [];
  const adj = Array.from({ length: N }, () => []);
  for (let i = 0; i < N; i++) {
    for (let j = i + 1; j < N; j++) {
      if (nodes[i].distanceToSquared(nodes[j]) < D2) {
        edges.push([i, j]);
        adj[i].push(j); adj[j].push(i);
        if (edges.length > 260) break;
      }
    }
    if (edges.length > 260) break;
  }

  const group = new THREE.Group();
  scene.add(group);

  // ----- edges as additive glowing lines -----
  const edgePos = new Float32Array(edges.length * 2 * 3);
  edges.forEach(([a, b], k) => {
    edgePos.set([nodes[a].x, nodes[a].y, nodes[a].z], k * 6);
    edgePos.set([nodes[b].x, nodes[b].y, nodes[b].z], k * 6 + 3);
  });
  const edgeGeo = new THREE.BufferGeometry();
  edgeGeo.setAttribute('position', new THREE.BufferAttribute(edgePos, 3));
  const edgeMat = new THREE.LineBasicMaterial({
    color: AMBER, transparent: true, opacity: 0.16,
    blending: THREE.AdditiveBlending, depthWrite: false,
  });
  group.add(new THREE.LineSegments(edgeGeo, edgeMat));

  // ----- nodes as additive points -----
  const nodePos = new Float32Array(N * 3);
  const nodeCol = new Float32Array(N * 3);
  const nodeSize = new Float32Array(N);
  nodes.forEach((p, i) => {
    nodePos.set([p.x, p.y, p.z], i * 3);
    const c = i % 11 === 0 ? VIOLET : i % 7 === 0 ? CYAN : AMBER;
    nodeCol.set([c.r, c.g, c.b], i * 3);
    nodeSize[i] = (i % 7 === 0 || i % 11 === 0) ? 2.1 : 1.3;
  });
  const nodeGeo = new THREE.BufferGeometry();
  nodeGeo.setAttribute('position', new THREE.BufferAttribute(nodePos, 3));
  nodeGeo.setAttribute('aColor', new THREE.BufferAttribute(nodeCol, 3));
  nodeGeo.setAttribute('aSize', new THREE.BufferAttribute(nodeSize, 1));

  const nodeMat = new THREE.ShaderMaterial({
    transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
    uniforms: { uPixelRatio: { value: renderer.getPixelRatio() }, uTime: { value: 0 } },
    vertexShader: `
      attribute vec3 aColor; attribute float aSize;
      uniform float uPixelRatio; uniform float uTime;
      varying vec3 vColor; varying float vAlpha;
      void main() {
        vColor = aColor;
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        float pulse = 0.78 + 0.22 * sin(uTime * 1.6 + position.x * 0.4 + position.y * 0.6);
        vAlpha = pulse;
        gl_PointSize = aSize * 9.0 * uPixelRatio * pulse * (60.0 / -mv.z);
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: `
      varying vec3 vColor; varying float vAlpha;
      void main() {
        float d = length(gl_PointCoord - vec2(0.5));
        if (d > 0.5) discard;
        float core = smoothstep(0.5, 0.0, d);
        gl_FragColor = vec4(vColor, core * core * vAlpha);
      }`,
  });
  group.add(new THREE.Points(nodeGeo, nodeMat));

  // ----- commit pulses: bright sprites walking the DAG edges -----
  const PULSES = reduceMotion ? 0 : 7;
  const pulseGeo = new THREE.BufferGeometry();
  const pulsePos = new Float32Array(Math.max(PULSES, 1) * 3);
  const pulseCol = new Float32Array(Math.max(PULSES, 1) * 3);
  pulseGeo.setAttribute('position', new THREE.BufferAttribute(pulsePos, 3));
  pulseGeo.setAttribute('aColor', new THREE.BufferAttribute(pulseCol, 3));
  const pulseMat = new THREE.ShaderMaterial({
    transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
    uniforms: { uPixelRatio: { value: renderer.getPixelRatio() } },
    vertexShader: `
      attribute vec3 aColor; uniform float uPixelRatio; varying vec3 vColor;
      void main() {
        vColor = aColor;
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = 26.0 * uPixelRatio * (60.0 / -mv.z);
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: `
      varying vec3 vColor;
      void main() {
        float d = length(gl_PointCoord - vec2(0.5));
        if (d > 0.5) discard;
        float core = smoothstep(0.5, 0.0, d);
        gl_FragColor = vec4(vColor, core * core);
      }`,
  });
  const pulsePoints = new THREE.Points(pulseGeo, pulseMat);
  group.add(pulsePoints);

  const walkers = [];
  for (let i = 0; i < PULSES; i++) {
    const from = (Math.random() * N) | 0;
    const to = adj[from].length ? adj[from][(Math.random() * adj[from].length) | 0] : from;
    walkers.push({ from, to, t: Math.random(), speed: 0.18 + Math.random() * 0.22 });
  }

  function stepWalkers(dt) {
    for (let i = 0; i < walkers.length; i++) {
      const w = walkers[i];
      w.t += w.speed * dt;
      while (w.t >= 1) {
        w.t -= 1;
        w.from = w.to;
        const nbrs = adj[w.from];
        w.to = nbrs.length ? nbrs[(Math.random() * nbrs.length) | 0] : w.from;
      }
      const a = nodes[w.from], b = nodes[w.to];
      const x = a.x + (b.x - a.x) * w.t;
      const y = a.y + (b.y - a.y) * w.t;
      const z = a.z + (b.z - a.z) * w.t;
      pulsePos.set([x, y, z], i * 3);
      const c = i % 2 ? CYAN : AMBER_SOFT;
      pulseCol.set([c.r, c.g, c.b], i * 3);
    }
    pulseGeo.attributes.position.needsUpdate = true;
    pulseGeo.attributes.aColor.needsUpdate = true;
  }

  // ----- interaction: gentle pointer parallax -----
  const pointer = { x: 0, y: 0, tx: 0, ty: 0 };
  if (!reduceMotion) {
    window.addEventListener('pointermove', (e) => {
      pointer.tx = (e.clientX / window.innerWidth - 0.5) * 2;
      pointer.ty = (e.clientY / window.innerHeight - 0.5) * 2;
    }, { passive: true });
  }

  // ----- run loop, paused when offscreen or tab hidden -----
  let running = true, raf = 0, last = performance.now();
  const clock = { t: 0 };

  function frame(now) {
    if (!running) return;
    const dt = Math.min((now - last) / 1000, 0.05);
    last = now;
    clock.t += dt;

    group.rotation.y += dt * 0.07;
    group.rotation.x = Math.sin(clock.t * 0.12) * 0.12;

    pointer.x += (pointer.tx - pointer.x) * 0.04;
    pointer.y += (pointer.ty - pointer.y) * 0.04;
    camera.position.x = pointer.x * 4.5;
    camera.position.y = -pointer.y * 3.0;
    camera.lookAt(0, 0, 0);

    nodeMat.uniforms.uTime.value = clock.t;
    if (PULSES) stepWalkers(dt);

    renderer.render(scene, camera);
    raf = requestAnimationFrame(frame);
  }

  function start() {
    if (running && raf) return;
    running = true; last = performance.now();
    raf = requestAnimationFrame(frame);
  }
  function stop() { running = false; if (raf) cancelAnimationFrame(raf); raf = 0; }

  // pause when the hero leaves the viewport
  if ('IntersectionObserver' in window) {
    new IntersectionObserver((entries) => {
      entries.forEach((e) => (e.isIntersecting ? start() : stop()));
    }, { threshold: 0.01 }).observe(MOUNT);
  }
  document.addEventListener('visibilitychange', () => (document.hidden ? stop() : start()));
  window.addEventListener('resize', onResize, { passive: true });

  if (reduceMotion) {
    // static single frame — depth and structure, no motion
    renderer.render(scene, camera);
  } else {
    start();
  }

  // ----- helpers -----
  function mountAspect() {
    const r = MOUNT.getBoundingClientRect();
    return Math.max(r.width, 1) / Math.max(r.height, 1);
  }
  function setSize() {
    const r = MOUNT.getBoundingClientRect();
    renderer.setSize(r.width, r.height, false);
  }
  function onResize() {
    camera.aspect = mountAspect();
    camera.updateProjectionMatrix();
    setSize();
    nodeMat.uniforms.uPixelRatio.value = renderer.getPixelRatio();
    pulseMat.uniforms.uPixelRatio.value = renderer.getPixelRatio();
    if (reduceMotion || !running) renderer.render(scene, camera);
  }
}
