import { motion, useReducedMotion } from "framer-motion";
import type { CSSProperties } from "react";
import type { HeavyProcesses, ProcessGroup } from "../../lib/types";
import { memoryReorder, memoryRowIn } from "../../island/motion";
import { formatMemory } from "./useHeavyProcesses";
import "./processes.css";

/** Rows shown, biggest first. */
const MAX_ROWS = 6;

/**
 * Panel section (3a): the apps using the most memory, read-only. One row per group: icon, name, a hairline share of
 * the biggest row, and the figure. Rows are keyed by group, so a refresh (every 2 s, pushed by Rust) updates figures
 * in place and a row that changes place glides there. Off, or before the first push: nothing.
 */
export function MemorySection({ processes }: { processes: HeavyProcesses }) {
  const still = useReducedMotion() ?? false;
  if (!processes.enabled || processes.groups.length === 0) return null;

  const rows = [...processes.groups].sort((a, b) => b.memoryBytes - a.memoryBytes).slice(0, MAX_ROWS);
  const biggest = rows[0].memoryBytes || 1;

  return (
    <section className="memory" aria-label="Memory">
      <h2 className="memory__heading">Memory</h2>
      <ul className="memory__list">
        {rows.map((group) => (
          <motion.li
            key={group.key}
            layout={still ? false : "position"}
            initial={still ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ layout: memoryReorder, opacity: memoryRowIn }}
          >
            <MemoryRow group={group} share={group.memoryBytes / biggest} />
          </motion.li>
        ))}
      </ul>
    </section>
  );
}

function MemoryRow({ group, share }: { group: ProcessGroup; share: number }) {
  const count = group.pids.length;
  return (
    <div
      className="memory-row"
      title={count > 1 ? `${group.name} · ${count} processes` : group.name}
      data-protected={group.protected || undefined}
    >
      {group.icon ? (
        <img className="memory-row__icon" src={group.icon} alt="" draggable={false} />
      ) : (
        <span className="memory-row__icon memory-row__icon--fallback" aria-hidden />
      )}
      <span className="memory-row__name">{group.name}</span>
      <span className="memory-row__share" aria-hidden>
        <span className="memory-row__fill" style={{ "--share": share } as CSSProperties} />
      </span>
      <span className="memory-row__size">{formatMemory(group.memoryBytes)}</span>
    </div>
  );
}
