"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  branchFrom,
  cancelRun,
  decideApproval,
  getExecution,
  getWorld,
  getWorldAt,
  subscribeEntries,
  resumeRun,
  subscribeExecution,
  subscribeStream,
  type CognitionEvent,
  type EntryDto,
  type ExecutionSession,
  type ResourceDto,
  type StreamConnectionState,
} from "@/lib/thymos-api";
import { EntryTimeline } from "@/components/trajectory/EntryTimeline";
import { ExecutionLog } from "@/components/trajectory/ExecutionLog";
import { StreamView } from "@/components/trajectory/StreamView";
import { WorldView } from "@/components/trajectory/WorldView";

type ConsoleTab = "execution" | "ledger" | "stream" | "world";

const statusStyles: Record<
  ExecutionSession["status"],
  { tone: string; glow: string; label: string }
> = {
  running: { tone: "#7dd3fc", glow: "rgba(125, 211, 252, 0.38)", label: "Live" },
  waiting_approval: { tone: "#fbbf24", glow: "rgba(251, 191, 36, 0.34)", label: "Awaiting approval" },
  completed: { tone: "#34d399", glow: "rgba(52, 211, 153, 0.34)", label: "Resolved" },
  failed: { tone: "#f87171", glow: "rgba(248, 113, 113, 0.34)", label: "Failed" },
  cancelled: { tone: "#94a3b8", glow: "rgba(148, 163, 184, 0.28)", label: "Cancelled" },
};

const tabLabels: Record<ConsoleTab, string> = {
  execution: "Execution Log",
  ledger: "Ledger Spine",
  stream: "Model Stream",
  world: "World State",
};

const terminalStatuses = new Set<ExecutionSession["status"]>(["completed", "failed", "cancelled"]);

function sessionRevision(session: ExecutionSession) {
  return [
    session.updated_at_ms,
    session.status,
    session.phase,
    session.current_step,
    session.active_tool ?? "",
    session.final_answer ?? "",
    session.log.length,
  ].join(":");
}

