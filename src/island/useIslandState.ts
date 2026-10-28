import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { IslandState } from "../lib/types";

/** Expanded/hidden state as decided by the Rust hover poller. */
export function useIslandState(): IslandState {
  const [state, setState] = useState<IslandState>({ expanded: false, hidden: false });

  useEffect(() => {
    let alive = true;
    // Resync after a webview reload, when Rust may already be expanded.
    commands
      .getIslandState()
      .then((s) => alive && setState(s))
      .catch(() => {});

    const subscriptions = [
      on.hoverEnter(() => setState((s) => ({ ...s, expanded: true }))),
      on.hoverLeave(() => setState((s) => ({ ...s, expanded: false }))),
      on.visibility(({ hidden }) => setState((s) => ({ ...s, hidden, expanded: hidden ? false : s.expanded }))),
    ];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return state;
}
