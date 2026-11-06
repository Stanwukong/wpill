import { useEffect, useState } from "react";
import { commands, errorMessage, on, unlistenAll } from "../../lib/ipc";
import type { TimerPayload, TimerState } from "../../lib/types";

/**
 * The timer from Rust (Phase 5a). Lives at the Island level, like `useMedia`, so it survives the panel remounting.
 * Rust sends `timer:state` on every change and once a second while one runs; React renders the latest state and never
 * counts on its own (no interval: WebView2 throttles them, and Rust owns time).
 */
export function useTimer(): TimerState | null {
  const [timer, setTimer] = useState<TimerState | null>(null);

  useEffect(() => {
    let alive = true;
    let heard = false;
    commands
      .getTimer()
      .then((p) => alive && !heard && setTimer(p.timer))
      .catch(() => {});
    const subscriptions = [
      on.timerState((p) => {
        heard = true;
        setTimer(p.timer);
      }),
    ];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return timer;
}

/** Runs one timer command; resolves to an error message, or null. Rust's `timer:state` carries the new state. */
export function timerAction(call: () => Promise<TimerPayload>): Promise<string | null> {
  return call().then(
    () => null,
    (err: unknown) => errorMessage(err),
  );
}