export function RunViewer({ id }: { id: string }) {
  const [session, setSession] = useState<ExecutionSession | null>(null);
  const [entries, setEntries] = useState<EntryDto[]>([]);
  const [streamEvents, setStreamEvents] = useState<CognitionEvent[]>([]);
  const [resources, setResources] = useState<ResourceDto[]>([]);
  const [tab, setTab] = useState<ConsoleTab>("execution");
  const [scrubSeq, setScrubSeq] = useState<number | null>(null);
  const [branchMsg, setBranchMsg] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<string | null>(null);
  const [actionBusy, setActionBusy] = useState<string | null>(null);
  const [executionLink, setExecutionLink] = useState<StreamConnectionState>("connecting");
  const [entryLink, setEntryLink] = useState<StreamConnectionState>("connecting");
  const [streamLink, setStreamLink] = useState<StreamConnectionState>("connecting");
  const [lastSnapshotAt, setLastSnapshotAt] = useState<number | null>(null);
  const [lastEntryAt, setLastEntryAt] = useState<number | null>(null);
  const sessionRevisionRef = useRef<string | null>(null);

  const status = session?.status ?? "running";
  const isTerminal = session ? terminalStatuses.has(session.status) : false;

  const acceptSnapshot = useCallback((snapshot: ExecutionSession) => {
    const revision = sessionRevision(snapshot);
    if (sessionRevisionRef.current === revision) return;
    sessionRevisionRef.current = revision;
    setSession(snapshot);
    setLastSnapshotAt(Date.now());
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | undefined;

    async function refreshSnapshot() {
      try {
        const snapshot = await getExecution(id);
        if (!cancelled) {
          acceptSnapshot(snapshot);
          if (terminalStatuses.has(snapshot.status) && timer) {
            clearInterval(timer);
            timer = undefined;
          }
        }
      } catch {
        if (!cancelled) setExecutionLink("reconnecting");
      }
    }

    void refreshSnapshot();
    timer = setInterval(() => {
      if (!cancelled) void refreshSnapshot();
    }, 3000);

    return () => {
      cancelled = true;
      if (timer) clearInterval(timer);
    };
  }, [id, acceptSnapshot]);

  useEffect(() => {
    if (isTerminal) {
      setExecutionLink("live");
      return;
    }

    const es = subscribeExecution(
      id,
      (snapshot) => {
        acceptSnapshot(snapshot);
      },
      {
        onOpen: () => setExecutionLink("live"),
        onError: () => setExecutionLink("reconnecting"),
      },
    );
    return () => es.close();
  }, [id, isTerminal, acceptSnapshot]);

  useEffect(() => {
    if (isTerminal) {
      setEntryLink("live");
      return;
    }

    const es = subscribeEntries(
      id,
      (entry) => {
        setEntries((prev) => {
          const key = `${entry.seq}-${entry.id}`;
          if (prev.some((item) => `${item.seq}-${item.id}` === key)) return prev;
          return [...prev, entry].slice(-300);
        });
        setLastEntryAt(Date.now());
      },
      {
        onOpen: () => setEntryLink("live"),
        onError: () => setEntryLink("reconnecting"),
      },
    );
    return () => es.close();
  }, [id, isTerminal]);

  useEffect(() => {
    if (isTerminal) {
      setStreamLink("live");
      return;
    }

    const es = subscribeStream(
      id,
      (evt) => setStreamEvents((prev) => [...prev, evt].slice(-500)),
      {
        onOpen: () => setStreamLink("live"),
        onError: () => setStreamLink("reconnecting"),
      },
    );
    return () => es.close();
  }, [id, isTerminal]);

  const fetchWorld = useCallback(async () => {
    try {
      if (scrubSeq !== null) {
        const world = await getWorldAt(id, scrubSeq);
        setResources(world.resources ?? []);
      } else {
        const world = await getWorld(id);
        setResources(world.resources ?? []);
      }
    } catch {
      /* world not ready */
    }
  }, [id, scrubSeq]);

  useEffect(() => {
    if (tab === "world") {
      void fetchWorld();
    }
  }, [tab, fetchWorld]);

  const commitLogs = useMemo(
    () => session?.log.filter((entry) => entry.commit_id && entry.seq !== undefined) ?? [],
    [session],
  );
  const maxSeq = useMemo(
    () => commitLogs.reduce((max, entry) => Math.max(max, entry.seq ?? 0), 0),
    [commitLogs],
  );
  const currentScrubSeq = scrubSeq ?? maxSeq;
  const pendingApproval = useMemo(() => {
    const log = session?.log
      .slice()
      .reverse()
      .find((entry) => entry.title.startsWith("Approval requested"));
    if (!log) return null;
    const channel = log.detail.match(/^Channel\s+([^:]+):/)?.[1]?.trim() ?? "default";
    return {
      channel,
      proposalId: log.proposal_id,
      tool: log.tool ?? session?.active_tool ?? "tool",
      detail: log.detail,
    };
  }, [session]);

  const handleBranch = useCallback(
    async (commitId: string) => {
      try {
        const res = await branchFrom(id, commitId, "thymos operator branch");
        setBranchMsg(`Shadow branch created at ${res.branch_trajectory_id.slice(0, 12)}...`);
      } catch (error) {
        setBranchMsg(`Branch failed: ${(error as Error).message}`);
      }
    },
    [id],
  );

  const handleCancel = useCallback(async () => {
    setActionBusy("cancel");
    setActionMsg(null);
    try {
      await cancelRun(id);
      setActionMsg("Cancellation signal sent to the runtime.");
      const snapshot = await getExecution(id);
      acceptSnapshot(snapshot);
    } catch (error) {
      setActionMsg(`Cancel failed: ${(error as Error).message}`);
    } finally {
      setActionBusy(null);
    }
  }, [id, acceptSnapshot]);

  const handleResume = useCallback(async () => {
    setActionBusy("resume");
    setActionMsg(null);
    try {
      await resumeRun(id, session?.task ?? "resume run", session?.max_steps ?? 16);
      setActionMsg("Resume requested. Reattaching to execution state.");
      const snapshot = await getExecution(id);
      acceptSnapshot(snapshot);
    } catch (error) {
      setActionMsg(`Resume failed: ${(error as Error).message}`);
    } finally {
      setActionBusy(null);
    }
  }, [id, session, acceptSnapshot]);

  const handleApproval = useCallback(
    async (approve: boolean) => {
      if (!pendingApproval) return;
      setActionBusy(approve ? "approve" : "deny");
      setActionMsg(null);
      try {
        await decideApproval(
          id,
          pendingApproval.channel,
          approve,
          pendingApproval.proposalId,
        );
        setActionMsg(approve ? "Approval sent. Runtime can continue." : "Denial sent. Runtime will adjust.");
        const snapshot = await getExecution(id);
        acceptSnapshot(snapshot);
      } catch (error) {
        setActionMsg(`Approval action failed: ${(error as Error).message}`);
      } finally {
        setActionBusy(null);
      }
    },
    [id, pendingApproval, acceptSnapshot],
  );

  const palette = statusStyles[status];
  const liveLabel = isTerminal
    ? "Session closed"
    : executionLink === "live"
      ? "Execution live"
      : executionLink === "reconnecting"
        ? "Reconnecting"
        : "Connecting";
  const entryLabel = isTerminal
    ? "Ledger closed"
    : entryLink === "live"
      ? "Ledger live"
      : entryLink === "reconnecting"
        ? "Ledger reconnecting"
        : "Ledger connecting";
  const streamLabel = isTerminal
    ? "Stream closed"
    : streamLink === "live"
      ? "Model stream live"
      : streamLink === "reconnecting"
        ? "Stream reconnecting"
        : "Stream connecting";
  const runtimeLagMs = session ? Math.max(0, Date.now() - session.updated_at_ms) : null;

  return (
    <main className="thymos-runtime-shell">
      <section className="thymos-console-panel thymos-runtime-header">
        <div className="thymos-console-topline">
          <span
            className="thymos-status-pill"
            style={{ borderColor: palette.glow, color: palette.tone }}
          >
            <span
              className="thymos-status-pill-dot"
              style={{ background: palette.tone, boxShadow: `0 0 18px ${palette.glow}` }}
            />
            {palette.label}
          </span>
          <span className="thymos-live-pill" data-state={isTerminal ? "closed" : executionLink}>
            <span className="thymos-live-dot" />
            {liveLabel}
          </span>
          <span className="thymos-eyebrow">Thymos Runtime</span>
          <code className="thymos-console-id">{id}</code>
        </div>

        <div className="thymos-runtime-task">
          <h1>Unified Thymos execution console</h1>
          <p>{session?.task ?? "Loading task context..."}</p>
          <div className="thymos-operator-state" style={{ color: palette.tone }}>
            {session?.operator_state ?? "Connecting to the shared runtime state..."}
          </div>
        </div>

        <div className="thymos-console-stat-grid">
          <MetricCard label="Step" value={`${session?.current_step ?? 0}/${session?.max_steps ?? 0}`} />
          <MetricCard label="Intents" value={String(session?.counters.intents_declared ?? 0)} />
          <MetricCard label="Commits" value={String(session?.counters.commits ?? 0)} />
          <MetricCard label="Recoveries" value={String((session?.counters.recoveries ?? 0) + (session?.counters.retries ?? 0))} />
          <MetricCard label="Approvals" value={String(session?.counters.approvals_pending ?? 0)} />
          <MetricCard label="Active Tool" value={session?.active_tool ?? "standby"} />
        </div>

        <div className="thymos-realtime-grid">
          <RealtimeCard
            label="Snapshot Feed"
            value={liveLabel}
            detail={`last ${lastSnapshotAt ? new Date(lastSnapshotAt).toLocaleTimeString() : "pending"} · lag ${formatLag(runtimeLagMs)}`}
            state={isTerminal ? "closed" : executionLink}
          />
          <RealtimeCard
            label="Ledger Feed"
            value={entryLabel}
            detail={`${entries.length} entries · last ${lastEntryAt ? new Date(lastEntryAt).toLocaleTimeString() : "pending"}`}
            state={isTerminal ? "closed" : entryLink}
          />
          <RealtimeCard
            label="Cognition Feed"
            value={streamLabel}
            detail={`${streamEvents.length} events · ${session?.active_tool ?? "no active tool"}`}
            state={isTerminal ? "closed" : streamLink}
          />
        </div>

        <div className="thymos-operator-actions">
          {session?.status === "waiting_approval" && pendingApproval ? (
            <div className="thymos-approval-card">
              <div>
                <span className="thymos-eyebrow">Approval Required</span>
                <strong>{pendingApproval.tool}</strong>
                <p>{pendingApproval.detail}</p>
              </div>
              <div className="thymos-chip-row">
                <PanelButton
                  onClick={() => void handleApproval(true)}
                  disabled={actionBusy !== null}
                >
                  {actionBusy === "approve" ? "Approving" : "Approve"}
                </PanelButton>
                <PanelButton
                  onClick={() => void handleApproval(false)}
                  disabled={actionBusy !== null}
                  tone="danger"
                >
                  {actionBusy === "deny" ? "Denying" : "Deny"}
                </PanelButton>
              </div>
            </div>
          ) : null}

          <div className="thymos-chip-row">
            <PanelButton
              onClick={() => void handleCancel()}
              disabled={!session || isTerminal || actionBusy !== null}
              tone="danger"
            >
              {actionBusy === "cancel" ? "Cancelling" : "Cancel Run"}
            </PanelButton>
            <PanelButton
              onClick={() => void handleResume()}
              disabled={!session || session.status === "completed" || actionBusy !== null}
            >
              {actionBusy === "resume" ? "Resuming" : "Resume"}
            </PanelButton>
          </div>

          {actionMsg ? <p className="thymos-action-message">{actionMsg}</p> : null}
        </div>
      </section>

      <section className="thymos-console-layout">
        <div className="thymos-console-main">
          <div className="thymos-tab-row">
            {(["execution", "ledger", "stream", "world"] as ConsoleTab[]).map((item) => (
              <button
                key={item}
                className={tab === item ? "thymos-tab is-active" : "thymos-tab"}
                onClick={() => setTab(item)}
              >
                {tabLabels[item]}
              </button>
            ))}
          </div>

          {tab === "execution" && <ExecutionLog session={session} />}
          {tab === "ledger" && <EntryTimeline entries={entries} />}
          {tab === "stream" && <StreamView events={streamEvents} />}
          {tab === "world" && <WorldView resources={resources} />}
        </div>

        <aside className="thymos-console-sidebar">
          <SidePanel
            title="Execution State"
            body={`Phase: ${session?.phase ?? "system"}\nLast update: ${
              session ? new Date(session.updated_at_ms).toLocaleTimeString() : "--"
            }\nLive link: ${liveLabel}\nSnapshot: ${
              lastSnapshotAt ? new Date(lastSnapshotAt).toLocaleTimeString() : "pending"
            }\nTrajectory: ${session?.trajectory_id ? session.trajectory_id.slice(0, 16) : "pending"}`}
          />

          <SidePanel
            title="Live Stream"
            body={`Execution: ${liveLabel}\nLedger: ${entryLabel}\nModel: ${streamLabel}\nEvents received: ${streamEvents.length}\nLedger entries: ${entries.length}\nActive tool: ${
              session?.active_tool ?? "standby"
            }`}
            accent={streamLink === "live" && !isTerminal ? "#34d399" : "#fbbf24"}
          />

          <SidePanel
            title="Outcome"
            body={session?.final_answer ?? "Final answer will appear here when the runtime resolves the task."}
            accent={session?.status === "completed" ? "#34d399" : "#77a9ff"}
          />

          <div className="thymos-console-panel-soft thymos-panel">
            <div className="thymos-world-replay-head">
              <strong className="thymos-panel-title">World Replay</strong>
              <span>
                {currentScrubSeq} / {maxSeq}
              </span>
            </div>

            {maxSeq > 0 ? (
              <>
                <input
                  className="thymos-range"
                  type="range"
                  min={0}
                  max={maxSeq}
                  value={currentScrubSeq}
                  onChange={(event) => setScrubSeq(Number(event.target.value))}
                  aria-label="Trajectory replay scrubber"
                />
                <div className="thymos-chip-row" style={{ marginTop: 12 }}>
                  {scrubSeq !== null ? (
                    <PanelButton onClick={() => setScrubSeq(null)}>Jump To Head</PanelButton>
                  ) : null}
                  {(() => {
                    const commit = commitLogs.find((entry) => entry.seq === currentScrubSeq);
                    if (!commit?.commit_id) return null;
                    return <PanelButton onClick={() => handleBranch(commit.commit_id!)}>Branch Here</PanelButton>;
                  })()}
                </div>
              </>
            ) : (
              <div className="thymos-empty-state">
                <strong>Replay Waiting</strong>
                <p>
                  Commit-backed world replay becomes available once the runtime records execution
                  results.
                </p>
              </div>
            )}

            {branchMsg ? (
              <p className="thymos-panel-copy" style={{ marginTop: 12, color: "#cfe0ff" }}>
                {branchMsg}
              </p>
            ) : null}
          </div>
        </aside>
      </section>
    </main>
  );
}

function MetricCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="thymos-console-stat">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function RealtimeCard({
  label,
  value,
  detail,
  state,
}: {
  label: string;
  value: string;
  detail: string;
  state: "connecting" | "live" | "reconnecting" | "closed";
}) {
  return (
    <div className="thymos-realtime-card" data-state={state}>
      <span className="thymos-live-dot" />
      <div>
        <span>{label}</span>
        <strong>{value}</strong>
        <p>{detail}</p>
      </div>
    </div>
  );
}

function formatLag(value: number | null) {
  if (value === null) return "pending";
  if (value < 1000) return `${value}ms`;
  return `${(value / 1000).toFixed(1)}s`;
}

function SidePanel({ title, body, accent = "#77a9ff" }: { title: string; body: string; accent?: string }) {
  return (
    <div className="thymos-console-panel-soft thymos-panel">
      <strong className="thymos-panel-title">{title}</strong>
      <div className="thymos-panel-bar" style={{ borderLeftColor: accent }}>
        {body}
      </div>
    </div>
  );
}

function PanelButton({
  onClick,
  children,
  disabled = false,
  tone = "default",
}: {
  onClick: () => void;
  children: ReactNode;
  disabled?: boolean;
  tone?: "default" | "danger";
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="thymos-button-secondary"
      data-tone={tone}
    >
      {children}
    </button>
  );
}
