import "./sessions.css";

/**
 * The working wave's two sizes. Each path is a sine one wavelength longer than its window, built from cubic half-waves
 * (controls at 0.4244 of the half-width and 4/3 of the amplitude, which matches the sine's slope and peak). The CSS
 * slides it one wavelength to the right per cycle (sessions.css), so it reads as a continuous ripple.
 */
const WAVES = {
  pill: {
    className: "working",
    run: 32,
    height: 10,
    stroke: 1.6,
    d: "M0 5C1.7 1.8 2.3 1.8 4 5C5.7 8.2 6.3 8.2 8 5C9.7 1.8 10.3 1.8 12 5C13.7 8.2 14.3 8.2 16 5C17.7 1.8 18.3 1.8 20 5C21.7 8.2 22.3 8.2 24 5C25.7 1.8 26.3 1.8 28 5C29.7 8.2 30.3 8.2 32 5",
  },
  small: {
    className: "working working--small",
    run: 21,
    height: 6,
    stroke: 1.5,
    d: "M0 3C1.27 1 1.73 1 3 3C4.27 5 4.73 5 6 3C7.27 1 7.73 1 9 3C10.27 5 10.73 5 12 3C13.27 1 13.73 1 15 3C16.27 5 16.73 5 18 3C19.27 1 19.73 1 21 3",
  },
};

/**
 * An amber wave rippling while a Claude Code session or an agent run works (pill, panel rows, command window). Render
 * it only while something works: it's absent, not paused, when nothing is running. `small` is the 14 × 6 row size.
 */
export function WorkingIndicator({ small = false }: { small?: boolean }) {
  const wave = small ? WAVES.small : WAVES.pill;
  return (
    <span className={wave.className} role="img" aria-label="Working">
      <svg className="working__run" width={wave.run} height={wave.height} viewBox={`0 0 ${wave.run} ${wave.height}`} aria-hidden="true">
        <path d={wave.d} strokeWidth={wave.stroke} />
      </svg>
    </span>
  );
}

/** The wave at rest: the icon of the Claude activities, the approval card and the command window. */
export function AgentMark() {
  return (
    <svg
      className="agent-mark"
      viewBox="0 0 22 22"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      aria-hidden="true"
    >
      <path d="M2 11C3.91 6.73 4.59 6.73 6.5 11C8.41 15.27 9.09 15.27 11 11C12.91 6.73 13.59 6.73 15.5 11C17.41 15.27 18.09 15.27 20 11" />
    </svg>
  );
}
