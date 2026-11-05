import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../lib/ipc";
import type { ApprovalPreview, SessionApprovalActivity } from "../lib/types";

export interface SessionAsksState {
  /** Held session requests, oldest first. */
  asks: SessionApprovalActivity[];
  /** The first list has arrived (until then a selected ask may simply not be known yet). */
  loaded: boolean;
}

/** Terminal sessions' requests the island holds (2.8b): the list on load, then every `sessions:asks`. */
export function useSessionAsks(): SessionAsksState {
  const [state, setState] = useState<SessionAsksState>({ asks: [], loaded: false });

  useEffect(() => {
    let alive = true;
    commands
      .sessionAsks()
      .then((a) => alive && setState({ asks: a.asks, loaded: true }))
      .catch((err: unknown) => {
        console.debug("session asks", err);
        // Nothing more to wait for: the next `sessions:asks` fills the list.
        if (alive) setState((s) => ({ ...s, loaded: true }));
      });
    const subscriptions = [on.sessionAsks((a) => setState({ asks: a.asks, loaded: true }))];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return state;
}

/**
 * The whole diff or command for an approval whose card has a preview: fetched once per request, the card's copy
 * until it arrives (or if Rust returns `null` because it was answered meanwhile). `fetch` must be stable. A plan
 * (2.8c) is never on the card: `always` fetches it anyway.
 */
export function useWholePreview(
  requestId: number,
  card: ApprovalPreview | null,
  fetch: (requestId: number) => Promise<ApprovalPreview | null>,
  always = false,
): ApprovalPreview | null {
  const [whole, setWhole] = useState<{ requestId: number; preview: ApprovalPreview } | null>(null);
  const hasPreview = always || card !== null;

  useEffect(() => {
    if (!hasPreview) return;
    let alive = true;
    fetch(requestId)
      .then((preview) => alive && preview && setWhole({ requestId, preview }))
      .catch((err: unknown) => console.debug("approval preview", err));
    return () => {
      alive = false;
    };
  }, [requestId, hasPreview, fetch]);

  return whole?.requestId === requestId ? whole.preview : card;
}
