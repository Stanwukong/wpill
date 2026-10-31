import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentApprovalActivity, AgentEntry, AgentEvent, AgentRun } from "../lib/types";
import { ClockGlyph, RiskGlyph, StopGlyph } from "../features/agent/AgentGlyphs";
import { endLabel, RISK_LABEL, runLabel } from "../features/agent/labels";
import { decide, isLive, stopRun } from "../features/agent/useAgentRuns";
import type { RoutineSeed } from "./routineLabels";
import type { Transcript } from "./useTranscripts";

type Decision = Extract<AgentEvent, { kind: "decision" }>;

/** Distance from the bottom (CSS px) that still counts as "at the bottom" for auto-scroll. */
const STICK_PX = 24;

/**
 * One run: its header (project, folder, status, Stop), the transcript, approvals waiting on it, and the follow-up
 * box (queued into the running turn, or resumes the run once it has ended).
 */
export function RunView({
  runId,
  run,
  transcript,
  focusTick,
  onSaveAsRoutine,
}: {
  runId: string;
  /** Missing for a moment right after the run starts (until `agent:runs` arrives). */
  run: AgentRun | undefined;
  transcript: Transcript | undefined;
  focusTick: number;
  onSaveAsRoutine: (seed: RoutineSeed) => void;
}) {
  const entries = transcript?.entries ?? [];
  const live = run ? isLive(run.status) : true;
  const log = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);

  // Follow new output while the reader is at the bottom; leave them alone once they scroll up.
  useLayoutEffect(() => {
    const el = log.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [entries, run?.pending.length]);

  const decisions = useMemo(() => {
    const byRequest = new Map<number, Decision>();
    for (const e of entries) if (e.kind === "decision") byRequest.set(e.requestId, e);
    return byRequest;
  }, [entries]);
  const asked = useMemo(
    () => new Set(entries.flatMap((e) => (e.kind === "approval" ? [e.requestId] : []))),
    [entries],
  );

  return (
    <div className="run">
      <div className="run__head">
        <span className="run__project">
          {run?.routine?.trigger === "schedule" && <ClockGlyph />}
          {run ? (run.routine?.name ?? run.project) : "Starting…"}
        </span>
        <span className="run__cwd" title={run?.cwd}>
          {run?.cwd}
        </span>
        {run && (
          <span className="run__status" data-status={run.status}>
            {runLabel(run)}
          </span>
        )}
        {run && !live && !run.routine && (
          <button
            className="cmd-btn cmd-btn--quiet run__stop"
            title="Save this task with a schedule"
            onClick={() =>
              onSaveAsRoutine({
                // The run's prompt is shortened; the transcript has it whole.
                prompt: entries.find((e): e is Extract<AgentEntry, { kind: "user" }> => e.kind === "user")?.text ?? run.prompt,
                cwd: run.cwd,
                model: run.model,
              })
            }
          >
            Save as routine
          </button>
        )}
        {run && live && (
          <button className="cmd-btn cmd-btn--quiet run__stop" onClick={() => stopRun(runId)}>
            <StopGlyph />
            Stop
          </button>
        )}
      </div>

      <div
        ref={log}
        className="run__log"
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
        }}
      >
        {transcript?.error && <p className="entry entry--error">{transcript.error}</p>}
        {entries.map((entry, i) => (
          <Entry key={i} entry={entry} decisions={decisions} asked={asked} />
        ))}
        {transcript?.loading && entries.length === 0 && <p className="run__empty">Loading…</p>}
      </div>

      {run && run.pending.length > 0 && (
        <ul className="run__pending" aria-label="Waiting for approval">
          {run.pending.map((a) => (
            <li key={a.requestId}>
              <PendingCard approval={a} />
            </li>
          ))}
        </ul>
      )}

      <Composer runId={runId} live={live} focusTick={focusTick} />
    </div>
  );
}

