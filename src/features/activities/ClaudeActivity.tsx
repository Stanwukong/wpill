import { ClaudeMark } from "../claude/ClaudeGlyphs";
import { focusClaudeSession } from "../claude/useClaudeSessions";

/**
 * A Claude Code session finished a turn or is waiting for the user. Interactive: Rust makes the island clickable
 * while the cursor is on it, and a click focuses the session's window (Rust then acknowledges and hides it).
 * The island's orange edge is driven by `data-claude` on `.island` (activities.css).
 */
export function ClaudeActivity({
  sessionId,
  project,
  detail,
  muted = false,
}: {
  sessionId: number;
  project: string;
  detail: string;
  muted?: boolean;
}) {
  return (
    <button className="activity activity--claude" onClick={() => focusClaudeSession(sessionId)}>
      <ClaudeMark />
      <span className="activity__text">
        <span className="activity__title">{project}</span>
        <span className="activity__detail" data-muted={muted}>
          {detail}
        </span>
      </span>
    </button>
  );
}
