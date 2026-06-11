---
layout: default
title: OpenThymos
hide_title: true
hero: true
permalink: /
---

<section class="hero">
  <div class="hero-canvas" id="hero-canvas" aria-hidden="true"></div>

  <div class="hero-brand">
    <img src="{{ '/assets/img/thymos-mark.png' | relative_url }}" alt="" width="46" height="46" />
    <span class="hero-wordmark">OPEN<em>THYMOS</em></span>
  </div>

  <div class="hero-eyebrow">
    <span class="dot"></span>
    GOVERNED EXECUTION RUNTIME FOR AI AGENTS
  </div>

  <h1>
    Cognition proposes.<br>
    The runtime governs.<br>
    The ledger records.
  </h1>

  <p class="lede">
    An AI agent runtime where the model can never act on its own. Every effect
    passes through a typed proposal, a signed capability writ, and an
    append-only ledger you can replay and verify — local-first, your keys never
    leave your machine.
  </p>

  <div class="hero-cta">
    <a class="btn btn-primary btn-lg" href="#downloads">
      ⬇ Download Desktop <span class="btn-arrow">→</span>
    </a>
    <a class="btn btn-secondary btn-lg" href="#download-cli">
      ⌨ Download CLI
    </a>
    <a class="btn btn-ghost btn-lg" href="{{ site.repo }}" target="_blank" rel="noopener">
      <svg width="15" height="15" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8a8 8 0 0 0 5.47 7.59c.4.07.55-.17.55-.38v-1.34c-2.22.48-2.69-1.07-2.69-1.07-.36-.92-.89-1.17-.89-1.17-.73-.5.05-.49.05-.49.81.06 1.23.83 1.23.83.72 1.23 1.88.88 2.34.67.07-.52.28-.88.51-1.08-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.58.82-2.14-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.03 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.14 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48v2.19c0 .21.15.46.55.38A8 8 0 0 0 16 8c0-4.42-3.58-8-8-8z"/></svg>
      GitHub
    </a>
  </div>

  <a class="release-badge" href="https://github.com/gryszzz/open-thymos/releases/latest">
    <span class="rb-dot"></span>
    Stable
    <img src="https://img.shields.io/github/v/release/gryszzz/open-thymos?style=flat-square&label=&color=2a1d49" alt="latest release version" height="18" />
    · release notes
  </a>

  <div class="hero-shot reveal">
    <img
      src="{{ '/assets/img/hero-atlas.jpg' | relative_url }}"
      alt="OpenThymos — governing machine authority: intents, proposals, writs, governance, ledger, execution, and replay around the runtime core"
      loading="eager" decoding="async" />
  </div>

  <div class="hero-meta">
    <span class="mono">Intent → Proposal → Commit</span>
    <span>·</span>
    <span>Signed capability writs</span>
    <span>·</span>
    <span>Deterministic replay</span>
  </div>
</section>

<section class="section">
  <div class="section-h reveal">
    <p class="eyebrow">Governed AI runtime</p>
    <h2>The model never holds the keys.</h2>
  </div>
  <div class="triad reveal">
    <div class="card">
      <h4>Governed</h4>
      <p>The model proposes. The runtime decides.</p>
      <p class="sub">
        Cognition enters the system through one narrow contract: emit intents.
        Authority, policy, and execution are runtime concerns — not model concerns.
      </p>
    </div>
    <div class="card">
      <h4>Auditable</h4>
      <p>Every effect is a ledger event.</p>
      <p class="sub">
        Commits, rejections, approvals, delegations, and branches are all written
        to the same append-only, content-addressed ledger. Nothing is out-of-band.
      </p>
    </div>
    <div class="card">
      <h4>Replayable</h4>
      <p>World state is a committed delta projection.</p>
      <p class="sub">
        Replay recomputes the world by folding committed deltas in order. It
        verifies the hash chain, sequence continuity, and compiler version — without
        calling providers.
      </p>
    </div>
  </div>
