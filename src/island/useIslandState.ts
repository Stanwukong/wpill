import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { IslandState } from "../lib/types";

/** Expanded/hidden state and the current activity, as decided by the Rust hover poller. */
export function useIslandState(): IslandState {
  const [state, setState] = useState<IslandState>({ expanded: false, hidden: false, activity: null });

  useEffect(() => {
    let alive = true;
    // Resync after a webview reload, when Rust may already be expanded.
    commands
      .getIslandState()
      .then((s) => alive && setState(s))
      .catch(() => {});

    const subscriptions = [
      // Rust ends any activity before expanding; clearing it here too keeps a lost hide harmless.
      on.hoverEnter(() => setState((s) => ({ ...s, expanded: true, activity: null }))),
      on.hoverLeave(() => setState((s) => ({ ...s, expanded: false }))),
      on.visibility(({ hidden }) =>
        setState((s) => (hidden ? { ...s, hidden, expanded: false, activity: null } : { ...s, hidden })),
      ),
      on.activityShow((activity) => setState((s) => ({ ...s, activity }))),
      on.activityHide(({ id }) => setState((s) => (s.activity?.id === id ? { ...s, activity: null } : s))),
    ];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return state;
}
