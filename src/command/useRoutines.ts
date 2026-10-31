import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { Routines } from "../lib/types";

/** Routines as Rust keeps them: the list on load, then every `routines:changed` (save, delete, runs, skips, pause). */
export function useRoutines(): Routines {
  const [routines, setRoutines] = useState<Routines>({ routines: [], paused: false });

  useEffect(() => {
    let alive = true;
    commands
      .routinesList()
      .then((r) => alive && setRoutines(r))
      .catch((err: unknown) => console.debug("routines", err));
    const subscriptions = [on.routinesChanged(setRoutines)];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return routines;
}
