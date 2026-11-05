import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MutableRefObject } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentApprovalActivity, AgentEntry, AgentRun, Interaction } from "../lib/types";
import { ClockGlyph, RiskGlyph, StopGlyph } from "../features/agent/AgentGlyphs";
import { MODE_LABEL, RISK_LABEL, runLabel } from "../features/agent/labels";
import { changePreview, PreviewBlock } from "../features/agent/PreviewBlock";
import { ModeChip } from "../features/agent/ModeChip";
import { decide, isLive, stopRun } from "../features/agent/useAgentRuns";
import { AgentTag } from "../features/sessions/AgentTag";
import { StepLabel } from "../features/sessions/StepLabel";
import { WorkingIndicator } from "../features/sessions/WorkingGlyphs";
import { AttachButton, AttachmentChips, useAttachments, useDropTarget, type DropInto } from "./Attachments";
import { composerDrafts } from "./composerDrafts";
import { PlanView, QuestionForm } from "./InteractionViews";
import { Markdown } from "./Markdown";
import type { RoutineSeed } from "./routineLabels";
import { decisionText, resultParts, toMarkdown } from "./runMarkdown";
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
  focusOnOpen,
  dropInto,
  picking,
  onSaveAsRoutine,
}: {
  runId: string;
  /** Missing for a moment right after the run starts (until `agent:runs` arrives). */
  run: AgentRun | undefined;
  transcript: Transcript | undefined;
  focusTick: number;
  /** `false` when opened from the run list by an arrow key: the row keeps focus until the window's focus changes. */
  focusOnOpen: boolean;
  /** Drops on the window go to this run's follow-up box while it shows. */
  dropInto: DropInto;
  picking: MutableRefObject<boolean>;
  onSaveAsRoutine: (seed: RoutineSeed) => void;
}) {
  const entries = transcript?.entries ?? [];
  const draft = transcript?.draft ?? null;
  const live = run ? isLive(run.status) : true;
  const log = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  /** A "Jump to latest" scroll is under way: its in-between positions still count as pinned. */
  const jumping = useRef(false);
  /** New output arrived while the reader was scrolled up. */
  const [behind, setBehind] = useState(false);
  const [copied, setCopied] = useState(false);

  // Follow new output (and the draft as it grows) while the reader is at the bottom; once they scroll up, leave them
  // alone and offer "Jump to latest".
  useLayoutEffect(() => {
    const el = log.current;
    if (!el) return;
    if (atBottom.current) el.scrollTop = el.scrollHeight;
    else setBehind(true);
  }, [entries, draft, run?.pending.length]);

  const jump = () => {
    const el = log.current;
    if (!el) return;
    atBottom.current = true;
    jumping.current = true;
    setBehind(false);
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    el.scrollTo({ top: el.scrollHeight, behavior: still ? "auto" : "smooth" });
  };

  const shaped = useMemo(() => turns(entries), [entries]);
  // A draft with no turn yet (a cut transcript) still needs somewhere to show.
  const shown = draft != null && shaped.length === 0 ? [{ user: null, items: [] }] : shaped;
  const title = run ? (run.routine?.name ?? run.project) : undefined;

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
        {entries.length > 0 && (
          <button
            className="cmd-btn cmd-btn--quiet run__stop"
            title="Copy the whole run as Markdown"
            onClick={() => navigator.clipboard.writeText(toMarkdown(entries, title)).then(() => setCopied(true), () => {})}
            onPointerLeave={() => setCopied(false)}
            onBlur={() => setCopied(false)}
          >
            {copied ? "Copied" : "Copy"}
          </button>
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

      <div className="run__logwrap">
        <div
          ref={log}
          className="run__log"
          onScroll={(e) => {
            const el = e.currentTarget;
            const bottom = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
            if (bottom) jumping.current = false;
            atBottom.current = bottom || jumping.current;
            if (bottom) setBehind(false);
          }}
          onWheel={() => {
            jumping.current = false;
          }}
        >
          {transcript?.error && <p className="entry entry--error">{transcript.error}</p>}
          {shown.map((turn, i) => (
            <TurnView
              key={turn.user ? `turn:${turn.user.atMs}:${i}` : `turn:start`}
              turn={turn}
              draft={i === shown.length - 1 ? draft : null}
            />
          ))}
          {transcript?.loading && entries.length === 0 && <p className="run__empty">Loading…</p>}
        </div>
        {behind && (
          <button type="button" className="run__jump" onClick={jump}>
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <path d="M8 3.5v9M4 8.5l4 4 4-4" />
            </svg>
            Jump to latest
          </button>
        )}
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

      <Composer
        runId={runId}
        live={live}
        acceptsImages={run?.acceptsImages ?? null}
        meta={run ? composerMeta(run) : null}
        focusTick={focusTick}
        focusOnOpen={focusOnOpen}
        dropInto={dropInto}
        picking={picking}
      />
    </div>
  );
}

function TurnView({ turn, draft }: { turn: Turn; draft: string | null }) {
  return (
    <section className="turn">
      {turn.user && (
        <div className="entry entry--user" aria-label="You">
          <CopyMessage text={turn.user.text} />
          <Markdown source={turn.user.text} />
          {turn.user.attachments && <AttachmentChips items={turn.user.attachments} />}
        </div>
      )}
      {(turn.items.length > 0 || draft != null) && (
        <div className="turn__agent">
          {/* One list, so the draft (keyed as its text will be) is replaced in place by the real entry: no fade, no jump. */}
          {[
            ...turn.items.map((item, i) =>
              item.kind === "text" ? (
                <TextItem key={textKey(i)} text={item.entry.text} />
              ) : (
                <Item key={itemKey(item, i)} item={item} />
              ),
            ),
            draft != null && <TextItem key={textKey(turn.items.length)} text={draft} writing />,
          ]}
        </div>
      )}
    </section>
  );
}

/** By position: items only ever append to a turn, and a draft becomes the text at the same index. */
function textKey(i: number): string {
  return `text:${i}`;
}

/** Stable across appends: an item's first entry never changes once it's in the transcript. */
function itemKey(item: Exclude<TurnItem, { kind: "text" }>, i: number): string {
  if (item.kind === "tools") {
    const first = item.calls[0];
    return `tools:${first?.tool?.id ?? first?.result?.id ?? i}`;
  }
  return `${item.kind}:${item.entry.atMs}:${i}`;
}

/** Claude's text, or its draft while it's written (`writing`: the working wave in the gutter instead of Copy). */
function TextItem({ text, writing = false }: { text: string; writing?: boolean }) {
  return (
    <div className="entry entry--text" data-writing={writing || undefined}>
      {writing ? (
        <span className="entry__writing">
          <WorkingIndicator small />
        </span>
      ) : (
        <CopyMessage text={text} />
      )}
      <Markdown source={text} />
    </div>
  );
}

/**
 * Copies a message's Markdown source. Shows on hover or focus of its message; "Copied" (a check) holds until the
 * pointer or focus leaves, so no timer runs.
 */
function CopyMessage({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="entry__copy"
      data-copied={copied || undefined}
      aria-label={copied ? "Copied" : "Copy message"}
      title={copied ? "Copied" : "Copy message"}
      onClick={() => navigator.clipboard.writeText(text).then(() => setCopied(true), () => {})}
      onPointerLeave={() => setCopied(false)}
      onBlur={() => setCopied(false)}
    >
      {copied ? <CheckGlyph /> : <CopyGlyph />}
    </button>
  );
}

function CopyGlyph() {
  return (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinejoin="round" aria-hidden="true">
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.8" />
      <path d="M10.5 3.2A1.8 1.8 0 0 0 8.8 2H4A2 2 0 0 0 2 4v4.8a1.8 1.8 0 0 0 1.2 1.7" />
    </svg>
  );
}

function CheckGlyph() {
  return (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M3.5 8.5l3 3 6-7" />
    </svg>
  );
}

function Item({ item }: { item: Exclude<TurnItem, { kind: "text" }> }) {
  switch (item.kind) {
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

/** "claude · haiku · Ask": whose run it is, its model if one was picked, and the mode the next message runs in. */
interface ComposerMeta {
  agent: string;
  model: string | null;
  mode: string;
}

function composerMeta(run: AgentRun): ComposerMeta {
  return { agent: run.backend, model: run.model, mode: MODE_LABEL[run.nextMode ?? run.mode] };
}

/**
 * Enter sends, Shift+Enter is a newline. Grows to about five lines. Attachments ride along as chips above the box;
 * a refused send keeps them so one can be removed and the message sent again. Unsent text is kept per run
 * (`composerDrafts`) while the window lives; attachments aren't.
 */
function Composer({
  runId,
  live,
  acceptsImages,
  meta,
  focusTick,
  focusOnOpen,
  dropInto,
  picking,
}: {
  runId: string;
  live: boolean;
  /** `false`: an ACP agent that gets images as file links; `null`: not known yet. */
  acceptsImages: boolean | null;
  /** The run's agent, model and mode; `null` until the run is known. */
  meta: ComposerMeta | null;
  focusTick: number;
  focusOnOpen: boolean;
  dropInto: DropInto;
  picking: MutableRefObject<boolean>;
}) {
  const [text, setText] = useState(() => composerDrafts.get(runId) ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);
  const attachments = useAttachments(picking, setError);
  useDropTarget(dropInto, true, attachments.addPaths);
  const imagesAsFiles = acceptsImages === false && attachments.items.some((a) => a.kind === "image");

  // Compared with the tick it opened at (not a "mounted" flag), so StrictMode's second run of the effect agrees.
  const openTick = useRef(focusTick);
  useEffect(() => {
    if (focusOnOpen || focusTick !== openTick.current) ref.current?.focus();
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
      const paths = attachments.items.map((a) => a.path);
      await commands.agentSend(runId, value, paths.length > 0 ? paths : undefined);
      composerDrafts.clear(runId);
      setText("");
      attachments.clear();
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
        {attachments.items.length > 0 && (
          <div className="composer__attachments">
            <AttachmentChips items={attachments.items} onRemove={attachments.remove} />
            {imagesAsFiles && <p className="composer__note">This agent gets images as files.</p>}
          </div>
        )}
        <textarea
          ref={ref}
          rows={1}
          className="composer__input"
          placeholder={live ? "Add to this task…" : "Follow up…"}
          value={text}
          spellCheck={false}
          onChange={(e) => {
            setText(e.target.value);
            composerDrafts.set(runId, e.target.value);
          }}
          onKeyDown={onKey}
          onPaste={attachments.onPaste}
        />
        <AttachButton onClick={() => void attachments.pick().then(() => ref.current?.focus())} />
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
      {meta && (
        <div className="composer__meta">
          <span className="composer__run" title={[meta.agent, meta.model, meta.mode].filter(Boolean).join(" · ")}>
            <span>{meta.agent}</span>
            {meta.model && <span className="composer__model">{meta.model}</span>}
            <span>{meta.mode}</span>
          </span>
          <span className="composer__hint">Enter to send · Shift+Enter for a new line</span>
        </div>
      )}
    </div>
  );
}
