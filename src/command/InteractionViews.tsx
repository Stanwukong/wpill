import { useState, type KeyboardEvent, type ReactNode } from "react";
import { errorMessage } from "../lib/ipc";
import type { ApprovalPreview, Interaction, QuestionAnswer, QuestionView } from "../lib/types";
import { Markdown } from "./Markdown";
import { useWholePreview } from "./useSessionAsks";

type Question = Extract<Interaction, { kind: "question" }>;

/** One question's draft: chosen labels, or your own words (typing clears the labels, choosing clears the words). */
interface Draft {
  labels: string[];
  other: string;
}

const EMPTY: Draft = { labels: [], other: "" };

function filled(d: Draft | undefined): d is Draft {
  return d !== undefined && (d.labels.length > 0 || d.other.trim() !== "");
}

/**
 * Claude's questions in the command window (2.8c): each as a block (header chip, text, options as radio or checkbox
 * rows with their descriptions, an "Other" field). Answered ones show their answer. "Send" answers the filled ones in
 * order, one call each; Rust re-sends the request with the answers recorded and withdraws it once all are in.
 * `lead` sits at the start of the footer (e.g. "Show session"); without `waiting` the form is read-only.
 */
export function QuestionForm({
  interaction,
  waiting = true,
  lead,
  onAnswer,
  onSkip,
}: {
  interaction: Question;
  waiting?: boolean;
  lead?: ReactNode;
  onAnswer: (answer: QuestionAnswer) => Promise<void>;
  onSkip: () => Promise<void>;
}) {
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const open = interaction.questions.filter((q) => !(q.question in interaction.answers));
  const ready = open.filter((q) => filled(drafts[q.question]));
  const disabled = busy || !waiting;

  const edit = (q: QuestionView, d: Draft) => setDrafts((all) => ({ ...all, [q.question]: d }));

  const send = async () => {
    if (disabled || ready.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      for (const q of ready) {
        const d = drafts[q.question];
        const other = d.other.trim();
        await onAnswer({ question: q.question, labels: other ? [] : d.labels, other: other || null });
      }
      // All of them sent: Rust withdraws the request; stay disabled until it does.
      if (ready.length < open.length) setBusy(false);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  const skip = () => {
    setBusy(true);
    setError(null);
    onSkip().catch((err: unknown) => {
      setError(errorMessage(err));
      setBusy(false);
    });
  };

  return (
    <div className="iq">
      {interaction.questions.map((q) =>
        q.question in interaction.answers ? (
          <AnsweredQuestion key={q.question} question={q} answer={interaction.answers[q.question]} />
        ) : (
          <QuestionField
            key={q.question}
            question={q}
            draft={drafts[q.question] ?? EMPTY}
            disabled={disabled}
            onChange={(d) => edit(q, d)}
            onSubmit={() => void send()}
          />
        ),
      )}
      <Footer lead={lead} error={error} waiting={waiting}>
        <button className="agent-btn agent-btn--quiet" disabled={disabled} onClick={skip}>
          Skip
        </button>
        <button
          className="agent-btn agent-btn--allow"
          disabled={disabled || ready.length === 0}
          onClick={() => void send()}
        >
          {open.length > 1 ? "Send answers" : "Send"}
        </button>
      </Footer>
    </div>
  );
}

function QuestionField({
  question,
  draft,
  disabled,
  onChange,
  onSubmit,
}: {
  question: QuestionView;
  draft: Draft;
  disabled: boolean;
  onChange: (d: Draft) => void;
  onSubmit: () => void;
}) {
  const { header, multiSelect, options } = question;
  const toggle = (label: string) => {
    const on = draft.labels.includes(label);
    const labels = multiSelect ? (on ? draft.labels.filter((l) => l !== label) : [...draft.labels, label]) : [label];
    onChange({ labels, other: "" });
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      onSubmit();
    }
  };

  return (
    <div className="iq__q">
      <p className="iq__ask">
        {header && <span className="iq__header">{header}</span>}
        {question.question}
      </p>
      <div className="iq__options" role={multiSelect ? "group" : "radiogroup"} aria-label={header || question.question}>
        {options.map((o) => {
          const on = draft.labels.includes(o.label);
          return (
            <button
              key={o.label}
              className="iq__option"
              role={multiSelect ? "checkbox" : "radio"}
              aria-checked={on}
              data-multi={multiSelect}
              disabled={disabled}
              onClick={() => toggle(o.label)}
            >
              <span className="iq__mark" aria-hidden="true" />
              <span className="iq__label">{o.label}</span>
              {o.description && <span className="iq__desc">{o.description}</span>}
            </button>
          );
        })}
      </div>
      <input
        className="iq__other"
        placeholder="Other…"
        aria-label="Other answer"
        value={draft.other}
        spellCheck={false}
        disabled={disabled}
        onChange={(e) => onChange({ labels: [], other: e.target.value })}
        onKeyDown={onKey}
      />
    </div>
  );
}

function AnsweredQuestion({ question, answer }: { question: QuestionView; answer: string }) {
  return (
    <div className="iq__q" data-answered="true">
      <p className="iq__ask">
        {question.header && <span className="iq__header">{question.header}</span>}
        {question.question}
      </p>
      <p className="iq__answer">
        <svg
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth={1.6}
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M3.5 8.5l3 3 6-6.5" />
        </svg>
        {answer}
      </p>
    </div>
  );
}

/**
 * Claude's plan (2.8c): the whole Markdown (fetched once; never on the card), a note if it was cut, then Approve,
 * your feedback ("Send feedback" sends it back and Claude keeps planning) or Reject. After any of them the controls
 * stay disabled until Rust withdraws the request; they come back on an error.
 */
export function PlanView({
  requestId,
  fetch,
  cutNote,
  waiting = true,
  lead,
  onPlan,
  onReject,
}: {
  requestId: number;
  fetch: (requestId: number) => Promise<ApprovalPreview | null>;
  cutNote: string;
  waiting?: boolean;
  lead?: ReactNode;
  onPlan: (approve: boolean, feedback?: string) => Promise<void>;
  onReject: () => Promise<void>;
}) {
  const preview = useWholePreview(requestId, null, fetch, true);
  const plan = preview?.kind === "plan" ? preview : null;
  const [feedback, setFeedback] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const disabled = busy || !waiting;

  const run = (call: () => Promise<void>) => {
    if (disabled) return;
    setBusy(true);
    setError(null);
    call().catch((err: unknown) => {
      setError(errorMessage(err));
      setBusy(false);
    });
  };
  const sendBack = () => feedback.trim() && run(() => onPlan(false, feedback.trim()));
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      sendBack();
    }
  };

  return (
    <div className="iplan">
      <div className="iplan__doc">
        {plan ? <Markdown source={plan.markdown} /> : <p className="iplan__empty">Loading the plan…</p>}
      </div>
      {plan?.cut && <p className="iplan__cut">{cutNote}</p>}
      <textarea
        className="iplan__feedback"
        rows={2}
        placeholder="What should change? Claude keeps planning with your words."
        value={feedback}
        spellCheck={false}
        disabled={disabled}
        onChange={(e) => setFeedback(e.target.value)}
        onKeyDown={onKey}
      />
      <Footer lead={lead} error={error} waiting={waiting}>
        <button className="agent-link" disabled={disabled} onClick={() => run(onReject)}>
          Reject
        </button>
        <button className="agent-btn agent-btn--quiet" disabled={disabled || !feedback.trim()} onClick={sendBack}>
          Send feedback
        </button>
        <button className="agent-btn agent-btn--allow" disabled={disabled} onClick={() => run(() => onPlan(true))}>
          Approve
        </button>
      </Footer>
    </div>
  );
}

/** `lead`, then the error (or room), then the choices; once it's no longer waiting, one quiet line instead. */
function Footer({
  lead,
  error,
  waiting,
  children,
}: {
  lead?: ReactNode;
  error: string | null;
  waiting: boolean;
  children: ReactNode;
}) {
  return (
    <div className="ask__footer">
      {lead}
      {!waiting ? (
        <span className="ask__gone">No longer waiting: answered, or back in the terminal.</span>
      ) : (
        <>
          {error ? (
            <span className="cmd-error" role="alert" title={error}>
              {error}
            </span>
          ) : (
            <span className="cmd__spacer" />
          )}
          {children}
        </>
      )}
    </div>
  );
}
