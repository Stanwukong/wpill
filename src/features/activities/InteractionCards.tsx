import { useState } from "react";
import { commands } from "../../lib/ipc";
import type {
  AgentApprovalActivity,
  AgentRun,
  SessionApprovalActivity,
  Interaction,
  QuestionAnswer,
  QuestionView,
} from "../../lib/types";
import { ChevronGlyph } from "../agent/AgentGlyphs";
import { ModeChip } from "../agent/ModeChip";
import { decide, openRun } from "../agent/useAgentRuns";
import { AgentTag } from "../sessions/AgentTag";
import { decideSession, focusSession } from "../sessions/useSessions";
import { ApprovalHead } from "./AgentApproval";

type Question = Extract<Interaction, { kind: "question" }>;
type Plan = Extract<Interaction, { kind: "plan" }>;
type HeadProps = Parameters<typeof ApprovalHead>[0];

/** A run's question or plan (2.8c), in place of its approval card. Never Allow: Rust refuses it for these. */
export function AgentInteraction({
  approval,
  interaction,
  run,
}: {
  approval: AgentApprovalActivity;
  interaction: Interaction;
  /** Its run, for the mode chip (2.8e): questions and plans reach you in every mode. */
  run?: AgentRun;
}) {
  const { requestId, runId, project, routineName, risk } = approval;
  const open = () => openRun(runId);
  const head = { title: routineName ?? project, risk, mode: run && <ModeChip run={run} /> };
  return interaction.kind === "question" ? (
    <QuestionCard
      head={{ ...head, tool: approval.tool }}
      interaction={interaction}
      onAnswer={(answer) => commands.agentAnswer(requestId, answer)}
      onType={open}
      onSkip={() => decide(requestId, "deny", "once")}
    />
  ) : (
    <PlanCard head={head} plan={interaction} onOpen={open} />
  );
}

/** A terminal session's question or plan: the same cards with "session", and the title showing the terminal. */
export function SessionInteraction({
  approval,
  interaction,
  mixed,
}: {
  approval: SessionApprovalActivity;
  interaction: Interaction;
  /** Some session isn't Claude Code's: the head names this one's agent (2.9a). */
  mixed: boolean;
}) {
  const { requestId, sessionId, agent, project, risk } = approval;
  const open = () =>
    commands.commandWindowShowAsk(requestId).catch((err: unknown) => console.debug("command window", err));
  const head = {
    title: project,
    titleHint: "Show session",
    onTitle: sessionId === null ? undefined : () => focusSession(sessionId),
    tag: <AgentTag agent={agent} mixed={mixed} />,
    origin: "session",
    risk,
  };
  return interaction.kind === "question" ? (
    <QuestionCard
      head={{ ...head, tool: approval.tool }}
      interaction={interaction}
      onAnswer={(answer) => commands.sessionAnswer(requestId, answer)}
      onType={open}
      onSkip={() => decideSession(requestId, "deny")}
    />
  ) : (
    <PlanCard head={head} plan={interaction} onOpen={open} />
  );
}

/** The question the card asks: the first one without an answer. */
export function currentQuestion(interaction: Question): QuestionView | undefined {
  return interaction.questions.find((q) => !(q.question in interaction.answers));
}

/** Option chips that won't sit comfortably in the 340 px card take the wide one (`data-wide`, island.css). */
export function questionNeedsRoom(interaction: Interaction): boolean {
  if (interaction.kind !== "question") return false;
  const q = currentQuestion(interaction);
  if (!q) return false;
  const chars = q.options.reduce((n, o) => n + o.label.length + 4, 0);
  return chars > 40 || q.question.length > 110;
}

/**
 * A question from Claude (2.8c): the approval head with `n/m` when there are several, then the first unanswered one:
 * header chip and text, its options as chips. A single choice answers at once; multiple choices toggle and Done
 * sends them. Rust re-sends the card with the answer recorded (the next question shows in place) and withdraws it
 * once all are answered, so a click only marks the choice as pending. Interactive while hovered (Rust's hit test).
 */