function Entry({
  entry,
  decisions,
  asked,
}: {
  entry: AgentEntry;
  decisions: Map<number, Decision>;
  asked: Set<number>;
}) {
  switch (entry.kind) {
    case "user":
      return <p className="entry entry--user">{entry.text}</p>;
    case "text":
      return <p className="entry entry--text">{entry.text}</p>;
    case "tool":
      return (
        <p className="entry entry--tool">
          <span className="entry__chevron" aria-hidden="true">
            ›
          </span>
          {entry.label}
        </p>
      );
    case "toolResult":
      return <ToolResult isError={entry.isError} text={entry.text} />;
    case "approval": {
      const d = decisions.get(entry.requestId);
      return (
        <p className="entry entry--note" data-outcome={d ? (d.allow ? "allowed" : "denied") : "waiting"}>
          <span className="entry__note-label">Asked</span>
          <span className="entry__note-what">
            {entry.tool} {entry.summary}
          </span>
          <span className="entry__note-outcome">{d ? decisionText(d) : "Waiting for you"}</span>
        </p>
      );
    }
    case "decision":
      // Shown with its approval; stand alone only if the approval isn't in the transcript.
      if (asked.has(entry.requestId)) return null;
      return (
        <p className="entry entry--note" data-outcome={entry.allow ? "allowed" : "denied"}>
          <span className="entry__note-outcome">{decisionText(entry)}</span>
        </p>
      );
    case "result":
      return (
        <p className="entry entry--result" data-status={entry.status}>
          {[
            endLabel(entry.status, entry.reason),
            `${entry.turns} ${entry.turns === 1 ? "turn" : "turns"}`,
            entry.costUsd != null ? `$${entry.costUsd.toFixed(4)}` : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </p>
      );
    case "error":
      return <p className="entry entry--error">{entry.message}</p>;
  }
}

function decisionText(d: Decision): string {
  switch (d.by) {
    case "user":
      return d.allow ? "Allowed by you" : "Denied by you";
    case "remembered":
      return d.allow ? "Allowed for this run" : "Denied (remembered)";
    case "timeout":
      return "Denied (timed out)";
    case "runEnded":
      return "Cancelled (run ended)";
  }
}

/** Tool output: collapsed to one muted line ("Output · 12 lines"), expandable. */
function ToolResult({ isError, text }: { isError: boolean; text: string }) {
  const [open, setOpen] = useState(false);
  const lines = text === "" ? 0 : text.split("\n").length;
  const label = isError ? "Error" : "Output";
  return (
    <div className="entry entry--result-block" data-error={isError}>
      <button className="entry__toggle" aria-expanded={open} disabled={lines === 0} onClick={() => setOpen((v) => !v)}>
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round">
          <path d="M6.5 4l4 4-4 4" />
        </svg>
        {lines === 0 ? `${label} · empty` : `${label} · ${lines} ${lines === 1 ? "line" : "lines"}`}
      </button>
      {open && <pre className="entry__output">{text}</pre>}
    </div>
  );
}

function PendingCard({ approval }: { approval: AgentApprovalActivity }) {
  const [sent, setSent] = useState(false);
  const answer = (decision: "allow" | "deny", remember: "once" | "run" = "once") => {
    setSent(true);
    decide(approval.requestId, decision, remember).catch(() => setSent(false));
  };
  return (
    <div className="pending" data-risk={approval.risk}>
      <span className="pending__what">
        <span className="pending__tool">{approval.tool}</span>
        <span className="agent-risk" data-risk={approval.risk}>
          <RiskGlyph desktop={approval.desktop} risk={approval.risk} />
          {RISK_LABEL[approval.risk]}
        </span>
        <span className="pending__summary" title={approval.summary}>
          {approval.summary}
        </span>
      </span>
      {approval.detail != null && <pre className="pending__detail">{approval.detail}</pre>}
      <span className="pending__actions">
        {approval.rememberAs && (
          <button className="agent-link" disabled={sent} onClick={() => answer("allow", "run")}>
            Allow {approval.rememberAs} for this run
          </button>
        )}
        <button className="agent-btn agent-btn--quiet" disabled={sent} onClick={() => answer("deny")}>
          Deny
        </button>
        <button className="agent-btn agent-btn--allow" disabled={sent} onClick={() => answer("allow")}>
          Allow
        </button>
      </span>
    </div>
  );
}

/** Enter sends, Shift+Enter is a newline. Grows to about five lines. */
function Composer({ runId, live, focusTick }: { runId: string; live: boolean; focusTick: number }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    ref.current?.focus();
  }, [focusTick]);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
  }, [text]);

  const send = async () => {
    const value = text.trim();
    if (!value || busy) return;
    setBusy(true);
    setError(null);
    try {
      await commands.agentSend(runId, value);
      setText("");
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
    }
  };

  return (
    <div className="composer">
      {error && (
        <span className="cmd-error" role="alert">
          {error}
        </span>
      )}
      <div className="composer__box">
        <textarea
          ref={ref}
          rows={1}
          className="composer__input"
          placeholder={live ? "Add to this task…" : "Follow up…"}
          value={text}
          spellCheck={false}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKey}
        />
        <button
          className="composer__send"
          aria-label="Send"
          title="Send (Enter)"
          disabled={!text.trim() || busy}
          onClick={() => void send()}
        >
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round">
            <path d="M8 12.5v-9M4 7.5l4-4 4 4" />
          </svg>
        </button>
      </div>
    </div>
  );
}
