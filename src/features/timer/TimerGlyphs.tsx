/**
 * The timer's marks (Phase 5a). A countdown is a thin ring: what's left, from 12 o'clock clockwise, over a faint full
 * track. It is drawn from Rust's latest state (about once a second); a short transition lets +1 min move rather than
 * jump. A stopwatch has no end to measure, so it gets a steady line stopwatch instead.
 */
export function TimerRing({ fraction, size = 16 }: { fraction: number; size?: number }) {
  return (
    <svg className="timer-ring" width={size} height={size} viewBox="0 0 16 16" aria-hidden="true">
      <circle className="timer-ring__track" cx="8" cy="8" r="6.1" />
      <circle
        className="timer-ring__arc"
        cx="8"
        cy="8"
        r="6.1"
        pathLength={1}
        strokeDasharray="1 1"
        strokeDashoffset={1 - fraction}
        transform="rotate(-90 8 8)"
        data-empty={fraction <= 0}
      />
    </svg>
  );
}

/** A line stopwatch (icons.tsx style: 24 px grid, 1.8 stroke, round caps), shown at any size. */
export function StopwatchGlyph({ size = 16, className }: { size?: number; className?: string }) {
  return (
    <svg
      className={className ? `timer-glyph ${className}` : "timer-glyph"}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="13.5" r="7.5" />
      <path d="M10 3h4M12 3v3M12 13.5V10M18.5 6.5l1.2-1.2" />
    </svg>
  );
}
