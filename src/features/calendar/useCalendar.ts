import { useEffect, useState } from "react";
import { commands, errorMessage, on, unlistenAll } from "../../lib/ipc";
import type { CalendarUpcoming, GoogleStatus } from "../../lib/types";

export interface CalendarState {
  /** Signed in to Google (only then does anything calendar show). */
  connected: boolean;
  upcoming: CalendarUpcoming;
}

const EMPTY: CalendarUpcoming = { events: [], tasks: [], pending: false };

/**
 * Google Calendar and Tasks from Rust's watcher (Phase 6). Lives at the Island level, like `useMedia`, so the state
 * survives the panel remounting on every expand. Rust re-sends at every boundary (soon, start, end, midnight), so
 * phases and `pending` are always current here: nothing in React times them.
 */
export function useCalendar(): CalendarState {
  const [status, setStatus] = useState<GoogleStatus>("disconnected");
  const [upcoming, setUpcoming] = useState<CalendarUpcoming>(EMPTY);

  useEffect(() => {
    let alive = true;
    commands
      .googleState()
      .then((s) => alive && setStatus(s.status))
      .catch(() => {});
    commands
      .calendarUpcoming()
      .then((u) => alive && setUpcoming(u))
      .catch(() => {});
    const subscriptions = [on.googleState((s) => setStatus(s.status)), on.calendarUpcoming(setUpcoming)];
    return () => {
      alive = false;
      unlistenAll(subscriptions);
    };
  }, []);

  return { connected: status === "connected", upcoming };
}

/** Opens an event or task in the browser; resolves to an error message, or null. */
export function openCalendarItem(id: string): Promise<string | null> {
  return commands.calendarOpen(id).then(
    () => null,
    (err: unknown) => errorMessage(err),
  );
}

/** Opens an event's meeting link (only offered when `hasJoin`); resolves to an error message, or null. */
export function joinCalendarEvent(id: string): Promise<string | null> {
  return commands.calendarJoin(id).then(
    () => null,
    (err: unknown) => errorMessage(err),
  );
}
