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
    OPENTHYMOS · GOVERNED COGNITION RUNTIME
  </div>

  <h1>
    Execution substrate for governed machine cognition.
  </h1>

  <p class="lede">
    OpenThymos converts untrusted cognition into auditable execution. Agents do
    not act autonomously; they emit intents. The runtime compiles proposals,
    enforces capability writs, applies policy, records commits, and replays the
    ledger into deterministic runtime state.
  </p>

  <div class="hero-cta">
    <a class="btn btn-primary" href="{{ '/specification' | relative_url }}">
      Read the specification <span class="btn-arrow">→</span>
    </a>
    <a class="btn btn-ghost" href="{{ '/replay' | relative_url }}">
      Study replay
    </a>
    <a class="btn btn-ghost" href="{{ '/package-distribution' | relative_url }}">
      Pull package
    </a>
  </div>

  <div class="hero-meta">
    <span class="mono">Intent -> Proposal -> Commit</span>
    <span>·</span>
    <span>Capability writs</span>
    <span>·</span>
    <span>Execution ledger</span>
  </div>
</section>

<section class="section">
  <div class="triad reveal">
    <div class="card">
      <h4>Governed</h4>
      <p>Authority is explicit.</p>
      <p class="sub">
        Signed writs define tool scope, budget, tenant boundary, effect
        ceiling, time window, and delegation bounds.
      </p>
    </div>
    <div class="card">
      <h4>Replayable</h4>
      <p>History is structured.</p>
      <p class="sub">
        Commits contain observations and deltas. Replay verifies the ledger and
        folds state without fresh model or tool calls.
      </p>
    </div>
    <div class="card">
      <h4>Deterministic</h4>
      <p>State is a fold.</p>
      <p class="sub">
        Runtime state is projected from content-addressed ledger entries, not
        reconstructed from chat transcript memory.
      </p>
    </div>
  </div>
</section>

<section class="section">
  <div class="section-h reveal">
    <p class="eyebrow">Runtime model</p>
    <h2>Cognition proposes. Runtime commits.</h2>
    <p>
      A provider may be stochastic, local, hosted, or mock. It enters the
      system through one narrow contract: produce intents. Everything after
      that point is governed by compiler order, writ validation, tool
      contracts, policy traces, and ledger semantics.
    </p>
  </div>

  <div class="cards reveal">
    <div class="fcard">
      <div class="icon">◎</div>
      <h3>Specification</h3>
      <p>
        Normative terms and execution semantics for the runtime protocol.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">⎇</div>
      <h3>Replay</h3>
      <p>
        Verification over ledger entries, parent chains, sequences, deltas,
        and compiler versions.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">⬢</div>
      <h3>Capability writs</h3>
      <p>
        Signed authority documents that bound what a cognitive subject may
        propose and delegate.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">⟐</div>
      <h3>Policy engine</h3>
      <p>
        Ordered pure rules that permit, deny, or require approval before
        execution reaches tools.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">▣</div>
      <h3>Execution ledger</h3>
      <p>
        Append-only, content-addressed trajectory history for audit and
        replay.
      </p>
    </div>
    <div class="fcard">
      <div class="icon">⎈</div>
      <h3>Provider boundary</h3>
      <p>
        Hosted and local providers can propose intents without changing
        runtime authority.
      </p>
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
      documents. The implementation exists to preserve these runtime
      semantics.
    </p>
    <div class="hero-cta">
      <a class="btn btn-primary" href="{{ '/architecture' | relative_url }}">
        Architecture <span class="btn-arrow">→</span>
      </a>
      <a class="btn btn-ghost" href="{{ '/runtime-invariants' | relative_url }}">
        Runtime invariants
      </a>
    </div>
  </div>
</section>
