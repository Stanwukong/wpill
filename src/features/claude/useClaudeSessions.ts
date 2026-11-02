import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../../lib/ipc";
import type { ClaudeSessions } from "../../lib/types";

const EMPTY: ClaudeSessions = { sessions: [], limits: null };

/**
 * Local Claude Code sessions as tracked by Rust (most recently updated first) and the newest usage limits (2.8f).
 * Lives at the Island level so state survives the panel remounting on every expand.
 */
export function useClaudeSessions(): ClaudeSessions {
  const [state, setState] = useState<ClaudeSessions>(EMPTY);

  useEffect(() => {
    let alive = true;
    commands
      .getClaudeSessions()
      .then((s) => alive && setState(s))
      .catch(() => {});
    const subscriptions = [on.claudeSessions(setState)];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return state;
}

/** Brings the session's terminal or editor forward. Rust acknowledges and hides its activity; failures stay quiet. */
export function focusClaudeSession(sessionId: number) {
  commands.claudeFocusSession(sessionId).catch((err: unknown) => console.debug("claude focus", err));
}

/** Answers a terminal session's permission request (2.8a). Rust withdraws the card; rejects once it's gone. */
export function decideClaude(requestId: number, answer: "allow" | "deny"): Promise<void> {
  return commands.claudeDecide(requestId, answer);
}