function QuestionCard({
  head,
  interaction,
  onAnswer,
  onType,
  onSkip,
}: {
  head: HeadProps;
  interaction: Question;
  onAnswer: (answer: QuestionAnswer) => Promise<void>;
  onType: () => void;
  onSkip: () => Promise<void>;
}) {
  const total = interaction.questions.length;
  const answered = Object.keys(interaction.answers).length;
  const question = currentQuestion(interaction);

  return (
    <div className="activity activity--approval activity--question">
      <ApprovalHead {...head} count={total > 1 ? `${Math.min(answered + 1, total)}/${total}` : undefined} />
      {/* Keyed by question: the next one starts with nothing chosen. */}
      {question && (
        <QuestionBody key={question.question} question={question} onAnswer={onAnswer} onType={onType} onSkip={onSkip} />
      )}
    </div>
  );
}

function QuestionBody({
  question,
  onAnswer,
  onType,
  onSkip,
}: {
  question: QuestionView;
  onAnswer: (answer: QuestionAnswer) => Promise<void>;
  onType: () => void;
  onSkip: () => Promise<void>;
}) {
  const { header, multiSelect, options } = question;
  const [picked, setPicked] = useState<string[]>([]);
  /** What was sent: an option (single), "done" (multi) or "skip"; cleared if the call fails. */
  const [sent, setSent] = useState<string | null>(null);
  const busy = sent !== null;

  const send = (what: string, call: () => Promise<void>) => {
    setSent(what);
    call().catch((err: unknown) => {
      console.debug("agent answer", err);
      setSent(null);
    });
  };
  const answer = (labels: string[]) => onAnswer({ question: question.question, labels, other: null });
  const choose = (label: string) => {
    if (!multiSelect) send(label, () => answer([label]));
    else setPicked((p) => (p.includes(label) ? p.filter((l) => l !== label) : [...p, label]));
  };

  return (
    <>
      <div className="question">
        <p className="question__text" title={question.question}>
          {header && <span className="question__header">{header}</span>}
          {question.question}
        </p>
        <div className="question__options" role="group" aria-label={header || "Options"}>
          {options.map((o) => {
            const on = multiSelect ? picked.includes(o.label) : sent === o.label;
            return (
              <button
                key={o.label}
                className="question__option"
                title={o.description || undefined}
                aria-pressed={multiSelect ? on : undefined}
                data-on={on}
                data-chosen={on}
                data-multi={multiSelect}
                disabled={busy}
                onClick={() => choose(o.label)}
              >
                {multiSelect && <span className="question__box" aria-hidden="true" />}
                <span className="question__label">{o.label}</span>
              </button>
            );
          })}
        </div>
      </div>
      <div className="approval__actions">
        <button className="agent-link" disabled={busy} onClick={onType}>
          Type an answer…
        </button>
        <button
          className="agent-link"
          disabled={busy}
          data-chosen={sent === "skip"}
          onClick={() => send("skip", onSkip)}
        >
          Skip
        </button>
        {multiSelect && (
          <span className="approval__buttons">
            <button
              className="agent-btn agent-btn--allow"
              disabled={busy || picked.length === 0}
              data-chosen={sent === "done"}
              onClick={() => send("done", () => answer(picked))}
            >
              Done
            </button>
          </span>
        )}
      </div>
    </>
  );
}

/**
 * Claude's plan is ready (2.8c): the head, then one well (title · line count · chevron) that opens the plan in the
 * command window, where it's approved or sent back. Nothing to decide here.
 */
function PlanCard({ head, plan, onOpen }: { head: Omit<HeadProps, "tool">; plan: Plan; onOpen: () => void }) {
  const lines = `${plan.lines} ${plan.lines === 1 ? "line" : "lines"}`;
  return (
    <div className="activity activity--approval activity--plan">
      <ApprovalHead {...head} risk={undefined} tool="Plan ready" />
      <button className="approval__summary" title={`${plan.title}\nOpen the plan`} onClick={onOpen}>
        <span className="approval__command">{plan.title || "Plan"}</span>
        <span className="plan-card__lines">{lines}</span>
        <ChevronGlyph />
      </button>
    </div>
  );
}
