import { useState } from "react";
import type { CalendarEvent, CalendarUpcoming, GoogleTask } from "../../lib/types";
import { joinCalendarEvent, openCalendarItem } from "./useCalendar";
import "./calendar.css";

const MAX_EVENTS = 4;
const MAX_TASKS = 3;

const timeFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dayFormat = new Intl.DateTimeFormat(undefined, { weekday: "short" });

/** "Now" / "Soon" from Rust's phase; a later event on another day shows its weekday. Worked out at render. */
function eventTag(event: CalendarEvent): string | null {
  if (event.phase === "now") return "Now";
  if (event.phase === "soon") return "Soon";
  const start = new Date(event.startMs);
  return start.toDateString() === new Date().toDateString() ? null : dayFormat.format(start);
}

/**
 * Panel section (6b): the next events (time, calendar colour, title, Now/Soon, Join) and today's or overdue tasks.
 * A row opens the item in the browser. Shown only while connected with something to list; Island decides that.
 */
export function CalendarSection({ upcoming }: { upcoming: CalendarUpcoming }) {
  const [error, setError] = useState<string | null>(null);
  const events = upcoming.events.slice(0, MAX_EVENTS);
  const tasks = upcoming.tasks.slice(0, MAX_TASKS);
  const more = upcoming.events.length - events.length + upcoming.tasks.length - tasks.length;
  const open = (id: string) => void openCalendarItem(id).then(setError);

  return (
    <section className="calendar" aria-label="Calendar">
      <h2 className="calendar__heading">Calendar</h2>
      <ul className="calendar__list">
        {events.map((e) => (
          <EventRow key={e.id} event={e} onOpen={open} onError={setError} />
        ))}
        {tasks.map((t) => (
          <TaskRow key={t.id} task={t} onOpen={open} />
        ))}
      </ul>
      {more > 0 && <p className="calendar__more">+{more} more</p>}
      {error && <p className="calendar__error">{error}</p>}
    </section>
  );
}

function EventRow({
  event,
  onOpen,
  onError,
}: {
  event: CalendarEvent;
  onOpen: (id: string) => void;
  onError: (message: string | null) => void;
}) {
  const tag = eventTag(event);
  return (
    <li>
      <div
        className="calendar-row"
        data-phase={event.phase}
        role="button"
        title={`${event.title} · ${event.calendar}`}
        onClick={() => onOpen(event.id)}
      >
        <span className="calendar-row__when">{timeFormat.format(event.startMs)}</span>
        <span className="calendar-row__mark">
          <span className="calendar-dot" style={event.color ? { background: event.color } : undefined} />
        </span>
        <span className="calendar-row__title">{event.title}</span>
        {tag && <span className="calendar-row__tag">{tag}</span>}
        {event.hasJoin && (
          <button
            className="calendar-join"
            onClick={(e) => {
              e.stopPropagation();
              void joinCalendarEvent(event.id).then(onError);
            }}
          >
            Join
          </button>
        )}
      </div>
    </li>
  );
}

function TaskRow({ task, onOpen }: { task: GoogleTask; onOpen: (id: string) => void }) {
  return (
    <li>
      <div
        className="calendar-row calendar-row--task"
        data-overdue={task.overdue}
        role="button"
        title={`${task.title} · ${task.list}`}
        onClick={() => onOpen(task.id)}
      >
        <span className="calendar-row__when">{task.overdue ? "Overdue" : "Today"}</span>
        <span className="calendar-row__mark">
          <span className="calendar-ring" />
        </span>
        <span className="calendar-row__title">{task.title}</span>
      </div>
    </li>
  );
}
