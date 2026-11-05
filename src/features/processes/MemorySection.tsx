import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import type { CSSProperties } from "react";
import type { HeavyProcesses, ProcessGroup } from "../../lib/types";
import { memoryReorder, memoryRowIn, memoryRowOut } from "../../island/motion";
import { formatMemory, type EndState } from "./useHeavyProcesses";
import "./processes.css";

/** Rows shown, biggest first. */
const MAX_ROWS = 6;

/**
 * Panel section (3a): the apps using the most memory. One row per group: icon, name, a hairline share of the biggest
 * row, the figure, and (3b) an End control that shows on hover, except for protected apps. Rows are keyed by group,
 * so a refresh (every 2 s, pushed by Rust) updates figures in place, a row that changes place glides there, and an
 * app that was ended fades out. The End state lives in `useHeavyProcesses` (Island), so it outlives the panel.
 */
export function MemorySection({
  processes,
  ends,
  onEnd,
}: {
  processes: HeavyProcesses;
  ends: Record<string, EndState>;
  onEnd: (group: ProcessGroup) => void;
}) {
  const still = useReducedMotion() ?? false;
  if (!processes.enabled || processes.groups.length === 0) return null;

  const rows = [...processes.groups].sort((a, b) => b.memoryBytes - a.memoryBytes).slice(0, MAX_ROWS);
  const biggest = rows[0].memoryBytes || 1;
  const pending = Object.values(ends).some((s) => s.phase === "pending");

  return (
    <section className="memory" aria-label="Memory">
      <h2 className="memory__heading">Memory</h2>
      <ul className="memory__list">
        <AnimatePresence initial={!still}>
          {rows.map((group) => (
            <motion.li
              key={group.key}
              layout={still ? false : "position"}
              initial={still ? false : { opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={still ? undefined : { opacity: 0, transition: memoryRowOut }}
              transition={{ layout: memoryReorder, opacity: memoryRowIn }}
            >
              <MemoryRow
                group={group}
                share={group.memoryBytes / biggest}
                end={ends[group.key]}
                canEnd={!pending}
                onEnd={onEnd}
              />
            </motion.li>
          ))}
        </AnimatePresence>
      </ul>
    </section>
  );
}

function MemoryRow({
  group,
  share,
  end,
  canEnd,
  onEnd,
}: {
  group: ProcessGroup;
  share: number;
  end: EndState | undefined;
  canEnd: boolean;
  onEnd: (group: ProcessGroup) => void;
}) {
  const count = group.pids.length;
  const ending = end?.phase === "pending";
  const message = end?.phase === "done" ? end.message : null;
  return (
    <div
      className="memory-row"
      title={count > 1 ? `${group.name} · ${count} processes` : group.name}
      data-protected={group.protected || undefined}
      data-ending={ending || undefined}
    >
      {group.icon ? (
        <img className="memory-row__icon" src={group.icon} alt="" draggable={false} />
      ) : (
        <span className="memory-row__icon memory-row__icon--fallback" aria-hidden />
      )}
      <span className="memory-row__name">{group.name}</span>
      {message !== null ? (
        <span className="memory-row__note" title={message}>
          {message}
        </span>
      ) : (
        <>
          <span className="memory-row__share" aria-hidden>
            <span className="memory-row__fill" style={{ "--share": share } as CSSProperties} />
          </span>
          <span className="memory-row__size">{ending ? "Ending…" : formatMemory(group.memoryBytes)}</span>
        </>
      )}
      {/* The slot is always there, so the figure never moves when the control appears (or for protected apps). */}
      <span className="memory-row__end-slot">
        {!group.protected && (
          <button
            className="memory-row__end"
            aria-label={`End ${group.name}`}
            title={`End ${group.name}`}
            disabled={!canEnd}
            onClick={(e) => {
              e.stopPropagation();
              onEnd(group);
            }}
          >
            <EndGlyph />
          </button>
        )}
      </span>
    </div>
  );
}

/** A small ×: line-drawn like the panel's other glyphs. */
function EndGlyph() {
  return (
    <svg className="memory-row__end-glyph" viewBox="0 0 12 12" aria-hidden="true">
      <path d="M3 3l6 6M9 3l-6 6" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}
