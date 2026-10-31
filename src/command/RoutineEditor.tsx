import { useEffect, useRef, useState, type KeyboardEvent, type MutableRefObject, type ReactNode } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentOptions, IfMissed, RoutineDraft, RoutineView, Schedule, Weekday } from "../lib/types";
import { ALL_DAYS, lastLine, scheduleLine, sortDays, WEEKDAYS, WORKDAYS } from "./routineLabels";
import { FolderField, LimitsField, useFolders, type Limits } from "./TaskFields";

type Kind = Schedule["kind"];

const KINDS: { kind: Kind; label: string }[] = [
  { kind: "off", label: "Off" },
  { kind: "days", label: "Days" },
  { kind: "every", label: "Every" },
  { kind: "start", label: "At start" },
];

/** Every schedule kind's fields, so switching kinds keeps what was typed. */
interface ScheduleFields {
  kind: Kind;
  time: string;
  days: Weekday[];
  hours: string;
  from: string;
}

function scheduleFields(schedule: Schedule): ScheduleFields {
  return {
    kind: schedule.kind,
    time: schedule.kind === "days" ? schedule.time : "09:00",
    days: schedule.kind === "days" ? schedule.days : WORKDAYS,
    hours: schedule.kind === "every" ? String(schedule.hours) : "4",
    from: schedule.kind === "every" ? schedule.from : "08:00",
  };
}

function toSchedule(f: ScheduleFields): Schedule {
  switch (f.kind) {
    case "off":
      return { kind: "off" };
    case "days":
      return { kind: "days", time: f.time, days: sortDays(f.days) };
    case "every":
      return { kind: "every", hours: Number.parseInt(f.hours, 10) || 0, from: f.from };
    case "start":
      return { kind: "start" };
  }
}

/**
 * A routine: name, prompt, folder, schedule and limits. Save (Ctrl+S or Ctrl+Enter) sends the draft to Rust, which
 * checks it; its message shows in the footer. Run now saves first if anything changed. A routine never approves
 * anything by itself: its runs ask like any other.
 */
