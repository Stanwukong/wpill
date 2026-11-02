import { useState } from "react";
import type { CalendarSoonActivity } from "../../lib/types";
import { CalendarMark } from "./CalendarMark";
import { joinCalendarEvent, openCalendarItem } from "./useCalendar";
import "./calendar.css";

/**
 * "Starting soon" (6b): one row, mark · title · "in 5 min", and Join at the right when the event has a meeting link.
 * Interactive while hovered (Rust); the row opens the event, Join opens the meeting. Either makes Rust retract the
 * activity. `minutes` is Rust's number from when it was offered: shown as-is, never recounted.
 */
export function CalendarSoon({ soon }: { soon: CalendarSoonActivity }) {
  const [error, setError] = useState<string | null>(null);
  const when = soon.minutes <= 0 ? "now" : `in ${soon.minutes} min`;
  return (
    <div
      className="activity calendar-activity"
      role="button"
      title={soon.title}
      onClick={() => void openCalendarItem(soon.eventId).then(setError)}
    >
      <span className="calendar-activity__mark">
        <CalendarMark size={20} />
      </span>
      {soon.color && <span className="calendar-dot" style={{ background: soon.color }} />}
      <span className="activity__title calendar-activity__title">{soon.title}</span>
      {error ? (
        <span className="calendar-activity__error">{error}</span>
      ) : (
        <span className="calendar-activity__when">{when}</span>
      )}
      {soon.hasJoin && (
        <button
          className="calendar-join calendar-join--primary"
          onClick={(e) => {
            e.stopPropagation();
            void joinCalendarEvent(soon.eventId).then(setError);
          }}
        >
          Join
        </button>
      )}
    </div>
  );
}
