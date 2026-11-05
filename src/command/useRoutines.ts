import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { Routines } from "../lib/types";

/**
 * Routines as Rust keeps them: the list on load, then every `routines:changed` (save, delete, runs, skips, pause).
 * `loaded` once the first list (or its failure) is in.
 */
export function useRoutines(): Routines & { loaded: boolean } {
  const [routines, setRoutines] = useState<Routines & { loaded: boolean }>({ routines: [], paused: false, loaded: false });
  const loaded = (r: Routines) => setRoutines({ ...r, loaded: true });

  useEffect(() => {
    let alive = true;
    commands
      .routinesList()
      .then((r) => alive && loaded(r))
      .catch((err: unknown) => {
        console.debug("routines", err);
        if (alive) setRoutines((r) => ({ ...r, loaded: true }));
      });
    const subscriptions = [on.routinesChanged(loaded)];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return routines;
}
