import { ThymosLogo } from "@/components/branding/ThymosLogo";
import { siteConfig } from "@/lib/site";

const proofItems = [
  "Rust runtime kernel",
  "Programmable capabilities",
  "Sandboxed tool fabric",
  "Signed authority",
  "Unified run surfaces",
  "Replayable trajectories",
];

const runtimeNotes = [
  {
    label: "Execution kernel",
    title: "Models do not own effects.",
    body: "OpenThymos treats cognition as a bounded proposer. The Rust runtime, policy engine, sandbox, and ledger own the decision surface.",
  },
  {
    label: "Framework",
    title: "Capabilities are programmable.",
    body: "Built-in Rust contracts, JSON manifests, and MCP bridges all register as governed capabilities with schemas, effect classes, and writ scopes.",
  },
  {
    label: "Surfaces",
    title: "Every client sees the same run.",
    body: "CLI, VS Code, terminal shell, and web console attach to one backend execution session instead of spawning separate agents.",
  },
];

const mechanismStages = [
  {
    step: "01",
    title: "Intent",
    body: "The model declares a typed action request with rationale. No side effects happen here.",
  },
  {
    step: "02",
    title: "Proposal",
    body: "Compiler, policy, and writ checks resolve authority, budget, and risk before execution is staged.",
  },
  {
    step: "03",
    title: "Commit",
    body: "The runtime executes the tool contract, verifies the result, and appends the signed outcome to the ledger.",
  },
];

const pillarCards = [
  {
    label: "Bounded authority",
    title: "Ed25519 writs define what the agent may do.",
    body: "Authority is explicit, scoped, signed, and time-bound instead of implied by the prompt.",
  },
  {
    label: "Unified surfaces",
    title: "CLI, VS Code, terminal, and web share one runtime.",
    body: "Every surface presents the same run id, phase, approvals, logs, world projection, and final outcome.",
  },
  {
    label: "Programmable capabilities",
    title: "Capabilities are contracts, not free-form tool blobs.",
    body: "Rust tools, manifest tools, and MCP bridge tools declare schemas, effects, risk, and observations before they can execute.",
  },
  {
    label: "Multi-provider cognition",
    title: "Hosted or local models plug into the same governed loop.",
    body: "Anthropic, OpenAI, Hugging Face, LM Studio, local OpenAI-compatible endpoints, and mock runs share one control plane.",
  },
  {
    label: "Sandboxed tool fabric",
    title: "Risky execution crosses a worker boundary.",
    body: "Shell, HTTP, and coding actions run through profile-gated sandbox paths with receipts and allowed roots.",
  },
  {
    label: "Replayable trajectories",
    title: "You can inspect what happened, not guess.",
    body: "Runs stream live and can be replayed later to reconstruct the same trajectory and world projection.",
  },
];

const codingTools = [
  ["repo_map", "Read", "top-level layout and workspace markers"],
  ["list_files", "Read", "bounded directory walk"],
  ["fs_read", "Read", "file reads with byte and line limits"],
  ["grep", "Read", "substring scan with filters"],
  ["fs_patch", "Write", "typed replace or overwrite patch mode"],
  ["test_run", "External", "suite execution with auto-detected toolchain"],
];

const providerGroups = [
  {
    label: "Local sovereignty",
    title: "LM Studio, Ollama, vLLM, and OpenAI-compatible local endpoints.",
    body: "Keep sensitive repositories on your machine while still running the same typed, ledgered execution pipeline.",
    chips: ["LM Studio", "OpenAI-compatible local", "Ollama", "vLLM"],
  },
  {
    label: "Hosted cognition",
    title: "Anthropic, OpenAI, Hugging Face, and mock runs.",
    body: "Swap cognition providers without rewriting the runtime contract. Policy, tools, and history stay consistent.",
    chips: ["Anthropic", "OpenAI", "Hugging Face", "Mock"],
  },
];

const workflowColumns = [
  {
    title: "Developer loop",
    items: [
      "Launch work from the CLI, shell, VS Code, or web console.",
      "Inspect the repo with typed read tools.",
      "Propose targeted file patches instead of opaque diffs.",
      "Run tests through the same governed execution path.",
      "Review the full trajectory from any attached surface.",
    ],
  },
  {
    title: "Operator control",
    items: [
      "Issue writs with bounded scopes and budgets.",
      "Apply policy gates before execution commits.",
      "Route shell and HTTP through the secure worker fabric.",
      "Replay or audit every run from the ledger.",
    ],
  },
];