</section>

<section class="section">
  <div class="split reveal">
    <div class="split-copy">
      <p class="eyebrow">Desktop control center</p>
      <h2>Chat with the agent.<br>Watch it think in Mind.</h2>
      <p>
        The desktop app is a local-first control center: chat drives governed
        runs, <strong>Mind</strong> renders the live reasoning graph — every
        node a real runtime record — and Runs, Audit, and Backups expose the
        ledger underneath. Connect Claude, OpenAI, Ollama, LM Studio, or any
        OpenAI-compatible model; keys stay on your machine.
      </p>
      <ul class="ticks">
        <li>Approve or deny risky actions before they run</li>
        <li>Live status: planning, executing, awaiting approval</li>
        <li>One-click audit trail and replay verification</li>
      </ul>
      <a class="btn btn-primary" href="#downloads">⬇ Download Desktop <span class="btn-arrow">→</span></a>
    </div>
    <figure class="split-shot">
      <img src="{{ '/assets/img/desktop-mind.png' | relative_url }}"
           alt="OpenThymos Desktop — the Mind view rendering a run's reasoning graph in 3D"
           loading="lazy" decoding="async" />
      <figcaption>Mind — the run's reasoning as a living graph. Every node is inspectable.</figcaption>
    </figure>
  </div>
</section>

<section class="section">
  <div class="split rev reveal">
    <div class="split-copy">
      <p class="eyebrow">CLI + runtime</p>
      <h2>The same governed runtime,<br>in your terminal.</h2>
      <p>
        <code>thymos</code> drives the identical runtime the desktop uses —
        same providers, same storage, same governance. Run tasks and watch
        <strong>Intent → Proposal → Commit</strong> stream live, approve
        suspended actions inline, and script everything with
        <code>--json</code>.
      </p>
      <pre class="cli-line"><code>curl -fsSL …/scripts/get.sh | sh   <span class="dim"># installs thymos + thymos-server</span></code></pre>
      <a class="btn btn-secondary" href="#download-cli">⌨ Download CLI</a>
    </div>
    <figure class="split-shot">
      <img src="{{ '/assets/img/cli-terminal.jpg' | relative_url }}"
           alt="The thymos CLI streaming a governed run in the terminal"
           loading="lazy" decoding="async" />
      <figcaption>thymos run "…" --follow — the governance feed, live in the terminal.</figcaption>
    </figure>
  </div>
</section>

<section class="section">
  <div class="section-h reveal">
    <p class="eyebrow">Skills · tools · grants</p>
    <h2>Authority is granted, never assumed.</h2>
  </div>
  <div class="triad reveal">
    <div class="card">
      <h4>Skills</h4>
      <p>Reusable, authority-narrowing templates.</p>
      <p class="sub">
        Binding a skill can only <em>shrink</em> what a run may do — tools
        intersect, ceilings AND, budgets take the minimum. Never widen.
      </p>
    </div>
    <div class="card">
      <h4>Typed tools</h4>
      <p>Capabilities are contracts, not blobs.</p>
      <p class="sub">
        Every tool declares schema, effect class, and risk before it can
        execute — built-in Rust tools, JSON manifests, and MCP bridges alike.
      </p>
    </div>
    <div class="card">
      <h4>Grants &amp; approvals</h4>
      <p>Dangerous actions pause for you.</p>
      <p class="sub">
        High-risk tools like <code>shell</code> suspend for an explicit
        operator sign-off — in the desktop or inline in the CLI — even when
        the writ permits them.
      </p>
    </div>
  </div>
</section>

