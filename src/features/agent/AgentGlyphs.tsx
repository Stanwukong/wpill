// Small glyphs for the agent UI (island panel and command window). They inherit `currentColor`.
import type { AgentStatus } from "../../lib/types";

/** Stop: a rounded square, like a media stop button. */
export function StopGlyph() {
  return (
    <svg className="agent-glyph" viewBox="0 0 16 16" aria-hidden="true">
      <rect x="4" y="4" width="8" height="8" rx="1.5" fill="currentColor" />
    </svg>
  );
}

/** "Open in the command window". */
export function ChevronGlyph() {
  return (
    <svg
      className="agent-glyph"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M6.5 4l4 4-4 4" />
    </svg>
  );
}

/** The run's state in a 14 px slot: Claude's spinner while it works, a breathing dot while it waits, a dot after. */
export function RunDot({ status }: { status: AgentStatus }) {
  return <span className="agent-dot" data-status={status} />;
}
