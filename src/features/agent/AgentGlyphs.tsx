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

/** A desktop look (windows, screenshot, UI tree): an eye, sized to the risk label it sits in. */
export function EyeGlyph() {
  return (
    <svg
      className="agent-risk__eye"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M1.75 8C3.3 5.2 5.4 3.75 8 3.75S12.7 5.2 14.25 8C12.7 10.8 10.6 12.25 8 12.25S3.3 10.8 1.75 8z" />
      <circle cx="8" cy="8" r="2" />
    </svg>
  );
}

/** The run's state in a 14 px slot: Claude's spinner while it works, a breathing dot while it waits, a dot after. */
export function RunDot({ status }: { status: AgentStatus }) {
  return <span className="agent-dot" data-status={status} />;
}
