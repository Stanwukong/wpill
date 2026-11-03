import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../../lib/ipc";
import type { AgentSessions } from "../../lib/types";

const EMPTY: AgentSessions = { sessions: [], limits: null };

/**
 * Local Claude Code sessions as tracked by Rust (most recently updated first) and the newest usage limits (2.8f).
 * Lives at the Island level so state survives the panel remounting on every expand.
 */
export function useSessions(): AgentSessions {
  const [state, setState] = useState<AgentSessions>(EMPTY);

  useEffect(() => {
    let alive = true;
    commands
      .getSessions()
      .then((s) => alive && setState(s))
      .catch(() => {});
    const subscriptions = [on.agentSessions(setState)];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return state;
}

/** Brings the session's terminal or editor forward. Rust acknowledges and hides its activity; failures stay quiet. */
export function focusSession(sessionId: number) {
  commands.sessionFocus(sessionId).catch((err: unknown) => console.debug("session focus", err));
}

/** Answers a terminal session's permission request (2.8a). Rust withdraws the card; rejects once it's gone. */
export function decideSession(requestId: number, answer: "allow" | "deny"): Promise<void> {
  return commands.sessionDecide(requestId, answer);
}