<section class="section">
  <div class="section-h reveal">
    <p class="eyebrow">Audit &amp; replay</p>
    <h2>Prove what happened. Replay it.</h2>
    <p>
      Every run leaves a hash-chained trail; replay folds it back into the same
      world state — without calling a provider or re-running a tool.
    </p>
  </div>
  <div class="terminal reveal">
    <div class="terminal-bar">
      <span class="dot r"></span><span class="dot y"></span><span class="dot g"></span>
      <span class="title">thymos replay run_847</span>
    </div>
    <div class="terminal-body" data-speed="14" data-type='[
      {"text":"$ thymos replay run_847 --verify --fold-world --policy-trace","cls":"cmd","pause":240},
      {"text":"[load] entries=8 head=seq:7 kind:commit","cls":"muted","pause":180},
      {"text":"[integrity] hash_chain=ok parent_chain=ok sequence=ok","cls":"ok","pause":200},
      {"text":"[policy] proposal=prop_c4b1 decision=require_approval","cls":"out","pause":180},
      {"text":"[approval] proposal=prop_c4b1 approved=true","cls":"out","pause":180},
      {"text":"[fold] seq=5 commit=commit_f2e4b7 world=a9014cc2","cls":"ok","pause":180},
      {"text":"[report] commits_replayed=4 final_world_hash=a9014cc2e1d44ef8","cls":"hl","pause":180},
      {"text":"result: replay verified","cls":"ok","pause":0}
    ]'>
    </div>
  </div>
</section>

<section class="section" id="downloads">
  <div class="section-h reveal">
    <p class="eyebrow">Download stable release</p>
    <h2>One official download. Two ways to run it.</h2>
    <p>
      Everything ships from the
      <a href="https://github.com/gryszzz/open-thymos/releases/latest">latest stable release</a>
      — release notes and checksums there. Nightly dev builds are separate and
      clearly marked.
    </p>
  </div>

  <div class="dl-grid reveal">
    <div class="dl-card primary">
      <div class="dl-head">
        <span class="download-kicker">Desktop · recommended</span>
        <strong>OpenThymos Desktop</strong>
        <small>Chat, Mind graph, audit &amp; replay — bundles the local governed runtime.</small>
      </div>
      <div class="dl-buttons">
        <a class="btn btn-primary" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-desktop-macos-arm64.dmg"> macOS · Apple silicon</a>
        <a class="btn btn-primary" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-desktop-macos-x64.dmg"> macOS · Intel</a>
        <a class="btn btn-primary" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-desktop-windows-x64.msi">⊞ Windows x64</a>
        <a class="btn btn-primary" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-desktop-linux-x64.AppImage">🐧 Linux x64</a>
      </div>
      <small class="dl-note">Unsigned installers — your OS will ask once on first launch.</small>
    </div>

    <div class="dl-card" id="download-cli">
      <div class="dl-head">
        <span class="download-kicker">CLI + runtime · power users</span>
        <strong>thymos · terminal release</strong>
        <small>Contains <code>thymos</code> and <code>thymos-server</code>. Scriptable, server-ready.</small>
      </div>
      <div class="dl-buttons">
        <a class="btn btn-ghost" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-cli-runtime-macos-arm64.tar.gz"> macOS · Apple silicon</a>
        <a class="btn btn-ghost" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-cli-runtime-macos-x64.tar.gz"> macOS · Intel</a>
        <a class="btn btn-ghost" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-cli-runtime-windows-x64.tar.gz">⊞ Windows x64</a>
        <a class="btn btn-ghost" href="https://github.com/gryszzz/open-thymos/releases/latest/download/OpenThymos-cli-runtime-linux-x64.tar.gz">🐧 Linux x64</a>
      </div>
      <pre class="cli-line"><code>curl -fsSL https://raw.githubusercontent.com/gryszzz/open-thymos/main/scripts/get.sh | sh</code></pre>
    </div>
  </div>
</section>

