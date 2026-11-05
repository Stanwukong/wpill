import { useCallback, useEffect, useRef, useState } from "react";
import { commands, on, unlistenAll } from "../../lib/ipc";
import type { AppError, EndResult, HeavyProcesses, ProcessGroup, ProcessId } from "../../lib/types";

const EMPTY: HeavyProcesses = { enabled: false, groups: [] };

/** A row's End state (3b), by group key. No entry: normal. */
export type EndState = { phase: "pending" } | { phase: "done"; message: string; pids: string };

export interface HeavyProcessesState {
  processes: HeavyProcesses;
  ends: Record<string, EndState>;
  /** Asks Rust to end a group (it confirms in a native box first). Ignored while another End is pending. */
  end: (group: ProcessGroup) => void;
}

/**
 * The memory-heavy apps from Rust's `processes-watcher` (Phase 3). Lives at the Island level, like `useCalendar`, so
 * the last list and any End in flight survive the panel remounting (it can collapse while the confirm box is up).
 * Rust refreshes only while the panel is open and pushes every 2 s; nothing here runs a timer. A row's message stays
 * until its group leaves the list, comes back with other pids, or is ended again.
 */
export function useHeavyProcesses(): HeavyProcessesState {
  const [processes, setProcesses] = useState<HeavyProcesses>(EMPTY);
  const [ends, setEnds] = useState<Record<string, EndState>>({});
  const latest = useRef<HeavyProcesses>(EMPTY);
  const busy = useRef(false);

  useEffect(() => {
    let alive = true;
    const receive = (p: HeavyProcesses) => {
      if (!alive) return;
      latest.current = p;
      setProcesses(p);
      setEnds((prev) => prune(prev, p.groups));
    };
    commands
      .listHeavyProcesses()
      .then(receive)
      .catch(() => {});
    const subscriptions = [on.processesHeavy(receive)];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  const end = useCallback((group: ProcessGroup) => {
    if (busy.current || group.protected) return;
    busy.current = true;
    const key = group.key;
    setEnds((prev) => ({ ...prev, [key]: { phase: "pending" } }));
    const settle = (message: string | null) => {
      busy.current = false;
      // Rust refreshes right after ending, so the push can beat the reply: a group that's gone keeps no message.
      const now = latest.current.groups.find((g) => g.key === key);
      setEnds((prev) => {
        const next = { ...prev };
        if (message === null || !now) delete next[key];
        else next[key] = { phase: "done", message, pids: pidSignature(now.pids) };
        return next;
      });
    };
    commands
      .endProcessGroup(key, group.pids)
      .then((result) => settle(endMessage(result)))
      .catch((e: AppError) => settle(e?.kind === "processGroupGone" ? "No longer running" : "Couldn't end it"));
  }, []);

  return { processes, ends, end };
}

/** Drops messages for groups that left the list or came back with a different set of processes. */
function prune(ends: Record<string, EndState>, groups: ProcessGroup[]): Record<string, EndState> {
  let changed = false;
  const next: Record<string, EndState> = {};
  for (const [key, state] of Object.entries(ends)) {
    if (state.phase === "pending") {
      next[key] = state;
      continue;
    }
    const group = groups.find((g) => g.key === key);
    if (group && pidSignature(group.pids) === state.pids) next[key] = state;
    else changed = true;
  }
  return changed ? next : ends;
}

function pidSignature(pids: ProcessId[]): string {
  return pids
    .map((p) => `${p.pid}:${p.created}`)
    .sort()
    .join(",");
}

/** The row's line for an End reply; null when nothing happened (Cancel). */
export function endMessage(result: EndResult): string | null {
  if (result.cancelled) return null;
  const outcomes = result.outcomes.map((o) => o.outcome);
  if (outcomes.length === 0) return "Couldn't end it";
  if (outcomes.every((o) => o === "ended" || o === "closed" || o === "notFound")) return "Ended";
  if (outcomes.includes("prompted")) return "Left open: it's asking you something";
  if (outcomes.includes("accessDenied")) return "Runs as administrator; Cay can't end it";
  if (outcomes.includes("protected")) return "Cay never ends this";
  return "Couldn't end it";
}

/** 1.2 GB, 340 MB: Task Manager's units, binary like its figures. */
export function formatMemory(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${Math.max(1, Math.round(mb))} MB`;
}
