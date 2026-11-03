import type { ReactNode } from "react";
import type { AgentSession, SessionStatus, UsageLimits } from "../../lib/types";
import { SessionModeTag } from "../agent/ModeChip";
import { StepLabel } from "./StepLabel";
import { hasLimits, UsageLine } from "./UsageLine";
import { WorkingIndicator } from "./WorkingGlyphs";
import { focusSession } from "./useSessions";
import "./sessions.css";

const STATUS: Record<SessionStatus, string> = {
  working: "Working",
  needsInput: "Needs your input",
  done: "Done",
  idle: "Idle",
};

/** "just now", "4 m ago", "2 h ago", "3 d ago". Computed at render; the panel re-renders on new events, nothing ticks. */
function ago(updatedAtMs: number): string {
  const minutes = Math.floor((Date.now() - updatedAtMs) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.floor(hours / 24)} d ago`;
}

/**
 * Panel section: one row per local Claude Code session; a click focuses its terminal or editor. A session's mode
 * shows as a read-only tag unless it's plain `default` (2.8e). `agent` holds the
 * agent runs' rows (`<li>`s, features/agent), listed first; null when there are none. `limits` (2.8f) shows
 * beside the heading; with no rows the section still shows for a live (not stale) reading, as just the heading line.
 */
export function AgentsSection({
  sessions,
  agent,
  limits,
}: {
  sessions: AgentSession[];
  agent?: ReactNode;
  limits: UsageLimits | null;
}) {
  const rows = sessions.length > 0 || !!agent;
  if (!rows && !hasLimits(limits, true)) return null;
  return (
    <section className="agents" aria-label="Claude Code">
      <div className="agents__head">
        <h2 className="agents__heading">Claude Code</h2>
        {hasLimits(limits) && <UsageLine limits={limits} />}
      </div>
      {rows && (
      <ul className="agents__list">
        {agent}
        {sessions.map((s) => (
          <li key={s.sessionId}>
            <button className="session-row" data-status={s.status} onClick={() => focusSession(s.sessionId)}>
              <span className="session-row__mark">
                {s.status === "working" ? <WorkingIndicator small /> : <span className="session-row__dot" />}
              </span>
              <span className="session-row__project">{s.project}</span>
              {s.status === "working" && (s.step || s.todos) ? (
                <StepLabel className="session-row__status session-row__step" text={s.step ?? STATUS.working} todos={s.todos} />
              ) : (
                <span className="session-row__status">{STATUS[s.status]}</span>
              )}
              <SessionModeTag mode={s.mode} />
              <span className="session-row__time">{ago(s.updatedAtMs)}</span>
            </button>
          </li>
        ))}
      </ul>
      )}
    </section>
  );
}