<section class="section">
  <div class="section-h reveal">
    <p class="eyebrow">Execution grammar</p>
    <h2>Intent → Proposal → Commit</h2>
    <p>
      Three types. One direction. No shortcuts.
    </p>
  </div>

  <div class="cards reveal">
    <div class="fcard">
      <div class="icon mono">I</div>
      <h3>Intent</h3>
      <p>
        Emitted by cognition. Carries no authority. Content-addressed by
        <code>blake3(canonical_json(body))</code>.
      </p>
    </div>
    <div class="fcard">
      <div class="icon mono">P</div>
      <h3>Proposal</h3>
      <p>
        Compiled by the runtime from <code>(Intent, Writ, World, ToolRegistry, PolicyEngine)</code>.
        Binds tool contract, budget projection, and policy trace.
      </p>
    </div>
    <div class="fcard">
      <div class="icon mono">C</div>
      <h3>Commit</h3>
      <p>
        The only record that mutates world state. Contains structured delta,
        observed tool output, writ id, proposal id, and compiler version.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">⬢</div>
      <h3>Capability writ</h3>
      <p>
        Ed25519-signed authority document. Bounds tool scopes, budgets, time
        windows, effect ceilings, and delegation depth.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">⟐</div>
      <h3>Policy engine</h3>
      <p>
        Pure function <code>(Intent, Writ, World) → Permit | Deny | RequireApproval</code>.
        Decision recorded in the proposal trace.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">▣</div>
      <h3>Execution ledger</h3>
      <p>
        Append-only, parent-chained trajectory history. Root, commit, rejection,
        pending approval, delegation, and branch entries.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">⎈</div>
      <h3>Replay verifier</h3>
      <p>
        Proves hash chain integrity, sequence continuity, and world projection
        correctness. Cannot call providers or execute tools.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">◎</div>
      <h3>Provider boundary</h3>
      <p>
        Providers emit intents. They cannot execute, authorize, or delegate.
        Provider identity grants no authority.
      </p>
    </div>
  </div>
</section>

<section class="section">
  <div class="section-h reveal">
    <p class="eyebrow">Five runtime guarantees</p>
    <h2>Not conventions. Invariants.</h2>
    <p>
      These are checked structurally by the runtime, recorded in the ledger,
      and verifiable by replay. They are not documented promises.
    </p>
  </div>

  <div class="axioms reveal">
    <div class="axiom">
      <span class="axiom-n">I</span>
      <div>
        <strong>Deterministic replay</strong>
        <p>A valid ledger can be folded into the same world projection under the recorded commit sequence.</p>
      </div>
    </div>
    <div class="axiom">
      <span class="axiom-n">II</span>
      <div>
        <strong>Runtime isolation</strong>
        <p>Cognition cannot execute tools or mutate state directly. The provider boundary is enforced at the type level.</p>
      </div>
    </div>
    <div class="axiom">
      <span class="axiom-n">III</span>
      <div>
        <strong>Execution integrity</strong>
        <p>Only staged proposals may reach the tool boundary. Only commits may mutate projected world state.</p>
      </div>
    </div>
    <div class="axiom">
      <span class="axiom-n">IV</span>
      <div>
        <strong>Capability constraints</strong>
        <p>Tool scopes, budgets, time windows, effect ceilings, tenant boundaries, and delegation bounds are checked before execution.</p>
      </div>
    </div>
    <div class="axiom">
      <span class="axiom-n">V</span>
      <div>
        <strong>Policy persistence</strong>
        <p>Policy decisions are recorded as proposal traces and cannot be erased by a client surface.</p>
      </div>
    </div>
  </div>
</section>

<section class="section">
  <div class="cta-wrap reveal">
    <h2>Run it locally. Govern everything.</h2>
    <p>
      Download the desktop app or the CLI, connect a model, and watch every
      action pass through Intent → Proposal → Commit.
    </p>
    <div class="hero-cta">
      <a class="btn btn-primary" href="#downloads">
        ⬇ Download <span class="btn-arrow">→</span>
      </a>
      <a class="btn btn-ghost" href="{{ '/getting-started' | relative_url }}">
        Getting started
      </a>
      <a class="btn btn-ghost" href="{{ '/specification' | relative_url }}">
        Specification
      </a>
    </div>
  </div>
</section>
