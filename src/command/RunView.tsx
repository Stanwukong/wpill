import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentApprovalActivity, AgentEntry, AgentRun, Interaction } from "../lib/types";
import { ClockGlyph, RiskGlyph, StopGlyph } from "../features/agent/AgentGlyphs";
import { RISK_LABEL, runLabel } from "../features/agent/labels";
import { changePreview, PreviewBlock } from "../features/agent/PreviewBlock";
import { ModeChip } from "../features/agent/ModeChip";
import { decide, isLive, stopRun } from "../features/agent/useAgentRuns";
import { AgentTag } from "../features/sessions/AgentTag";
import { StepLabel } from "../features/sessions/StepLabel";
import { PlanView, QuestionForm } from "./InteractionViews";
import { Markdown } from "./Markdown";
import type { RoutineSeed } from "./routineLabels";
import { decisionText, resultParts } from "./runMarkdown";
import { turns, type Call, type Turn, type TurnItem } from "./transcriptTurns";
import { useWholePreview } from "./useSessionAsks";
import type { Transcript } from "./useTranscripts";

/** Distance from the bottom (CSS px) that still counts as "at the bottom" for auto-scroll. */
const STICK_PX = 24;

/**
 * One run: its header (project, folder, status, mode chip, Stop), the transcript, approvals waiting on it, and the follow-up
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

  const shaped = useMemo(() => turns(entries), [entries]);

  return (
    <div className="run">
      <div className="run__head">
        <span className="run__project">
          {run?.routine?.trigger === "schedule" && <ClockGlyph />}
          {run ? (run.routine?.name ?? run.project) : "Starting…"}
        </span>
        {run && <AgentTag agent={run.backend} mixed={false} />}
        <span className="run__cwd" title={run?.cwd}>
          {run?.cwd}
        </span>
        {run?.status === "working" ? (
          <StepLabel className="run__status" text={runLabel(run)} todos={run.todos} />
        ) : (
          run && (
            <span className="run__status" data-status={run.status}>
              {runLabel(run)}
            </span>
          )
        )}
        {run && <ModeChip run={run} />}
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
                backend: run.backend,
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
        {shaped.map((turn, i) => (
          <TurnView key={turn.user ? `turn:${turn.user.atMs}:${i}` : `turn:start`} turn={turn} />
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

function TurnView({ turn }: { turn: Turn }) {
  return (
    <section className="turn">
      {turn.user && (
        <div className="entry entry--user" aria-label="You">
          <Markdown source={turn.user.text} />
        </div>
      )}
      {turn.items.length > 0 && (
        <div className="turn__agent">
          {turn.items.map((item, i) => (
            <Item key={itemKey(item, i)} item={item} />
          ))}
        </div>
      )}
    </section>
  );
}

/** Stable across appends: an item's first entry never changes once it's in the transcript. */
function itemKey(item: TurnItem, i: number): string {
  if (item.kind === "tools") {
    const first = item.calls[0];
    return `tools:${first?.tool?.id ?? first?.result?.id ?? i}`;
  }
  return `${item.kind}:${item.entry.atMs}:${i}`;
}

function Item({ item }: { item: TurnItem }) {
  switch (item.kind) {
    case "text":
      return (
        <div className="entry entry--text">
          <Markdown source={item.entry.text} />
        </div>
      );
    case "tools":
      return item.calls.length === 1 ? (
        <div className="entry entry--calls">
          <CallView call={item.calls[0]} />
        </div>
      ) : (
        <ToolGroup calls={item.calls} summary={item.summary} failed={item.failed} />
      );
    case "approval": {
      const d = item.decision;
      return (
        <p className="entry entry--note" data-outcome={d ? (d.allow ? "allowed" : "denied") : "waiting"}>
          <span className="entry__note-label">Asked</span>
          <span className="entry__note-what">
            {item.entry.tool} {item.entry.summary}
          </span>
          <span className="entry__note-outcome">{d ? decisionText(d) : "Waiting for you"}</span>
        </p>
      );
    }
    case "decision":
      return (
        <p className="entry entry--note" data-outcome={item.entry.allow ? "allowed" : "denied"}>
          <span className="entry__note-outcome">{decisionText(item.entry)}</span>
        </p>
      );
    case "result": {
      const { outcome, details } = resultParts(item.entry);
      return (
        <p className="entry entry--result" data-status={item.entry.status}>
          <span className="entry__result-outcome">{outcome}</span>
          <span className="entry__result-details">{details.join(" · ")}</span>
        </p>
      );
    }
    case "error":
      return <p className="entry entry--error">{item.entry.message}</p>;
  }
}

