import { useState } from "react";
import type { CalendarEvent, CalendarUpcoming, GoogleTask } from "../../lib/types";
import { joinCalendarEvent, openCalendarItem } from "./useCalendar";
import "./calendar.css";

/** Today's events, at most this many; with none left today, only tomorrow's first. */
const MAX_EVENTS = 3;

const timeFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const dayFormat = new Intl.DateTimeFormat(undefined, { weekday: "short" });

/** "Now" / "Soon" from Rust's phase; a later event on another day shows its weekday. Worked out at render. */
function eventTag(event: CalendarEvent): string | null {
  if (event.phase === "now") return "Now";
  if (event.phase === "soon") return "Soon";
  const start = new Date(event.startMs);
  return start.toDateString() === new Date().toDateString() ? null : dayFormat.format(start);
}

/** On now or starting soon, or later today (Rust's list covers the next 24 h, so the rest are tomorrow's). */
function isToday(event: CalendarEvent): boolean {
  return event.phase !== "later" || new Date(event.startMs).toDateString() === new Date().toDateString();
}

/** "2 tasks due", "1 task overdue", "3 tasks due · 1 overdue". */
function taskSummary(tasks: GoogleTask[]): string {
  const overdue = tasks.filter((t) => t.overdue).length;
  const due = tasks.length - overdue;
  const plural = (n: number) => (n === 1 ? "task" : "tasks");
  if (due === 0) return `${overdue} ${plural(overdue)} overdue`;
  return overdue === 0 ? `${due} ${plural(due)} due` : `${due} ${plural(due)} due · ${overdue} overdue`;
}

/**
 * Panel section (6b): up to three of today's events (time, calendar colour, title, Now/Soon, Join while soon or on
 * now), or tomorrow's first once today is over, then the due tasks as one line. A row opens the item in the browser;
 * the tasks line opens the first task. Shown only while connected with something to list; Island decides that.
 * Worked out at render: Rust re-sends at every boundary (soon, start, end, midnight).
 */
export function CalendarSection({ upcoming }: { upcoming: CalendarUpcoming }) {
  const [error, setError] = useState<string | null>(null);
  const today = upcoming.events.filter(isToday);
  const pool = today.length > 0 ? today : upcoming.events;
  const events = pool.slice(0, today.length > 0 ? MAX_EVENTS : 1);
  const more = pool.length - events.length;
  const open = (id: string) => void openCalendarItem(id).then(setError);

  return (
    <section className="calendar" aria-label="Calendar">
      <h2 className="calendar__heading">Calendar</h2>
      <ul className="calendar__list">
        {events.map((e) => (
          <EventRow key={e.id} event={e} onOpen={open} onError={setError} />
        ))}
        {more > 0 && <li className="calendar__more">+{more} more</li>}
        {upcoming.tasks.length > 0 && <TasksRow tasks={upcoming.tasks} onOpen={open} />}
      </ul>
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
        {event.hasJoin && event.phase !== "later" && (
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

/** All due tasks as one line: the counts, then the first task's title, muted. Opens that first task. */
function TasksRow({ tasks, onOpen }: { tasks: GoogleTask[]; onOpen: (id: string) => void }) {
  const first = tasks[0];
  return (
    <li>
      <div
        className="calendar-row calendar-row--tasks"
        role="button"
        title={`${first.title} · ${first.list}`}
        onClick={() => onOpen(first.id)}
      >
        <span className="calendar-row__when" />
        <span className="calendar-row__mark">
          <span className="calendar-ring" />
        </span>
        <span className="calendar-row__count">{taskSummary(tasks)}</span>
        <span className="calendar-row__title">{first.title}</span>
      </div>
    </li>
  );
}
