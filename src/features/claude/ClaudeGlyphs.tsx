import "./claude.css";

/**
 * Claude Code's working spinner, frame-stepped in CSS (claude.css). Render it only while a session works:
 * it's absent, not paused, when nothing is running.
 */
export function ClaudeSpinner() {
  return <span className="claude-spinner" role="img" aria-label="Claude Code is working" />;
}

/** The static ✻ Claude Code prints when a turn ends; the icon of the Claude activities. */
export function ClaudeMark() {
  return (
    <span className="claude-mark" aria-hidden="true">
      {"✻"}
    </span>
  );
}
