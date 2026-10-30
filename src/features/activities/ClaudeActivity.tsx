import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { claudeCheckDraw, claudeDotsOut, claudeStatusIn } from "../../island/motion";
import { ClaudeMark } from "../claude/ClaudeGlyphs";
import { focusClaudeSession } from "../claude/useClaudeSessions";

export type ClaudeActivityState = "done" | "stopped" | "needsInput";

/**
 * A Claude Code session finished a turn or is waiting for the user: one row, ✻ · project · status, with the
 * state's mark at the right end (a check that draws in, or waiting dots). Interactive: Rust makes the island
 * clickable while the cursor is on it, and a click focuses the session's window (Rust then acknowledges and hides
 * it). The island keeps this element mounted when needs-input turns into done (same key), so only the status and
 * the end mark animate. The orange edge is driven by `data-claude` on `.island` (activities.css).
 */
export function ClaudeActivity({
  sessionId,
  project,
  state,
}: {
  sessionId: number;
  project: string;
  state: ClaudeActivityState;
}) {
  // Reduced motion: status and check appear in their end state (initial={false}); the dots stop in CSS.
  const still = useReducedMotion() ?? false;
  const status = state === "done" ? "Done" : state === "stopped" ? "Stopped" : null;

  return (
    <button className="activity activity--claude" data-state={state} onClick={() => focusClaudeSession(sessionId)}>
      <ClaudeMark />
      <span className="claude-activity__line">
        <span className="activity__title">{project}</span>
        {status && (
          // The clip makes the text emerge from behind the name rather than fly in.
          <span className="claude-activity__status">
            <motion.span
              key={status}
              initial={still ? false : { x: "-100%", opacity: 0 }}
              animate={{ x: 0, opacity: 1 }}
              transition={claudeStatusIn}
            >
              {status}
            </motion.span>
          </span>
        )}
      </span>
      <span className="claude-activity__end">
        <AnimatePresence initial={!still}>
          {state === "needsInput" && (
            <motion.span
              key="dots"
              className="claude-dots"
              role="img"
              aria-label="needs your input"
              exit={{ opacity: 0 }}
              transition={claudeDotsOut}
            >
              <span />
              <span />
              <span />
            </motion.span>
          )}
          {state === "done" && (
            <svg
              key="check"
              className="activity__icon claude-check"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth={1.8}
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              <motion.path
                d="M5.5 12.5l4.25 4.25L18.5 8"
                initial={still ? false : { pathLength: 0, opacity: 0 }}
                animate={{ pathLength: 1, opacity: 1 }}
                transition={claudeCheckDraw}
              />
            </svg>
          )}
        </AnimatePresence>
      </span>
    </button>
  );
}
