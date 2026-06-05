/* THYMOS — hero 3D brand scene (three.js, CDN, graceful-degrading)
   Matches the OpenThymos logo art: a glowing violet wireframe "open box"
   (the cube) hovering over a particle portal of orbital rings and drifting
   stars, with bright "commit" pulses running along the cube's edges.
   Self-guards: no container / no WebGL / CDN failure → hero text stands alone. */

const MOUNT = document.getElementById('hero-canvas');
const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

if (MOUNT) {
  import('https://cdn.jsdelivr.net/npm/three@0.160.0/build/three.module.js')
    .then((THREE) => boot(THREE))
    .catch(() => { /* CDN/WebGL unavailable — hero text stands on its own */ });
}

function boot(THREE) {
  // ----- brand palette (mirrors the logo + site.css) -----
  const VIOLET = new THREE.Color('#a855f7');
  const VIOLET_HI = new THREE.Color('#c77dff');
  const LAVENDER = new THREE.Color('#e7d2ff');
  const STAR = new THREE.Color('#8be9ff');
  const DEEP = new THREE.Color('#7c3aed');
  const BG = new THREE.Color('#08040f');

  const scene = new THREE.Scene();
  scene.fog = new THREE.Fog(BG, 40, 110);

  const camera = new THREE.PerspectiveCamera(56, mountAspect(), 0.1, 300);
  camera.position.set(0, 6, 60);
  camera.lookAt(0, -1, 0);

  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, powerPreference: 'high-performance' });
  } catch { return; }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  setSize();
  renderer.domElement.setAttribute('aria-hidden', 'true');
  MOUNT.appendChild(renderer.domElement);
  const PR = renderer.getPixelRatio();

  const world = new THREE.Group();
  world.rotation.x = -0.34; // gentle isometric look-down, like the logo art
  scene.add(world);

  // additive glow point material (shared shader)
  const pointMat = (sizePx) => new THREE.ShaderMaterial({
    transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
    uniforms: { uPR: { value: PR }, uTime: { value: 0 }, uSize: { value: sizePx } },
    vertexShader: `
      attribute vec3 aColor; attribute float aScale; attribute float aPhase;
      uniform float uPR; uniform float uTime; uniform float uSize;
      varying vec3 vColor; varying float vA;
      void main() {
        vColor = aColor;
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        float tw = 0.65 + 0.35 * sin(uTime * 1.7 + aPhase);
        vA = tw;
        gl_PointSize = uSize * aScale * uPR * tw * (60.0 / -mv.z);
        gl_Position = projectionMatrix * mv;
      }`,
    fragmentShader: `
      varying vec3 vColor; varying float vA;
      void main() {
        float d = length(gl_PointCoord - vec2(0.5));
        if (d > 0.5) discard;
        float core = smoothstep(0.5, 0.0, d);
        gl_FragColor = vec4(vColor, core * core * vA);
      }`,
  });

  // =========================================================
  // 1) The cube — wireframe "open box"
  // =========================================================
  const H = 11; // half-size
  const CUBE_Y = 3;
  const corners = [
    [-H,-H,-H],[ H,-H,-H],[ H, H,-H],[-H, H,-H],
    [-H,-H, H],[ H,-H, H],[ H, H, H],[-H, H, H],
  ].map((c) => new THREE.Vector3(c[0], c[1] + CUBE_Y, c[2]));
  const cubeEdges = [
    [0,1],[1,2],[2,3],[3,0], [4,5],[5,6],[6,7],[7,4], [0,4],[1,5],[2,6],[3,7],
  ];
  const cube = new THREE.Group();
  world.add(cube);

  // edges
  const ePos = new Float32Array(cubeEdges.length * 2 * 3);
  cubeEdges.forEach(([a, b], k) => {
    ePos.set([corners[a].x, corners[a].y, corners[a].z], k * 6);
    ePos.set([corners[b].x, corners[b].y, corners[b].z], k * 6 + 3);
  });
  const eGeo = new THREE.BufferGeometry();
  eGeo.setAttribute('position', new THREE.BufferAttribute(ePos, 3));
  cube.add(new THREE.LineSegments(eGeo, new THREE.LineBasicMaterial({
    color: VIOLET, transparent: true, opacity: 0.55, blending: THREE.AdditiveBlending, depthWrite: false,
  })));
  // a second, larger faint cube for a halo of depth
  cube.add(new THREE.LineSegments(eGeo, new THREE.LineBasicMaterial({
    color: DEEP, transparent: true, opacity: 0.18, blending: THREE.AdditiveBlending, depthWrite: false,
  })).clone());

  // bright corner nodes
  const cn = makePoints(corners.map((p) => ({ p, color: VIOLET_HI, scale: 2.0 })), 11);
  cube.add(cn.points);

  // =========================================================
  // 2) Commit pulses — walk the cube edges
  // =========================================================
  const adj = corners.map(() => []);
  cubeEdges.forEach(([a, b]) => { adj[a].push(b); adj[b].push(a); });
  const PULSES = reduceMotion ? 0 : 8;
  const pulse = makePoints(
    Array.from({ length: Math.max(PULSES, 1) }, () => ({ p: corners[0].clone(), color: LAVENDER, scale: 1.0 })),
    30
  );
  cube.add(pulse.points);
  const walkers = [];
  for (let i = 0; i < PULSES; i++) {
    const from = (Math.random() * 8) | 0;
    walkers.push({ from, to: adj[from][(Math.random() * adj[from].length) | 0], t: Math.random(), speed: 0.3 + Math.random() * 0.4 });
  }

  // =========================================================
  // 3) Portal — concentric orbital rings on a flat plane
  // =========================================================
  const PORTAL_Y = -15;
  const portal = new THREE.Group();
  portal.position.y = PORTAL_Y;
  portal.rotation.x = -Math.PI / 2; // lay the rings flat
  world.add(portal);

  const ringDefs = [
    { r: 17, op: 0.5, color: VIOLET_HI },
    { r: 25, op: 0.3, color: VIOLET },
    { r: 34, op: 0.18, color: DEEP },
  ];
  ringDefs.forEach(({ r, op, color }) => {
    const seg = 160;
    const pos = new Float32Array((seg + 1) * 3);
    for (let i = 0; i <= seg; i++) {
      const a = (i / seg) * Math.PI * 2;
      pos.set([Math.cos(a) * r, Math.sin(a) * r, 0], i * 3);
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
    portal.add(new THREE.Line(g, new THREE.LineBasicMaterial({
      color, transparent: true, opacity: op, blending: THREE.AdditiveBlending, depthWrite: false,
    })));
  });

  // particle disc on the portal plane + ambient starfield
  const discN = 320;
  const discItems = [];
  for (let i = 0; i < discN; i++) {
    const a = Math.random() * Math.PI * 2;
    const rad = 15 + Math.pow(Math.random(), 0.7) * 28;
    const p = new THREE.Vector3(Math.cos(a) * rad, Math.sin(a) * rad, (Math.random() - 0.5) * 1.6);
    const roll = Math.random();
    discItems.push({ p, color: roll > 0.86 ? STAR : roll > 0.5 ? VIOLET_HI : VIOLET, scale: 0.5 + Math.random() * 1.1 });
  }
  const disc = makePoints(discItems, 8);
  portal.add(disc.points);

  const ambN = 220;
  const ambItems = [];
  for (let i = 0; i < ambN; i++) {
    const p = new THREE.Vector3((Math.random() - 0.5) * 130, (Math.random() - 0.5) * 70, (Math.random() - 0.5) * 90);
    ambItems.push({ p, color: Math.random() > 0.9 ? STAR : VIOLET, scale: 0.4 + Math.random() * 0.8 });
  }
  const amb = makePoints(ambItems, 7);
  world.add(amb.points);

  // =========================================================
  // interaction + loop
  // =========================================================
  const pointer = { x: 0, y: 0, tx: 0, ty: 0 };
  if (!reduceMotion) {
    window.addEventListener('pointermove', (e) => {
      pointer.tx = (e.clientX / window.innerWidth - 0.5) * 2;
      pointer.ty = (e.clientY / window.innerHeight - 0.5) * 2;
    }, { passive: true });
  }

  const mats = [cn.mat, pulse.mat, disc.mat, amb.mat];
  let running = false, raf = 0, last = performance.now();
  let t = 0;

  function frame(now) {
    if (!running) return;
    const dt = Math.min((now - last) / 1000, 0.05);
    last = now; t += dt;

    cube.rotation.y += dt * 0.18;
    portal.rotation.z -= dt * 0.06;
    amb.points.rotation.y += dt * 0.01;
    world.rotation.z = Math.sin(t * 0.1) * 0.03;

    pointer.x += (pointer.tx - pointer.x) * 0.04;
    pointer.y += (pointer.ty - pointer.y) * 0.04;
    camera.position.x = pointer.x * 6;
    camera.position.y = 6 - pointer.y * 4;
    camera.lookAt(0, -1, 0);

    for (const m of mats) m.uniforms.uTime.value = t;
    if (PULSES) stepWalkers(dt);

    renderer.render(scene, camera);
    raf = requestAnimationFrame(frame);
  }

  function stepWalkers(dt) {
    const arr = pulse.points.geometry.attributes.position.array;
    for (let i = 0; i < walkers.length; i++) {
      const w = walkers[i];
      w.t += w.speed * dt;
      while (w.t >= 1) {
        w.t -= 1; w.from = w.to;
        const nb = adj[w.from];
        w.to = nb[(Math.random() * nb.length) | 0];
      }
      const a = corners[w.from], b = corners[w.to];
      arr[i * 3] = a.x + (b.x - a.x) * w.t;
      arr[i * 3 + 1] = a.y + (b.y - a.y) * w.t;
      arr[i * 3 + 2] = a.z + (b.z - a.z) * w.t;
    }
    pulse.points.geometry.attributes.position.needsUpdate = true;
  }

  function start() { if (running) return; running = true; last = performance.now(); raf = requestAnimationFrame(frame); }
  function stop() { running = false; if (raf) cancelAnimationFrame(raf); raf = 0; }

  if ('IntersectionObserver' in window) {
    new IntersectionObserver((es) => es.forEach((e) => (e.isIntersecting ? start() : stop())), { threshold: 0.01 }).observe(MOUNT);
  } else { start(); }
  document.addEventListener('visibilitychange', () => (document.hidden ? stop() : (!reduceMotion && start())));
  window.addEventListener('resize', onResize, { passive: true });

  if (reduceMotion) renderer.render(scene, camera); else start();

  // ----- helpers -----
  function makePoints(items, sizePx) {
    const n = items.length;
    const pos = new Float32Array(n * 3), col = new Float32Array(n * 3);
    const scl = new Float32Array(n), pha = new Float32Array(n);
    items.forEach((it, i) => {
      pos.set([it.p.x, it.p.y, it.p.z], i * 3);
      col.set([it.color.r, it.color.g, it.color.b], i * 3);
      scl[i] = it.scale; pha[i] = Math.random() * 6.28;
    });
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
    g.setAttribute('aColor', new THREE.BufferAttribute(col, 3));
    g.setAttribute('aScale', new THREE.BufferAttribute(scl, 1));
    g.setAttribute('aPhase', new THREE.BufferAttribute(pha, 1));
    const mat = pointMat(sizePx);
    return { points: new THREE.Points(g, mat), mat };
  }
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
    if (reduceMotion || !running) renderer.render(scene, camera);
  }
}
