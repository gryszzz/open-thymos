---
layout: default
title: OpenThymos
hide_title: true
hero: true
permalink: /
---

<section class="hero">
  <div class="hero-eyebrow">
    <span class="dot"></span>
    OPENTHYMOS · GOVERNED EXECUTION RUNTIME
  </div>

  <h1>
    Cognition proposes.<br>
    The runtime governs.<br>
    The ledger records.
  </h1>

  <p class="lede">
    A model cannot call a tool, mutate state, spend budget, delegate authority,
    or erase history — not by convention, by runtime semantics. Every effect
    passes through a typed proposal, a signed capability writ, a policy trace,
    and an append-only execution ledger.
  </p>

  <div class="hero-cta">
    <a class="btn btn-primary" href="{{ '/specification' | relative_url }}">
      Read the specification <span class="btn-arrow">→</span>
    </a>
    <a class="btn btn-ghost" href="{{ '/replay' | relative_url }}">
      Replay verifier
    </a>
    <a class="btn btn-ghost" href="{{ '/capability-writs' | relative_url }}">
      Capability writs
    </a>
    <a class="btn btn-ghost" href="{{ '/architecture' | relative_url }}">
      Architecture
    </a>
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

<section class="section">
  <div class="cta-wrap reveal">
    <h2>Begin with the protocol.</h2>
    <p>
      Read the specification, then the architecture, then the replay and writ
      documents. The implementation exists to preserve these runtime semantics.
    </p>
    <div class="hero-cta">
      <a class="btn btn-primary" href="{{ '/specification' | relative_url }}">
        Specification <span class="btn-arrow">→</span>
      </a>
      <a class="btn btn-ghost" href="{{ '/runtime-invariants' | relative_url }}">
        Runtime invariants
      </a>
      <a class="btn btn-ghost" href="{{ '/architecture' | relative_url }}">
        Architecture
      </a>
    </div>
  </div>
</section>
