import { useCallback, useEffect, useState } from "react";
import { commands, on, unlistenAll } from "../../lib/ipc";
import type { MediaSession } from "../../lib/types";

/** A manual pick from the switcher, remembered with the session that was on top when it was made. */
interface Pin {
  id: number;
  topId: number;
}

export interface MediaController {
  sessions: MediaSession[];
  /** The session the card and pill show: the pinned one, else the most recently active. */
  active: MediaSession | null;
  select: (id: number) => void;
}

/**
 * Media sessions from Rust's GSMTC watcher. Lives at the Island level so state survives the
 * panel remounting on every expand. A pin lasts until a different session becomes most recent.
 */
export function useMedia(): MediaController {
  const [sessions, setSessions] = useState<MediaSession[]>([]);
  const [pin, setPin] = useState<Pin | null>(null);

  useEffect(() => {
    let alive = true;
    commands
      .getMediaState()
      .then((s) => alive && setSessions(s.sessions))
      .catch(() => {});
    const subscriptions = [on.mediaState((s) => setSessions(s.sessions))];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  const top = sessions[0] ?? null;
  const pinned = pin && top && pin.topId === top.id ? sessions.find((s) => s.id === pin.id) : undefined;
  const active = pinned ?? top;

  const select = useCallback(
    (id: number) => setPin(top ? { id, topId: top.id } : null),
    [top],
  );

  return { sessions, active, select };
}