const launchNotes = [
  "Pre-alpha open runtime",
  "Rust execution kernel",
  "Programmable capability layer",
  "Unified CLI / VS Code / web state",
];

const packageCards = [
  {
    label: "Canonical image",
    title: "ghcr.io/gryszzz/openthymos-runtime",
    body: "Release tags publish the runtime server and CLI as a traceable OCI artifact.",
  },
  {
    label: "Compatibility alias",
    title: "ghcr.io/gryszzz/thymos-server",
    body: "Existing scripts keep working while the public package name moves to OpenThymos.",
  },
  {
    label: "Immutable pull",
    title: "Prefer semver or sha tags",
    body: "Production automation should pin a release or source revision, not the moving latest tag.",
  },
];

export function ThymosLanding() {
  return (
    <main className="thymos-page">
      <div className="thymos-grid-haze" aria-hidden="true" />

      <div className="thymos-shell">
        <header className="thymos-header thymos-reveal">
          <ThymosLogo priority />

          <nav className="thymos-nav" aria-label="Primary">
            <a href="#runtime">Runtime</a>
            <a href="#mechanism">Protocol</a>
            <a href="#coding-agent">Capabilities</a>
            <a href="#packages">Packages</a>
            <a href="#backends">Backends</a>
          </nav>

          <a
            className="thymos-header-cta"
            href={siteConfig.readmeUrl}
            target="_blank"
            rel="noreferrer"
          >
            Get Started
          </a>
        </header>

        <section className="thymos-hero" id="top">
          <div className="thymos-hero-copy">
            <h1 className="thymos-reveal thymos-delay-2">{siteConfig.headline}</h1>
            <p className="thymos-hero-lede thymos-reveal thymos-delay-3">{siteConfig.tagline}</p>
            <p className="thymos-hero-subcopy thymos-reveal thymos-delay-4">
              {siteConfig.subheadline}
            </p>

            <div className="thymos-hero-actions thymos-reveal thymos-delay-5">
              <a className="thymos-primary-action" href={`${siteConfig.basePath}/runs/`}>
                Open the runtime
              </a>
              <a
                className="thymos-secondary-action"
                href={siteConfig.readmeUrl}
                target="_blank"
                rel="noreferrer"
              >
                Read the README
              </a>
              <a
                className="thymos-secondary-action"
                href={siteConfig.wikiUrl}
                target="_blank"
                rel="noreferrer"
              >
                Open the wiki
              </a>
            </div>

            <div className="thymos-download thymos-reveal thymos-delay-5">
              <a
                className="thymos-download-primary"
                href={siteConfig.releasesUrl}
                target="_blank"
                rel="noreferrer"
              >
                ⬇ Download OpenThymos
              </a>
              <div className="thymos-download-plats">
                {["macOS", "Linux", "Windows"].map((plat) => (
                  <a
                    key={plat}
                    className="thymos-plat"
                    href={siteConfig.releasesUrl}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {plat}
                  </a>
                ))}
              </div>
              <p className="thymos-download-note">
                CLI + runtime for every platform — no Rust, no compile. One line:{" "}
                <code>curl -fsSL …/scripts/get.sh | sh</code>. The desktop app
                (chat, Mind view, audit) builds from source today; signed{" "}
                <code>.dmg</code> / <code>.msi</code> / <code>.AppImage</code> ship
                with the next release.
              </p>
            </div>

            <div className="thymos-launch-notes thymos-reveal thymos-delay-6">
              {launchNotes.map((item) => (
                <span className="thymos-launch-chip" key={item}>
                  {item}
                </span>
              ))}
            </div>
          </div>

          <div className="thymos-hero-visual thymos-reveal thymos-delay-3" aria-hidden="true">
            <div className="thymos-visual-stack">
              <div className="thymos-runtime-stage">
                <div className="thymos-stage-topline">
                  <span>open thymos / unified runtime</span>
                  <span>trajectory active</span>
                </div>

                <div className="thymos-stage-graph">
                  <article className="thymos-stage-node">
                    <span className="thymos-stage-label">Intent</span>
                    <strong>inspect crates/thymos-ledger</strong>
                    <p>Model proposes typed coding action.</p>
                  </article>
                  <span className="thymos-stage-link" />
                  <article className="thymos-stage-node">
                    <span className="thymos-stage-label">Policy</span>
                    <strong>allow under coding.writ.local</strong>
                    <p>Scope, budget, and path checks succeed.</p>
                  </article>
                  <span className="thymos-stage-link" />
                  <article className="thymos-stage-node">
                    <span className="thymos-stage-label">Commit</span>
                    <strong>ledger entry #000184</strong>
                    <p>Observation signed and appended.</p>
                  </article>
                </div>

                <div className="thymos-stage-console">
                  <div className="thymos-console-line">
                    <span>provider</span>
                    <strong>lmstudio / qwen2.5-coder</strong>
                  </div>
                  <div className="thymos-console-line">
                    <span>tool</span>
                    <strong>repo_map -&gt; fs_read -&gt; grep -&gt; test_run</strong>
                  </div>
                  <div className="thymos-console-line">
                    <span>receipt</span>
                    <strong>sandbox worker receipt emitted</strong>
                  </div>
                </div>
              </div>

              <article className="thymos-sidecard thymos-sidecard-a">
                <span className="thymos-sidecard-label">Signed writ</span>
                <strong>tool scope</strong>
                <p>repo_map, fs_read, grep, fs_patch, test_run</p>
              </article>

              <article className="thymos-sidecard thymos-sidecard-b">
                <span className="thymos-sidecard-label">Policy trace</span>
                <strong>allow / deny / suspend</strong>
                <p>Decision state is first-class data, not a missing log line.</p>
              </article>
            </div>
          </div>
        </section>

        <section className="thymos-proof-strip thymos-reveal thymos-delay-4">
          {proofItems.map((item) => (
            <span className="thymos-proof-chip" key={item}>
              {item}
            </span>
          ))}
        </section>

        <section className="thymos-section" id="runtime">
          <div className="thymos-section-head">
            <span className="thymos-kicker">What OpenThymos is</span>
            <h2>One Rust runtime for coding-agent execution.</h2>
            <p>
              OpenThymos is a model-agnostic execution framework where cognition proposes,
              programmable capabilities cross governed execution boundaries, and every surface
              observes the same ledgered run.
            </p>
          </div>

          <div className="thymos-runtime-grid">
            <article className="thymos-story-card">
              <span className="thymos-card-label">Runtime thesis</span>
              <h3>Agent work stays under runtime control.</h3>
              <p>
                The category is unified AI execution. Typed intents, signed writs, policy gates,
                sandbox receipts, and commit-time verification give OpenThymos the properties
                serious builders expect from infrastructure, not promptware.
              </p>
              <div className="thymos-story-metrics">
                <div>
                  <span>governance</span>
                  <strong>signed authority</strong>
                </div>
                <div>
                  <span>effects</span>
                  <strong>programmable capabilities</strong>
                </div>
                <div>
                  <span>surfaces</span>
                  <strong>CLI / VS Code / web</strong>
                </div>
              </div>
            </article>

            <div className="thymos-runtime-notes">
              {runtimeNotes.map((item) => (
                <article className="thymos-runtime-note" key={item.title}>
                  <span className="thymos-card-label">{item.label}</span>
                  <h3>{item.title}</h3>
                  <p>{item.body}</p>
                </article>
              ))}
            </div>
          </div>
        </section>

        <section className="thymos-section" id="mechanism">
          <div className="thymos-section-head compact">
            <span className="thymos-kicker">How it works</span>
            <h2>Intent -&gt; Proposal -&gt; Commit.</h2>
            <p>
              The core mechanism is fast to grasp because the runtime shape stays stable. Cognition
              emits intent. The system resolves authority and policy. Approved work becomes a
              durable commit.
            </p>
          </div>

          <div className="thymos-mechanism-shell">
            <div className="thymos-mechanism-rail" aria-hidden="true" />
            <div className="thymos-mechanism-grid">
              {mechanismStages.map((stage) => (
                <article className="thymos-mechanism-card" key={stage.step}>
                  <span className="thymos-card-label">{stage.step}</span>
                  <h3>{stage.title}</h3>
                  <p>{stage.body}</p>
                </article>
              ))}
            </div>

            <div className="thymos-mechanism-ledger">
              <span className="thymos-card-label">Ledger outcome</span>
              <strong>Allowed, denied, and suspended steps all survive as trajectory history.</strong>
            </div>
          </div>
        </section>

        <section className="thymos-section" id="pillars">
          <div className="thymos-section-head compact">
            <span className="thymos-kicker">Feature pillars</span>
            <h2>Built like runtime infrastructure, not agent theater.</h2>
            <p>
              The product surface is sharp because the internals are opinionated: bounded
              authority, typed execution, durable history, and provider portability.
            </p>
          </div>

          <div className="thymos-pillar-grid">
            {pillarCards.map((pillar) => (
              <article className="thymos-pillar-card" key={pillar.title}>
                <span className="thymos-card-label">{pillar.label}</span>
                <h3>{pillar.title}</h3>
                <p>{pillar.body}</p>
              </article>
            ))}
          </div>
        </section>

        <section className="thymos-section" id="onboarding">
          <div className="thymos-section-head compact">
            <span className="thymos-kicker">Onboarding paths</span>
            <h2>Start from the surface that matches how you work.</h2>
            <p>
              The CLI, VS Code sidebar, interactive terminal shell, and web console all talk to the
              same runtime. Pick your entry point and the execution state stays consistent
              everywhere.
            </p>
          </div>

          <div className="thymos-workflow-grid">
            <article className="thymos-workflow-card">
              <h3>Browser runtime</h3>
              <ul>
                <li>Open the unified run console and submit a task.</li>
                <li>Watch intent, proposal, execution, and result update live.</li>
                <li>Review the execution log, world state, and replay controls in one place.</li>
              </ul>
            </article>
            <article className="thymos-workflow-card">
              <h3>CLI and terminal</h3>
              <ul>
                <li>Run `thymos shell` or create runs directly from the CLI.</li>
                <li>See the same execution session the browser and sidebar show.</li>
                <li>Use the README quickstart when you want the fastest local setup path.</li>
              </ul>
            </article>
            <article className="thymos-workflow-card">
              <h3>Docs and wiki</h3>
              <ul>
                <li>Use the docs for architecture, interfaces, and API references.</li>
                <li>Use the wiki source as the operator-facing knowledge base.</li>
                <li>Keep onboarding and architecture guidance aligned with the runtime.</li>
              </ul>
            </article>
          </div>
        </section>

        <section className="thymos-section" id="coding-agent">
          <div className="thymos-section-head">
            <span className="thymos-kicker">Coding-agent surface</span>
            <h2>Programmable capabilities inside a coding sandbox.</h2>
            <p>
              The first high-value OpenThymos workload is coding work: inspect a repository, read
              files, patch code, run tests, and extend the capability set without changing the run
              semantics.
            </p>
          </div>

          <div className="thymos-coding-grid">
            <article className="thymos-coding-panel">
              <div className="thymos-coding-head">
                <div>
                  <span className="thymos-card-label">Typed tools</span>
                  <h3>Real repo actions, not vague tool JSON.</h3>
                </div>
                <span className="thymos-coding-badge">path-confined</span>
              </div>

              <div className="thymos-tool-table" role="table" aria-label="Coding tools">
                {codingTools.map(([tool, effect, summary]) => (
                  <div className="thymos-tool-row" role="row" key={tool}>
                    <strong role="cell">{tool}</strong>
                    <span role="cell">{effect}</span>
                    <p role="cell">{summary}</p>
                  </div>
                ))}
              </div>
            </article>

            <article className="thymos-trajectory-panel">
              <span className="thymos-card-label">Run trace</span>
              <h3>Each edit and test becomes governed history.</h3>

              <div className="thymos-trace-list">
                <div className="thymos-trace-item">
                  <span>01</span>
                  <div>
                    <strong>repo_map</strong>
                    <p>workspace discovered, crate graph loaded</p>
                  </div>
                </div>
                <div className="thymos-trace-item">
                  <span>02</span>
                  <div>
                    <strong>fs_read</strong>
                    <p>policy allowed read inside scoped root</p>
                  </div>
                </div>
                <div className="thymos-trace-item">
                  <span>03</span>
                  <div>
                    <strong>fs_patch</strong>
                    <p>unique-anchor replace committed to trajectory</p>
                  </div>
                </div>
                <div className="thymos-trace-item">
                  <span>04</span>
                  <div>
                    <strong>test_run</strong>
                    <p>suite executed, observation attached to commit</p>
                  </div>
                </div>
              </div>
            </article>
          </div>
        </section>

        <section className="thymos-section" id="backends">
          <div className="thymos-section-head compact">
            <span className="thymos-kicker">Local + hosted backends</span>
            <h2>Use any model. Keep execution under your control.</h2>
            <p>
              Cognition is pluggable. Governance is not. OpenThymos keeps the runtime contract
              stable whether you point it at a local endpoint or a hosted provider.
            </p>
          </div>

          <div className="thymos-backend-grid">
            {providerGroups.map((group) => (
              <article className="thymos-backend-card" key={group.title}>
                <span className="thymos-card-label">{group.label}</span>
                <h3>{group.title}</h3>
                <p>{group.body}</p>
                <div className="thymos-provider-chips">
                  {group.chips.map((chip) => (
                    <span key={chip}>{chip}</span>
                  ))}
                </div>
              </article>
            ))}
          </div>
        </section>

        <section className="thymos-section" id="packages">
          <div className="thymos-section-head compact">
            <span className="thymos-kicker">Package distribution</span>
            <h2>Release artifacts should be boring, traceable, and pinned.</h2>
            <p>
              The release workflow publishes GitHub Packages container images and platform
              binaries from the same source revision. Manual dispatches produce branch and SHA
              images for staging.
            </p>
          </div>

          <div className="thymos-package-grid">
            {packageCards.map((item) => (
              <article className="thymos-package-card" key={item.title}>
                <span className="thymos-card-label">{item.label}</span>
                <h3>{item.title}</h3>
                <p>{item.body}</p>
              </article>
            ))}
          </div>

          <div className="thymos-command-card thymos-package-command">
            <span className="thymos-card-label">GitHub Packages</span>
            <div className="thymos-command-line">
              <span>$</span>
              <code>docker pull ghcr.io/gryszzz/openthymos-runtime:&lt;tag&gt;</code>
            </div>
            <div className="thymos-command-line">
              <span>$</span>
              <code>docker run --rm -p 3001:3001 ghcr.io/gryszzz/openthymos-runtime:&lt;tag&gt;</code>
            </div>
          </div>
        </section>

        <section className="thymos-section" id="workflow">
          <div className="thymos-section-head compact">
            <span className="thymos-kicker">Developer / operator workflow</span>
            <h2>Fast enough for builders. Controlled enough for operators.</h2>
            <p>
              The same runtime supports day-to-day coding work and higher-assurance operating
              modes. That is the point of making governance a first-class system primitive.
            </p>
          </div>

          <div className="thymos-workflow-grid">
            {workflowColumns.map((column) => (
              <article className="thymos-workflow-card" key={column.title}>
                <h3>{column.title}</h3>
                <ul>
                  {column.items.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </article>
            ))}
          </div>
        </section>

        <section className="thymos-cta-shell thymos-section">
          <div className="thymos-cta-copy">
            <span className="thymos-kicker">Deployable coding-agent runtime</span>
            <h2>OpenThymos turns model output into sandboxed execution.</h2>
            <p>
              Run the live console, inspect the architecture, or add a capability manifest. The
              point is the same in every path: coding-agent execution stays bounded, programmable,
              and replayable.
            </p>

            <div className="thymos-hero-actions">
              <a
                className="thymos-primary-action"
                href={siteConfig.readmeUrl}
                target="_blank"
                rel="noreferrer"
              >
                Read the README
              </a>
              <a
                className="thymos-secondary-action"
                href={siteConfig.packageDocsUrl}
                target="_blank"
                rel="noreferrer"
              >
                Package protocol
              </a>
            </div>
          </div>

          <div className="thymos-command-card">
            <span className="thymos-card-label">Get started</span>
            <div className="thymos-command-line">
              <span>$</span>
              <code>cargo run -p thymos-server</code>
            </div>
            <div className="thymos-command-line">
              <span>$</span>
              <code>npm run dev</code>
            </div>
            <div className="thymos-command-line">
              <span>$</span>
              <code>open /runs</code>
            </div>
          </div>
        </section>

        <footer className="thymos-footer">
          <ThymosLogo />
          <div className="thymos-footer-copy">
            <strong>OpenThymos is a unified governed execution runtime for coding agents.</strong>
            <span>
              An {siteConfig.org} project · Apache-2.0 ·{" "}
              <a href={siteConfig.issuesUrl} target="_blank" rel="noreferrer">
                GitHub Issues
              </a>
            </span>
          </div>
        </footer>
      </div>
    </main>
  );
}
