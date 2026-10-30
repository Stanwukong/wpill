import type { ClaudeSession, ClaudeStatus } from "../../lib/types";
import { ClaudeSpinner } from "./ClaudeGlyphs";
import { focusClaudeSession } from "./useClaudeSessions";
import "./claude.css";

const STATUS: Record<ClaudeStatus, string> = {
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

/** Panel section: one row per local Claude Code session; a click focuses its terminal or editor. */
export function ClaudeSection({ sessions }: { sessions: ClaudeSession[] }) {
  if (sessions.length === 0) return null;
  return (
    <section className="claude" aria-label="Claude Code">
      <h2 className="claude__heading">Claude Code</h2>
      <ul className="claude__list">
        {sessions.map((s) => (
          <li key={s.sessionId}>
            <button className="claude-row" data-status={s.status} onClick={() => focusClaudeSession(s.sessionId)}>
              <span className="claude-row__mark">
                {s.status === "working" ? <ClaudeSpinner /> : <span className="claude-row__dot" />}
              </span>
              <span className="claude-row__project">{s.project}</span>
              <span className="claude-row__status">{STATUS[s.status]}</span>
              <span className="claude-row__time">{ago(s.updatedAtMs)}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
