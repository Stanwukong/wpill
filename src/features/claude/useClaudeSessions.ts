import { useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../../lib/ipc";
import type { ClaudeSession } from "../../lib/types";

/**
 * Local Claude Code sessions as tracked by Rust (most recently updated first). Lives at the Island level so
 * state survives the panel remounting on every expand.
 */
export function useClaudeSessions(): ClaudeSession[] {
  const [sessions, setSessions] = useState<ClaudeSession[]>([]);

  useEffect(() => {
    let alive = true;
    commands
      .getClaudeSessions()
      .then((s) => alive && setSessions(s.sessions))
      .catch(() => {});
    const subscriptions = [on.claudeSessions((s) => setSessions(s.sessions))];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return sessions;
}

/** Brings the session's terminal or editor forward. Rust acknowledges and hides its activity; failures stay quiet. */
export function focusClaudeSession(sessionId: number) {
  commands.claudeFocusSession(sessionId).catch((err: unknown) => console.debug("claude focus", err));
}
