"use client";

import { useEffect, useMemo, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import { RunViewer } from "@/components/trajectory/RunViewer";
import {
  createRun,
  getHealth,
  getReady,
  listRuns,
  type CognitionProvider,
  type RunListResponse,
  type RuntimeHealth,
  type RuntimeReady,
} from "@/lib/thymos-api";

const suggestedTasks = [
  {
    title: "Repository inspection",
    body: "Map the repo, explain the architecture, and call out the highest-value next steps.",
  },
  {
    title: "Runtime verification",
    body: "Trace the execution loop, run the right checks, and confirm the runtime is healthy.",
  },
  {
    title: "Product polish",
    body: "Tune the Thymos interfaces for clarity and brand consistency, then verify the affected surfaces.",
  },
];

const providerOptions: Array<{
  value: CognitionProvider;
  label: string;
  help: string;
  placeholder: string;
}> = [
  {
    value: "mock",
    label: "Mock",
    help: "Deterministic, no API key required.",
    placeholder: "No model needed",
  },
  {
    value: "openai",
    label: "OpenAI",
    help: "Use OPENAI_API_KEY or a compatible gateway.",
    placeholder: "gpt-4o",
  },
  {
    value: "local",
    label: "Local",
    help: "Ollama, vLLM, llama.cpp, or custom OpenAI-compatible endpoints.",
    placeholder: "llama3",
  },
  {
    value: "lmstudio",
    label: "LM Studio",
    help: "Local desktop model server.",
    placeholder: "qwen2.5-coder-32b-instruct",
  },
  {
    value: "huggingface",
    label: "Hugging Face",
    help: "Hosted router with HF_TOKEN.",
    placeholder: "Qwen/Qwen2.5-Coder-32B-Instruct",
  },
  {
    value: "anthropic",
    label: "Anthropic",
    help: "Optional hosted frontier provider.",
    placeholder: "opus",
  },
];

export default function RunsPage() {
  const router = useRouter();
  const [runId, setRunId] = useState<string | null>(null);
  const [task, setTask] = useState("");
  const [maxSteps, setMaxSteps] = useState(24);
  const [provider, setProvider] = useState<CognitionProvider>("mock");
  const [model, setModel] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [health, setHealth] = useState<RuntimeHealth | null>(null);
  const [ready, setReady] = useState<RuntimeReady | null>(null);
  const [recentRuns, setRecentRuns] = useState<RunListResponse["runs"]>([]);
  const [opsError, setOpsError] = useState<string | null>(null);
  const selectedProvider = providerOptions.find((option) => option.value === provider) ?? providerOptions[0];
  const taskPreview = useMemo(
    () => buildTaskPreview(task, provider, model, maxSteps),
    [task, provider, model, maxSteps],
  );

  useEffect(() => {
    if (typeof window === "undefined") return;
    setRunId(new URLSearchParams(window.location.search).get("id"));
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function refreshControlPlane() {
      try {
        const [healthSnapshot, readySnapshot, runsSnapshot] = await Promise.all([
          getHealth(),
          getReady(),
          listRuns(6),
        ]);

        if (!cancelled) {
          setHealth(healthSnapshot);
          setReady(readySnapshot);
          setRecentRuns(runsSnapshot.runs);
          setOpsError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setOpsError(err instanceof Error ? err.message : String(err));
        }
      }
    }

    void refreshControlPlane();
    const timer = setInterval(refreshControlPlane, 5000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!task.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const result = await createRun(task, maxSteps, {
        provider,
        ...(model.trim() ? { model: model.trim() } : {}),
      });
      router.push(`/runs?id=${encodeURIComponent(result.run_id)}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setLoading(false);
    }
  }

  if (runId) {
    return <RunViewer id={runId} />;
  }

  return (
    <main className="thymos-runtime-shell">
      <section className="thymos-console-panel thymos-runtime-hero">
        <div className="thymos-runtime-copy">
          <div className="thymos-eyebrow">Thymos Unified Runtime</div>
          <h1 className="thymos-runtime-title">
            Start one task. Watch the same execution session everywhere.
          </h1>
          <p className="thymos-runtime-summary">
            Thymos runs the full operator loop from intent to result against a shared backend
            runtime. CLI, sidebar, terminal, and web console all attach to the same live execution
            state, approvals, and completion record.
          </p>

          <div className="thymos-runtime-chip-grid">
            {[
              "Intent → Proposal → Execution → Result",
              "Shared live execution log",
              "Runtime-led recovery and retries",
              "Replayable world state and branching",
            ].map((item) => (
              <div className="thymos-runtime-chip" key={item}>
                {item}
              </div>
            ))}
          </div>

          <div className="thymos-console-banner">
            <strong>Operator Feed</strong>
            <p>
              Submit a task once, then watch the runtime plan, act, recover, and finish through
              the same observable loop every surface shares.
            </p>
            <div className="thymos-console-command">
              <span>$</span>
              <code>thymos run --follow "Inspect the runtime, verify the result, and report back"</code>
            </div>
          </div>

          <div className="thymos-control-grid" aria-label="Runtime control plane status">
            <ControlTile
              label="Runtime"
              value={health?.status ?? "offline"}
              detail={health ? `${health.mode} mode` : opsError ?? "Waiting for backend"}
              tone={health?.status === "ok" ? "good" : "warn"}
            />
            <ControlTile
              label="Readiness"
              value={ready?.status ?? "unknown"}
              detail={ready?.checks ? formatChecks(ready.checks) : "Health probes + stores"}
              tone={ready?.status === "ready" ? "good" : "warn"}
            />
            <ControlTile
              label="Recent Runs"
              value={String(recentRuns.length)}
              detail="Live cache from /runs"
              tone={recentRuns.length > 0 ? "signal" : "neutral"}
            />
          </div>
        </div>

        <form onSubmit={handleSubmit} className="thymos-console-panel-soft thymos-runtime-form">
          <label htmlFor="task" className="thymos-field">
            <span className="thymos-field-label">Operator Task</span>
            <textarea
              id="task"
              className="thymos-textarea"
              value={task}
              onChange={(event) => setTask(event.target.value)}
              placeholder="Refactor the TypeScript API client, run the relevant checks, and leave the runtime in a verified state."
              rows={7}
            />
          </label>

          <div className="thymos-preview-card" aria-live="polite">
            <div className="thymos-preview-head">
              <div>
                <span className="thymos-eyebrow">Live Execution Preview</span>
                <strong>{taskPreview.title}</strong>
              </div>
              <span className="thymos-preview-risk" data-risk={taskPreview.risk}>
                {taskPreview.risk}
              </span>
            </div>

            <p>{taskPreview.summary}</p>

            <div className="thymos-preview-route">
              {taskPreview.phases.map((phase, index) => (
                <span key={phase}>
                  <b>{String(index + 1).padStart(2, "0")}</b>
                  {phase}
                </span>
              ))}
            </div>

            <div className="thymos-preview-meta">
              <span>Provider: {selectedProvider.label}</span>
              <span>Suggested steps: {taskPreview.suggestedSteps}</span>
              <span>{task.trim().length} chars</span>
            </div>

            <div className="thymos-console-command thymos-preview-command">
              <span>$</span>
              <code>{taskPreview.command}</code>
            </div>

            {taskPreview.suggestedSteps !== maxSteps ? (
              <button
                type="button"
                className="thymos-preview-tune"
                onClick={() => setMaxSteps(taskPreview.suggestedSteps)}
              >
                Use suggested step budget
              </button>
            ) : null}
          </div>

          <div className="thymos-suggestion-row">
            {suggestedTasks.map((suggestion) => (
              <button
                key={suggestion.title}
                type="button"
                className="thymos-suggestion"
                onClick={() => setTask(suggestion.body)}
              >
                <strong>{suggestion.title}</strong>
                <span>{suggestion.body}</span>
              </button>
            ))}
          </div>

          <div className="thymos-form-row">
            <label className="thymos-field thymos-provider-field">
              <span className="thymos-field-label">Provider</span>
              <select
                className="thymos-select"
                value={provider}
                onChange={(event) => {
                  setProvider(event.target.value as CognitionProvider);
                  setModel("");
                }}
              >
                {providerOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>

            <label className="thymos-field thymos-model-field">
              <span className="thymos-field-label">Model</span>
              <input
                className="thymos-text-input"
                type="text"
                value={model}
                disabled={provider === "mock"}
                placeholder={selectedProvider.placeholder}
                onChange={(event) => setModel(event.target.value)}
              />
            </label>

            <label className="thymos-field">
              <span className="thymos-field-label">Max Steps</span>
              <input
                className="thymos-number-input"
                type="number"
                value={maxSteps}
                min={1}
                max={100}
                onChange={(event) => setMaxSteps(Number(event.target.value))}
              />
            </label>

            <button type="submit" disabled={loading || !task.trim()} className="thymos-button">
              {loading ? "Launching Runtime" : "Start Thymos Run"}
            </button>
          </div>

          <p className="thymos-provider-note">{selectedProvider.help}</p>

          {error ? <p className="thymos-error">{error}</p> : null}
        </form>
      </section>

      <section className="thymos-console-panel thymos-run-history-panel">
        <div className="thymos-history-head">
          <div>
            <span className="thymos-eyebrow">Run Memory</span>
            <h2>Recent execution sessions</h2>
          </div>
          <p>Jump back into live or completed runtime state without hunting through terminal logs.</p>
        </div>

        {opsError ? (
          <div className="thymos-empty-state">
            <strong>Runtime Offline</strong>
            <p>{opsError}</p>
          </div>
        ) : recentRuns.length > 0 ? (
          <div className="thymos-run-history-list">
            {recentRuns.map((run) => (
              <button
                type="button"
                className="thymos-run-history-item"
                key={run.run_id}
                onClick={() => router.push(`/runs?id=${encodeURIComponent(run.run_id)}`)}
              >
                <span className="thymos-run-status" data-status={run.status}>
                  {run.status}
                </span>
                <strong>{run.task}</strong>
                <code>{run.run_id.slice(0, 12)}</code>
              </button>
            ))}
          </div>
        ) : (
          <div className="thymos-empty-state">
            <strong>No Runs Yet</strong>
            <p>Start a mock run and this panel becomes your local runtime history.</p>
          </div>
        )}
      </section>
    </main>
  );
}

function formatChecks(checks: Record<string, boolean>) {
  return Object.entries(checks)
    .map(([name, ok]) => `${name.replace(/_/g, " ")}: ${ok ? "ok" : "wait"}`)
    .join(" · ");
}

function ControlTile({
  label,
  value,
  detail,
  tone,
}: {
  label: string;
  value: string;
  detail: string;
  tone: "good" | "warn" | "signal" | "neutral";
}) {
  return (
    <div className="thymos-control-tile" data-tone={tone}>
      <span>{label}</span>
      <strong>{value}</strong>
      <p>{detail}</p>
    </div>
  );
}

function buildTaskPreview(
  task: string,
  provider: CognitionProvider,
  model: string,
  maxSteps: number,
) {
  const normalized = task.trim().toLowerCase();
  const hasTask = normalized.length > 0;
  const isCodeChange = /\b(refactor|fix|implement|add|update|change|edit|patch|build)\b/.test(normalized);
  const isVerification = /\b(test|verify|check|lint|typecheck|audit)\b/.test(normalized);
  const isExploration = /\b(inspect|map|explain|summarize|analyze|review)\b/.test(normalized);
  const isRisky = /\b(delete|remove|deploy|release|push|production|secret|token|billing|payment)\b/.test(normalized);

  const suggestedSteps = isRisky
    ? 36
    : isCodeChange && isVerification
      ? 30
      : isCodeChange
        ? 24
        : isExploration
          ? 14
          : 16;

  const title = !hasTask
    ? "Type a task to preview the runtime path"
    : isRisky
      ? "Guarded execution with approval posture"
      : isCodeChange
        ? "Code execution route"
        : isVerification
          ? "Verification route"
          : isExploration
            ? "Inspection route"
            : "General runtime route";

  const risk = !hasTask ? "idle" : isRisky ? "high" : isCodeChange ? "medium" : "low";
  const phases = [
    "Intent",
    isRisky ? "Policy + approval" : "Policy check",
    isVerification ? "Test run" : isCodeChange ? "Patch proposal" : "Tool plan",
    "Ledger result",
  ];
  const providerArg = provider === "mock" ? "--provider mock" : `--provider ${provider}`;
  const modelArg = model.trim() ? ` --model ${shellQuote(model.trim())}` : "";
  const prompt = hasTask ? task.trim() : "Describe the task you want Thymos to run";

  return {
    title,
    risk,
    suggestedSteps,
    phases,
    summary: hasTask
      ? "Thymos will turn this into typed intents, check policy and budget, execute through allowed tools, then stream the result into the ledger."
      : "The preview updates live as you type so operators can see the likely path before launching a run.",
    command: `thymos run ${providerArg}${modelArg} --max-steps ${Math.max(1, Math.trunc(maxSteps || suggestedSteps))} ${shellQuote(prompt)}`,
  };
}

function shellQuote(value: string) {
  return `"${value.replace(/(["\\$`])/g, "\\$1")}"`;
}
