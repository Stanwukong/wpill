import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { sessionCheckDraw, sessionDotsOut, sessionStatusIn } from "../../island/motion";
import { AgentMark } from "../sessions/WorkingGlyphs";

export type SessionActivityState = "done" | "stopped" | "needsInput";

/**
 * A Claude Code session (or an agent run) finished a turn or is waiting for the user: one row, mark · project ·
 * status, with the state's mark at the right end (a check that draws in, or waiting dots). Interactive: Rust makes
 * the island clickable while the cursor is on it; a click focuses the session's window or opens the run (Rust then
 * acknowledges and hides it). The island keeps this element mounted when needs-input turns into done (same key), so
 * only the status and the end mark animate. The amber edge is driven by `data-agent` on `.island` (activities.css).
 */
export function SessionActivity({
  project,
  state,
  label,
  onOpen,
}: {
  project: string;
  state: SessionActivityState;
  /** Overrides the "Done" / "Stopped" status text (e.g. "Turn limit reached"). */
  label?: string;
  onOpen: () => void;
}) {
  // Reduced motion: status and check appear in their end state (initial={false}); the dots stop in CSS.
  const still = useReducedMotion() ?? false;
  const status = state === "needsInput" ? null : (label ?? (state === "done" ? "Done" : "Stopped"));

  return (
    <button className="activity activity--session" data-state={state} onClick={onOpen}>
      <AgentMark />
      <span className="session-activity__line">
        <span className="activity__title">{project}</span>
        {status && (
          // The clip makes the text emerge from behind the name rather than fly in.
          <span className="session-activity__status">
            <motion.span
              key={status}
              initial={still ? false : { x: "-100%", opacity: 0 }}
              animate={{ x: 0, opacity: 1 }}
              transition={sessionStatusIn}
            >
              {status}
            </motion.span>
          </span>
        )}
      </span>
      <span className="session-activity__end">
        <AnimatePresence initial={!still}>
          {state === "needsInput" && (
            <motion.span
              key="dots"
              className="session-dots"
              role="img"
              aria-label="needs your input"
              exit={{ opacity: 0 }}
              transition={sessionDotsOut}
            >
              <span />
              <span />
              <span />
            </motion.span>
          )}
          {state === "done" && (
            <svg
              key="check"
              className="activity__icon session-check"
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
                transition={sessionCheckDraw}
              />
            </svg>
          )}
        </AnimatePresence>
      </span>
    </button>
  );
}