export function RoutineEditor({
  options,
  initial,
  view,
  paused,
  picking,
  onSaved,
  onDeleted,
  onClose,
  onShowRun,
}: {
  options: AgentOptions;
  /** What the form starts from (a saved routine, or a new one prefilled from a task). */
  initial: RoutineDraft;
  /** The saved routine as Rust reports it now (status, next run); missing while it's new. */
  view: RoutineView | undefined;
  paused: boolean;
  picking: MutableRefObject<boolean>;
  onSaved: (id: string) => void;
  onDeleted: () => void;
  onClose: () => void;
  onShowRun: (runId: string) => void;
}) {
  const [id, setId] = useState(initial.id);
  const [name, setName] = useState(initial.name);
  const [prompt, setPrompt] = useState(initial.prompt);
  const { folders, cwd, setCwd, add } = useFolders(options, initial.cwd || null);
  const [limits, setLimits] = useState<Limits>({
    model: initial.model ?? "",
    maxTurns: String(initial.maxTurns),
    budget: String(initial.maxBudgetUsd),
    claudePath: initial.claudePath ?? options.defaultInstall ?? options.installs[0]?.path ?? "",
  });
  const [schedule, setSchedule] = useState(() => scheduleFields(initial.schedule));
  const [enabled, setEnabled] = useState(initial.enabled);
  const [ifMissed, setIfMissed] = useState<IfMissed>(initial.ifMissed);
  const [waitForMe, setWaitForMe] = useState(initial.waitForMe);
  const [desktopTools, setDesktopTools] = useState(initial.desktopTools);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  /** The draft as last saved, to tell whether Run now has to save first. */
  const saved = useRef<string | null>(null);

  const turns = Number.parseInt(limits.maxTurns, 10);
  const draft = (): RoutineDraft => ({
    id,
    name: name.trim(),
    prompt: prompt.trim(),
    cwd: cwd ?? "",
    model: limits.model || null,
    maxTurns: Number.isFinite(turns) && turns > 0 ? turns : options.maxTurns,
    maxBudgetUsd: Number.parseFloat(limits.budget),
    claudePath: limits.claudePath || null,
    schedule: toSchedule(schedule),
    enabled,
    ifMissed,
    waitForMe,
    desktopTools,
  });

  // The saved routine as this form normalizes it (first render only), so an untouched form counts as saved.
  useEffect(() => {
    if (initial.id) saved.current = JSON.stringify(draft());
  }, []);

  /** Saves if needed; the routine's id, or `null` if it couldn't be saved. */
  const save = async (): Promise<string | null> => {
    const d = draft();
    if (!(d.maxBudgetUsd > 0)) {
      setError("Set a budget under Options ($0.05 to $20).");
      return null;
    }
    if (id && saved.current === JSON.stringify(d)) return id;
    setBusy(true);
    setError(null);
    try {
      const routine = await commands.routineSave(d);
      saved.current = JSON.stringify({ ...d, id: routine.id });
      setId(routine.id);
      onSaved(routine.id);
      return routine.id;
    } catch (err) {
      setError(errorMessage(err));
      return null;
    } finally {
      setBusy(false);
    }
  };

  const runNow = async () => {
    const routineId = await save();
    if (!routineId) return;
    try {
      const { runId } = await commands.routineRun(routineId);
      onShowRun(runId);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  const remove = async () => {
    if (!id) return onClose();
    try {
      await commands.routineDelete(id);
      onDeleted();
    } catch (err) {
      setConfirmDelete(false);
      setError(errorMessage(err));
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.nativeEvent.isComposing || !e.ctrlKey) return;
    if (e.key === "s" || e.key === "S" || e.key === "Enter") {
      e.preventDefault();
      void save();
    }
  };

  const running = view?.runningRunId ?? null;
  const last = view ? lastLine(view) : null;
  const lastRunId = running ?? view?.lastRun?.runId ?? null;
  const setKind = (kind: Kind) => setSchedule((s) => ({ ...s, kind }));
  const toggleDay = (day: Weekday) =>
    setSchedule((s) => ({ ...s, days: s.days.includes(day) ? s.days.filter((d) => d !== day) : [...s.days, day] }));
  const timed = schedule.kind === "days" || schedule.kind === "every";

  return (
    <form
      className="task routine"
      onKeyDown={onKey}
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div className="routine__head">
        <span className="task__label">{id ? "Routine" : "New routine"}</span>
        <span className="cmd__spacer" />
        {id &&
          (confirmDelete ? (
            <span className="routine__confirm" role="group" aria-label="Delete this routine?">
              <span>Delete this routine?</span>
              <button type="button" className="agent-link" onClick={() => setConfirmDelete(false)}>
                Keep
              </button>
              <button type="button" className="agent-link routine__delete" onClick={() => void remove()}>
                Delete
              </button>
            </span>
          ) : (
            <button type="button" className="agent-link" onClick={() => setConfirmDelete(true)}>
              Delete
            </button>
          ))}
      </div>

      {!view && <p className="routine__status">Its runs ask for approval like any other task.</p>}
      {view && last && (
        <p className="routine__status">
          <span>{scheduleLine(view, paused)}</span>
          <span className="routine__last" data-tone={last.tone}>
            {last.text}
          </span>
          {lastRunId && (
            <button type="button" className="agent-link" onClick={() => onShowRun(lastRunId)}>
              View run
            </button>
          )}
        </p>
      )}

      <label className="task-field task-field--name">
        <span>Name</span>
        <input
          value={name}
          autoFocus={!id}
          spellCheck={false}
          placeholder="Morning summary"
          onChange={(e) => setName(e.target.value)}
        />
      </label>

      <textarea
        ref={promptRef}
        className="task__prompt routine__prompt"
        placeholder="What should Claude do each time?"
        value={prompt}
        spellCheck={false}
        onChange={(e) => setPrompt(e.target.value)}
      />

      <FolderField
        folders={folders}
        cwd={cwd}
        picking={picking}
        onSelect={setCwd}
        onBrowsed={(path) => {
          if (path) add(path);
          promptRef.current?.focus();
        }}
        onError={setError}
      />

      <div className="task__section">
        <div className="routine__row">
          <span className="task__label">Schedule</span>
          <span className="cmd__spacer" />
          {schedule.kind !== "off" && (
            <Switch checked={enabled} onChange={setEnabled}>
              {enabled ? "On" : "Off"}
            </Switch>
          )}
        </div>
        <div className="seg" role="radiogroup" aria-label="Schedule">
          {KINDS.map((k) => (
            <button
              key={k.kind}
              type="button"
              role="radio"
              aria-checked={schedule.kind === k.kind}
              className="seg__item"
              onClick={() => setKind(k.kind)}
            >
              {k.label}
            </button>
          ))}
        </div>
        <div className="routine__when" data-off={schedule.kind !== "off" && !enabled}>
          {schedule.kind === "off" && <p className="routine__hint">Runs only when you click Run now.</p>}
          {schedule.kind === "start" && <p className="routine__hint">Runs 2 minutes after TopBar starts.</p>}
          {schedule.kind === "days" && (
            <>
              <div className="routine__row">
                <span className="routine__word">At</span>
                <input
                  type="time"
                  className="routine__time"
                  aria-label="Time"
                  value={schedule.time}
                  onChange={(e) => setSchedule((s) => ({ ...s, time: e.target.value }))}
                />
                <span className="routine__word">on</span>
                <span className="days" role="group" aria-label="Days">
                  {WEEKDAYS.map((w) => (
                    <button
                      key={w.day}
                      type="button"
                      className="days__day"
                      aria-pressed={schedule.days.includes(w.day)}
                      aria-label={w.short}
                      title={w.short}
                      onClick={() => toggleDay(w.day)}
                    >
                      {w.letter}
                    </button>
                  ))}
                </span>
              </div>
              <div className="routine__row routine__shortcuts">
                <button type="button" className="agent-link" onClick={() => setSchedule((s) => ({ ...s, days: WORKDAYS }))}>
                  Weekdays
                </button>
                <button type="button" className="agent-link" onClick={() => setSchedule((s) => ({ ...s, days: ALL_DAYS }))}>
                  Every day
                </button>
              </div>
            </>
          )}
          {schedule.kind === "every" && (
            <div className="routine__row">
              <span className="routine__word">Every</span>
              <input
                type="number"
                className="routine__hours"
                aria-label="Hours"
                min={1}
                max={24}
                value={schedule.hours}
                onChange={(e) => setSchedule((s) => ({ ...s, hours: e.target.value }))}
              />
              <span className="routine__word">hours from</span>
              <input
                type="time"
                className="routine__time"
                aria-label="From"
                value={schedule.from}
                onChange={(e) => setSchedule((s) => ({ ...s, from: e.target.value }))}
              />
            </div>
          )}
          {timed && (
            <div className="routine__row routine__missed">
              <span className="routine__word">If missed</span>
              <span className="seg seg--small" role="radiogroup" aria-label="If missed">
                <button
                  type="button"
                  role="radio"
                  aria-checked={ifMissed === "once"}
                  className="seg__item"
                  title="A time missed while TopBar was closed or the PC slept runs once when you're back (within 24 h)"
                  onClick={() => setIfMissed("once")}
                >
                  Run once when back
                </button>
                <button
                  type="button"
                  role="radio"
                  aria-checked={ifMissed === "skip"}
                  className="seg__item"
                  onClick={() => setIfMissed("skip")}
                >
                  Skip
                </button>
              </span>
            </div>
          )}
        </div>
      </div>

      <LimitsField options={options} limits={limits} onChange={setLimits} budgetRequired />

      <div className="task__section routine__switches">
        {/* Only scheduled runs wait; with no schedule it would say nothing. */}
        {schedule.kind !== "off" && (
          <Switch
            checked={waitForMe}
            onChange={setWaitForMe}
            hint="On a scheduled run, an approval's 10-minute timeout pauses while you're locked or away, up to 12 h."
          >
            Approvals wait for me while I'm away
          </Switch>
        )}
        <Switch
          checked={desktopTools}
          onChange={setDesktopTools}
          hint="Each look and action still asks you first."
        >
          Can use desktop tools (look at and act on windows)
        </Switch>
      </div>

      <footer className="task__footer">
        {error && (
          <span className="cmd-error" role="alert" title={error}>
            {error}
          </span>
        )}
        <span className="task__hint">Ctrl+S</span>
        <button type="button" className="cmd-btn cmd-btn--text" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="cmd-btn cmd-btn--quiet"
          disabled={busy || running !== null}
          title={running ? "Running" : "Saves, then runs it once"}
          onClick={() => void runNow()}
        >
          {running ? "Running" : "Run now"}
        </button>
        <button type="submit" className="cmd-btn cmd-btn--primary" disabled={busy}>
          {busy ? "Saving…" : "Save"}
        </button>
      </footer>
    </form>
  );
}

/** A Windows-style toggle with its label (and an optional quiet line under it). */
function Switch({
  checked,
  onChange,
  hint,
  children,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <label className="switch">
      <input type="checkbox" role="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span className="switch__track" aria-hidden="true" />
      <span className="switch__text">
        <span className="switch__label">{children}</span>
        {hint && <span className="switch__hint">{hint}</span>}
      </span>
    </label>
  );
}