/** Consecutive tool calls as one row ("Read 3 files, ran 2 commands"); open by default when one of them failed. */
function ToolGroup({ calls, summary, failed }: { calls: Call[]; summary: string; failed: boolean }) {
  const [open, setOpen] = useState(failed);
  const failures = calls.filter((c) => c.result?.isError).length;
  return (
    <div className="entry entry--tools" data-failed={failed}>
      <button className="tools__head" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <Chevron />
        <span className="tools__summary">{summary}</span>
        {failures > 0 && <span className="tools__failed">{failures} failed</span>}
      </button>
      {open && (
        <div className="tools__calls">
          {calls.map((c, i) => (
            <CallView key={c.tool?.id ?? c.result?.id ?? i} call={c} />
          ))}
        </div>
      )}
    </div>
  );
}

/** One call: its label, an edit's diff under it, then its result folded to one line (errors open). */
function CallView({ call }: { call: Call }) {
  const preview = changePreview(call.tool?.preview ?? null);
  return (
    <div className="call">
      {call.tool && (
        <p className="call__label">
          <span className="entry__chevron" aria-hidden="true">
            ›
          </span>
          {call.tool.label}
        </p>
      )}
      {preview && (
        <div className="cmd-preview call__preview">
          <PreviewBlock preview={preview} />
        </div>
      )}
      {call.result && <ToolResult isError={call.result.isError} text={call.result.text} />}
    </div>
  );
}

function Chevron() {
  return (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" aria-hidden="true">
      <path d="M6.5 4l4 4-4 4" />
    </svg>
  );
}

/** Lines a result shows before "Show all" (results are cut to 2,000 chars by Rust, so "all" stays small). */
const RESULT_LINES = 20;

/** Tool output: collapsed to one muted line ("Output · 12 lines"), expandable; errors start open. */
function ToolResult({ isError, text }: { isError: boolean; text: string }) {
  const [open, setOpen] = useState(isError);
  const [whole, setWhole] = useState(false);
  const all = text === "" ? [] : text.split("\n");
  const lines = all.length;
  const label = isError ? "Error" : "Output";
  const cut = !whole && lines > RESULT_LINES;
  return (
    <div className="entry--result-block" data-error={isError}>
      <button className="entry__toggle" aria-expanded={open} disabled={lines === 0} onClick={() => setOpen((v) => !v)}>
        <Chevron />
        {lines === 0 ? `${label} · empty` : `${label} · ${lines} ${lines === 1 ? "line" : "lines"}`}
      </button>
      {open && lines > 0 && <pre className="entry__output">{cut ? all.slice(0, RESULT_LINES).join("\n") : text}</pre>}
      {open && cut && (
        <button className="entry__more" onClick={() => setWhole(true)}>
          Show all · {lines - RESULT_LINES} more {lines - RESULT_LINES === 1 ? "line" : "lines"}
        </button>
      )}
    </div>
  );
}

function PendingCard({ approval }: { approval: AgentApprovalActivity }) {
  return approval.interaction ? (
    <PendingInteraction approval={approval} interaction={approval.interaction} />
  ) : (
    <PendingApproval approval={approval} />
  );
}

/** A question or a plan (2.8c): what kind, then the form; the question text is the form's, so no summary for it. */
function PendingInteraction({ approval, interaction }: { approval: AgentApprovalActivity; interaction: Interaction }) {
  const { requestId } = approval;
  return (
    <div className="pending pending--interaction" data-risk={approval.risk}>
      <span className="pending__what">
        <span className="pending__tool">{approval.tool}</span>
        {interaction.kind === "plan" && (
          <span className="pending__summary" title={interaction.title}>
            {interaction.title}
          </span>
        )}
      </span>
      {interaction.kind === "question" ? (
        <QuestionForm
          interaction={interaction}
          onAnswer={(answer) => commands.agentAnswer(requestId, answer)}
          onSkip={() => decide(requestId, "deny", "once")}
        />
      ) : (
        <PlanView
          requestId={requestId}
          fetch={commands.agentApprovalPreview}
          cutNote="Cut on the way to Cay."
          onPlan={(approve, feedback) => commands.agentPlan(requestId, approve, feedback)}
          onReject={() => decide(requestId, "deny", "once")}
        />
      )}
    </div>
  );
}

function PendingApproval({ approval }: { approval: AgentApprovalActivity }) {
  const [sent, setSent] = useState(false);
  const preview = changePreview(useWholePreview(approval.requestId, approval.preview, commands.agentApprovalPreview));
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
      {preview && (
        <div className="cmd-preview">
          <PreviewBlock preview={preview} />
        </div>
      )}
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
