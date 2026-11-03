import type { TodoCount } from "../../lib/types";
import "./sessions.css";

/** Anything that reports a live step (sessions and agent runs, 2.8d). */
interface Stepping {
  step: string | null;
  todos: TodoCount | null;
  updatedAtMs: number;
}

/** The most recently updated of `items` that has a step, or null. Computed at render; Rust sends every change. */
export function latestStep<T extends Stepping>(items: T[]): T | null {
  let best: T | null = null;
  for (const item of items) {
    if (item.step !== null && (best === null || item.updatedAtMs > best.updatedAtMs)) best = item;
  }
  return best;
}

/**
 * What a session or run is doing ("Edit · src/hover/logic.rs"), ellipsized, plus its to-do progress ("3/7"). The
 * text is keyed, so a new step fades in over the old one (sessions.css); the count just swaps (tabular).
 */
export function StepLabel({ text, todos, className }: { text: string; todos: TodoCount | null; className?: string }) {
  return (
    <span className={className ? `step ${className}` : "step"} title={text}>
      <span key={text} className="step__text">
        {text}
      </span>
      {todos && todos.total > 0 && (
        <span className="step__todos">
          {todos.done}/{todos.total}
        </span>
      )}
    </span>
  );
}
