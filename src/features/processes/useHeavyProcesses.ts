import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../../lib/ipc";
import type { HeavyProcesses } from "../../lib/types";

const EMPTY: HeavyProcesses = { enabled: false, groups: [] };

/**
 * The memory-heavy apps from Rust's `processes-watcher` (Phase 3). Lives at the Island level, like `useCalendar`, so
 * the last list survives the panel remounting. Rust refreshes only while the panel is open and pushes every 2 s;
 * nothing here runs a timer.
 */
export function useHeavyProcesses(): HeavyProcesses {
  const [state, setState] = useState<HeavyProcesses>(EMPTY);

  useEffect(() => {
    let alive = true;
    commands
      .listHeavyProcesses()
      .then((p) => alive && setState(p))
      .catch(() => {});
    const subscriptions = [on.processesHeavy(setState)];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return state;
}

/** 1.2 GB, 340 MB: Task Manager's units, binary like its figures. */
export function formatMemory(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${Math.max(1, Math.round(mb))} MB`;
}
