// Mirrors the payload structs in src-tauri/src/ipc.rs and src-tauri/src/error.rs.

export interface IslandState {
  expanded: boolean;
  hidden: boolean;
  /** What's on screen when collapsed with an activity showing. */
  activity: ActivityPayload | null;
}

export interface VolumeActivity {
  /** 0–100. */
  level: number;
  muted: boolean;
  device: string;
}

export interface BrightnessActivity {
  /** 0–100. */
  level: number;
}

export interface BluetoothActivity {
  name: string;
  connected: boolean;
  /** 0–100, when the device reports it. */
  battery: number | null;
}

export interface PowerActivity {
  event: "pluggedIn" | "unplugged" | "low";
  /** Battery percentage. */
  level: number;
}

export interface CapsLockActivity {
  on: boolean;
}

export interface ScreenshotActivity {
  fileName: string;
  /** PNG data URL. */
  thumbnail: string;
  width: number;
  height: number;
}

/** Which coding agent a terminal session belongs to (2.9a, D47). Mirrors Rust's `AgentKind`. */
export type AgentKind = "claude" | "codex" | "opencode";

export interface SessionDoneActivity {
  /** Ours (matches `AgentSession.sessionId`). */
  sessionId: number;
  agent: AgentKind;
  /** The session's working folder name. */
  project: string;
  /** The turn ended on an API error. */
  failed: boolean;
}

export interface SessionNeedsInputActivity {
  sessionId: number;
  agent: AgentKind;
  project: string;
}

export type AgentRisk = "read" | "change" | "input";

/**
 * A terminal session asks to use a tool (2.8a). Sticky: Rust withdraws it once you answer here or in the terminal,
 * or after it hands the request back to the terminal. Allow / Deny only.
 */
export interface SessionApprovalActivity {
  requestId: number;
  /** The session's row (click to focus its window); `null` if Cay hasn't seen the session yet. */
  sessionId: number | null;
  agent: AgentKind;
  project: string;
  /** Display name, e.g. `Bash` or `github · create_issue`. */
  tool: string;
  /** One line: the command, the file, the host. */
  summary: string;
  risk: AgentRisk;
  /** The card's diff or wrapped command (2.8b); the whole one comes from `sessionAskPreview`. */
  preview: ApprovalPreview | null;
  /** A question or a plan (2.8c): answered with `sessionAnswer` / `sessionPlan`, never Allow. Deny skips it. */
  interaction: Interaction | null;
}

/**
 * `AskUserQuestion` or `ExitPlanMode` (2.8c). A question carries the answers given so far (question text → label,
 * or labels joined by ", "); Rust updates the card in place after each. A plan's text comes from the preview
 * commands (`kind: "plan"`); the island only says "Plan ready".
 */
export type Interaction =
  | { kind: "question"; questions: QuestionView[]; answers: Record<string, string> }
  | { kind: "plan"; title: string; lines: number };

export interface QuestionView {
  /** A short label ("Framework"). */
  header: string;
  question: string;
  options: { label: string; description: string }[];
  multiSelect: boolean;
}

/** One question's answer: option labels (one for a single-select), or your own words in `other`. */
export interface QuestionAnswer {
  question: string;
  labels: string[];
  other: string | null;
}

/** `sessions:asks` / `session_asks`: the session requests the island holds, oldest first (command window). */
export interface SessionAsks {
  asks: SessionApprovalActivity[];
}

/**
 * What an approval shows beyond its one line (2.8b), built in Rust. The card's copy holds at most a few lines
 * (`moreLines` counts the rest); the command window fetches the whole one.
 */
export type ApprovalPreview =
  | ({ kind: "diff" } & DiffPreview)
  | ({ kind: "command" } & CommandPreview)
  /** A plan (2.8c), whole, from the preview commands only. `cut`: shortened and not found whole in its file. */
  | { kind: "plan"; markdown: string; cut: boolean };

export interface DiffPreview {
  /** Relative to the folder when inside it. */
  file: string;
  lines: DiffLine[];
  /** `null` when the input was cut on its way to Cay: say "cut", never a count. */
  added: number | null;
  removed: number | null;
  moreLines: number;
  cut: boolean;
  /** "New file", "Every occurrence", "Replaces the whole file". */
  note: string | null;
}

/** `gap`: unchanged lines left out between two changes (empty `text`). */
export interface DiffLine {
  op: "add" | "del" | "ctx" | "gap";
  text: string;
}

/** A shell command wrapped at token boundaries to 48 columns; continuation lines start with two spaces. */
export interface CommandPreview {
  lines: string[];
  moreLines: number;
  cut: boolean;
}

/** `command_take_focus`: what a newly created command window selects. */
export type CommandFocus = { kind: "run"; runId: string } | { kind: "ask"; requestId: number };
export type AgentStatus = "starting" | "working" | "waiting" | "done" | "failed" | "stopped";
export type AgentEndReason = "completed" | "maxTurns" | "maxBudget" | "error" | "stopped";

/** An agent run asks to use a tool. Sticky and interactive: Allow / Deny. */
export interface AgentApprovalActivity {
  requestId: number;
  runId: string;
  project: string;
  /** Display name, e.g. `Bash` or `github · create_issue`. */
  tool: string;
  /** One line: the command, the file, the host. */
  summary: string;
  risk: AgentRisk;
  /** What "allow for this run" covers ("git", "edits"); `null` where it isn't offered. */
  rememberAs: string | null;
  /**
   * One of Cay's desktop tools: with `risk: "read"` Claude looks (windows, screenshot, UI tree; an eye), with
   * `risk: "input"` it acts (focus, invoke, set a field, click, type, keys, launch; a pointer).
   */
  desktop: boolean;
  /** The full text an action would type or set, for the command window's card (the island shows `summary`). */
  detail: string | null;
  /** Set for a routine's run (2.7e): show it in place of `project`. */
  routineName: string | null;
  /** The card's diff or wrapped command (2.8b); the whole one comes from `agentApprovalPreview`. */
  preview: ApprovalPreview | null;
  /** A question or a plan (2.8c): answered with `agentAnswer` / `agentPlan`, never Allow. Deny skips it. */
  interaction: Interaction | null;
}

/**
 * Cay is acting on the desktop for a run (Phase 2.7d). Sticky, not interactive; Rust withdraws it when the
 * action ends. `waiting`: it acts once you've let go of the mouse and keyboard; `acting`: it's clicking or typing.
 * Any input of yours stops it.
 */
export interface AgentActingActivity {
  runId: string;
  project: string;
  /** "Clicking Save", "Typing into Notepad". */
  step: string;
  phase: "waiting" | "acting";
}

export interface AgentFinishedActivity {
  runId: string;
  project: string;
  /** Set for a routine's run (2.7e): show it in place of `project`. */
  routineName: string | null;
  status: AgentStatus;
  reason: AgentEndReason | null;
}

/** Phase 6b: an event starts soon. Shown once, at the lead time; interactive (Join, open). */
export interface CalendarSoonActivity {
  eventId: string;
  title: string;
  startsAtMs: number;
  /** Whole minutes left when offered ("in 5 min"); 0 = starting now. Rust's number: don't recount it. */
  minutes: number;
  hasJoin: boolean;
  /** The calendar's colour in Google. */
  color: string | null;
}

export type Activity =
  | { kind: "volume"; payload: VolumeActivity }
  | { kind: "brightness"; payload: BrightnessActivity }
  | { kind: "bluetooth"; payload: BluetoothActivity }
  | { kind: "power"; payload: PowerActivity }
  | { kind: "capsLock"; payload: CapsLockActivity }
  | { kind: "screenshot"; payload: ScreenshotActivity }
  | { kind: "sessionDone"; payload: SessionDoneActivity }
  | { kind: "sessionNeedsInput"; payload: SessionNeedsInputActivity }
  | { kind: "sessionApproval"; payload: SessionApprovalActivity }
  | { kind: "agentApproval"; payload: AgentApprovalActivity }
  | { kind: "agentActing"; payload: AgentActingActivity }
  | { kind: "agentFinished"; payload: AgentFinishedActivity }
  | { kind: "calendarSoon"; payload: CalendarSoonActivity };

/** `activity:show`. The same `id` again means the activity was updated in place (e.g. another volume step). */
export type ActivityPayload = Activity & {
  id: number;
  /** Time left; Rust times it, React never hides it on its own. `null`: until Rust withdraws it. */
  durationMs: number | null;
};

export interface ActivityHidePayload {
  id: number;
}

export interface VisibilityPayload {
  hidden: boolean;
  reason: "fullscreen" | null;
}

/** `monitor:changed`: the island's monitor, resolution or DPI scale changed (scale without Text size). */
export interface MonitorChangedPayload {
  scale: number;
}

export interface WindowInfo {
  hwnd: number;
  title: string;
  exeName: string | null;
  /** PNG data URL. */
  icon: string | null;
}

export type MediaStatus = "playing" | "paused" | "stopped" | "changing";

export interface MediaSession {
  /** Assigned by Rust, stable for the session's lifetime. */
  id: number;
  appName: string;
  title: string;
  artist: string;
  album: string;
  /** Image data URL. */
  artwork: string | null;
  status: MediaStatus;
  /** Position at `updatedAtMs`; interpolate from there while playing. */
  positionMs: number;
  durationMs: number | null;
  /** Unix epoch ms. */
  updatedAtMs: number;
  playbackRate: number;
  canPlay: boolean;
  canPause: boolean;
  canNext: boolean;
  canPrev: boolean;
  canSeek: boolean;
}

export interface MediaState {
  /** Most recently active first. */
  sessions: MediaSession[];
}

export type SessionStatus = "idle" | "working" | "done" | "needsInput";

export interface AgentSession {
  /** Assigned by Rust, stable for the session's lifetime. */
  sessionId: number;
  agent: AgentKind;
  project: string;
  status: SessionStatus;
  /** Unix epoch ms of the last status change. */
  updatedAtMs: number;
  /** The tool it's using and on what ("Edit · hover/logic.rs"), from Rust (2.8d); `null` between tasks. */
  step: string | null;
  /** Its to-do list's progress, from the latest `TodoWrite` (2.8d). */
  todos: TodoCount | null;
  /**
   * Its permission mode as its hooks report it (2.8e): `default`, `acceptEdits`, `plan`, `auto`, or another of
   * Claude Code's (`bypassPermissions`, `dontAsk`). Read-only: Cay never changes a terminal session's mode.
   */
  mode: string | null;
}

/**
 * An agent run's permission mode (2.8e), Claude Code's names: `auto` (its classifier approves everyday steps; the
 * default), `default` (Ask), `acceptEdits`, `plan`. Bypass is never offered.
 */
export type AgentMode = "auto" | "default" | "acceptEdits" | "plan";

/** `TodoWrite`'s progress (2.8d): items completed and in all. */
export interface TodoCount {
  done: number;
  total: number;
}

export interface AgentSessions {
  /** Most recently updated first. */
  sessions: AgentSession[];
  /** The newest usage limits from any session's status line (2.8f); `null` until one arrives. */
  limits: UsageLimits | null;
}

/**
 * Claude usage limits (2.8f), from Claude Code's status line input (Pro/Max/Ultra, after the session's first response).
 * Each window may be missing. `stale` is set by Rust once `resetsAtMs` has passed without a newer reading.
 */
export interface UsageLimits {
  fiveHour: LimitWindow | null;
  sevenDay: LimitWindow | null;
  /** When the status line read it (Unix ms). */
  atMs: number;
}

export interface LimitWindow {
  /** 0–100. */
  usedPercent: number;
  resetsAtMs: number;
  stale: boolean;
}

export interface ClaudeHooksStatus {
  installed: boolean;
  /** Connected by an earlier version: Connect updates it (2.8f). */
  outOfDate: boolean;
}

/** The agent a run uses (2.9b). Mirrors Rust's `AgentBackend`. */
export type AgentBackend = "claude" | "opencode" | "codex";

/** One agent in the command window's picker (2.9c). */
export interface AgentBackendInfo {
  backend: AgentBackend;
  /** Installed where Cay can start it. */
  available: boolean;
  /** How to install it, when it isn't. */
  install: string | null;
  /** The model a task uses when it names none. */
  defaultModel: string | null;
  /** The models the agent lists; empty means free text (Claude Code) or not known yet. */
  models: string[];
}

/** `agent_backends`. */
export interface AgentBackendsPayload {
  defaultBackend: AgentBackend;
  backends: AgentBackendInfo[];
}

export interface AgentRun {
  runId: string;
  /** Which agent runs it. */
  backend: AgentBackend;
  /** Folder name of `cwd`. */
  project: string;
  cwd: string;
  /** The first prompt, shortened. */
  prompt: string;
  status: AgentStatus;
  /** Set once the run has ended. */
  reason: AgentEndReason | null;
  /** What it's doing now ("Bash · git status", 2.8d), while working. */
  step: string | null;
  /** The latest `TodoWrite`'s progress (2.8d), while working. */
  todos: TodoCount | null;
  /** Its permission mode as Claude Code last reported it, or as just set (2.8e). */
  mode: AgentMode;
  /** Chosen while no process runs: applies with your next message (the chip says so). */
  nextMode: AgentMode | null;
  turns: number;
  /** Claude Code's own estimate for the session. */
  costUsd: number | null;
  model: string | null;
  /** Approvals waiting for an answer, oldest first. */
  pending: AgentApprovalActivity[];
  error: string | null;
  startedAtMs: number;
  updatedAtMs: number;
  /** The routine that started it (2.7e). */
  routine: AgentRunRoutine | null;
  /** Whether images go to the agent as images (2.10b.2); null until an ACP agent has said. Files always go. */
  acceptsImages: boolean | null;
}

export interface AgentRunRoutine {
  id: string;
  name: string;
  trigger: RoutineTrigger;
}

/** How a routine's run started: Run now, or its schedule. */
export type RoutineTrigger = "manual" | "schedule";

export type Weekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

/** When a routine runs by itself, while Cay is running. Times are local `HH:MM`. */
export type Schedule =
  | { kind: "off" }
  | { kind: "days"; time: string; days: Weekday[] }
  /** Every `hours` (1–24) hours, counted from `from` on the day the schedule was saved. */
  | { kind: "every"; hours: number; from: string }
  /** Two minutes after Cay starts. */
  | { kind: "start" };

/** A due time missed while Cay was closed or the PC slept: run once when back, or skip it. */
export type IfMissed = "once" | "skip";

export type SkipReason = "missed" | "paused" | "stillRunning" | "tooMany" | "folderMissing" | "startFailed";

export interface RoutineLastRun {
  runId: string;
  trigger: RoutineTrigger;
  atMs: number;
  /** `null` while it runs. */
  status: AgentStatus | null;
  reason: AgentEndReason | null;
  costUsd: number | null;
}

export interface RoutineSkip {
  atMs: number;
  /** The due time that was skipped. */
  dueMs: number;
  why: SkipReason;
}

/** What `routine_save` takes: a new routine (`id: null`) or an edit. Rust checks and clamps it. */
export interface RoutineDraft {
  id: string | null;
  name: string;
  prompt: string;
  cwd: string;
  model: string | null;
  maxTurns: number;
  /** Required: routines always have a budget ($0.05–$20). */
  maxBudgetUsd: number;
  claudePath: string | null;
  schedule: Schedule;
  /** The schedule is on (Run now works either way). */
  enabled: boolean;
  ifMissed: IfMissed;
  /** Scheduled runs: an approval's timeout pauses while you're away (locked, or no input for 5 min; 12 h at most). */
  waitForMe: boolean;
  /** Cay's desktop looks and actions. Off by default. */
  desktopTools: boolean;
  /** Cay's Google tools (6c: calendar, tasks, Gmail). Off by default and for routines saved before 6c. */
  googleTools: boolean;
  /** The mode its runs start in (2.8e); new routines start at `auto`, ones saved before 2.8e read as `default`. */
  mode: AgentMode;
  /** The agent its runs use (2.9c); routines saved before 2.9c read as `claude`. */
  backend: AgentBackend;
}

/** A saved routine, as Rust stores it. */
export interface Routine extends Omit<RoutineDraft, "id"> {
  id: string;
  createdAtMs: number;
  updatedAtMs: number;
  scheduleSinceMs: number;
  lastDueMs: number;
  lastRun: RoutineLastRun | null;
  lastSkip: RoutineSkip | null;
}

export interface RoutineView extends Routine {
  /** When its schedule runs it next (`null`: off, disabled, paused, or "at start" saved after this start). */
  nextRunAtMs: number | null;
  /** Its run going now, if any. */
  runningRunId: string | null;
}

/** `routines:changed` and `routines_list`. */
export interface Routines {
  routines: RoutineView[];
  paused: boolean;
}

export interface AgentRuns {
  /** Most recently started first. */
  runs: AgentRun[];
}

/** An attachment as the transcript shows it (2.10b): never its content or where it came from. */
export interface AgentAttachment {
  name: string;
  kind: "image" | "file";
  /** The original file's size. */
  bytes: number;
}

export type AgentEvent =
  | { kind: "user"; text: string; attachments?: AgentAttachment[] }
  | { kind: "text"; text: string }
  | { kind: "tool"; id: string; name: string; label: string; preview?: ApprovalPreview }
  | { kind: "toolResult"; id: string; isError: boolean; text: string }
  | { kind: "approval"; requestId: number; tool: string; summary: string; risk: AgentRisk }
  | { kind: "decision"; requestId: number; allow: boolean; by: "user" | "remembered" | "timeout" | "runEnded" | "answered" | "approved" | "feedback" }
  | { kind: "result"; status: AgentStatus; reason: AgentEndReason; turns: number; costUsd: number | null; durationMs: number | null }
  | { kind: "error"; message: string };

export type AgentEntry = AgentEvent & { atMs: number };

/** `agent:output` (command window only). */
export interface AgentOutput {
  runId: string;
  entry: AgentEntry;
}

/** `agent:textDraft` (2.10a): the whole draft so far, at most 10 times a second. */
export interface AgentTextDraft {
  runId: string;
  text: string;
}

export interface AgentStartRequest {
  prompt: string;
  cwd: string;
  model?: string | null;
  maxTurns?: number | null;
  maxBudgetUsd?: number | null;
  claudePath?: string | null;
  /** The permission mode (2.8e); omitted uses the default (`auto`). */
  mode?: AgentMode | null;
  /** The agent (2.9b); omitted is Claude Code. */
  backend?: AgentBackend | null;
  /** Files to attach to the first message (2.10b): paths, never bytes. Not for routines. */
  attachments?: string[];
}

export interface ClaudeInstall {
  path: string;
  version: string | null;
  source: string;
}

export interface AgentOptions {
  installs: ClaudeInstall[];
  defaultInstall: string | null;
  /** Most recent first. */
  recentFolders: string[];
  model: string | null;
  maxTurns: number;
  maxBudgetUsd: number | null;
  hotkey: string;
  /** Set when a hotkey couldn't be registered. */
  hotkeyError: string | null;
  /** The mode a new task starts in (2.8e; `auto` unless changed in settings.json). */
  mode: AgentMode;
}

export interface AppError {
  kind:
    | "windowGone"
    | "foregroundDenied"
    | "noIslandWindow"
    | "noMonitor"
    | "mediaSessionGone"
    | "mediaRejected"
    | "activityGone"
    | "sessionGone"
    | "sessionWindowNotFound"
    | "claudeSettings"
    | "agentRunGone"
    | "agentApprovalGone"
    | "agentNoClaude"
    | "agentInvalid"
    | "agentStart"
    | "noCommandWindow"
    | "google"
    | "calendarItemGone"
    | "processGroupGone"
    | "processEndBusy"
    | "win32"
    | "tauri";
  message: string;
}

// ---- Phase 6: Google Calendar and Tasks (mirrors ipc.rs) ----

export type GoogleStatus = "disconnected" | "connecting" | "connected" | "reconnect";

/** `google:state`. Never carries a token. */
export interface GoogleState {
  status: GoogleStatus;
  /** The primary calendar's id (its address), once a fetch has seen it. */
  account: string | null;
  /** What Google granted (the consent screen lets you untick a scope). */
  calendar: boolean;
  tasks: boolean;
  /** Gmail for agent runs (6c): reading and drafts were granted. */
  gmail: boolean;
  lastSyncMs: number | null;
  /** Why the last refresh or sign-in failed; the last data is kept. */
  error: string | null;
  /** This build (or settings.json) has a Google client. */
  configured: boolean;
}

/** Rust works out the phase and re-sends at each boundary; React never times it. */
export type EventPhase = "later" | "soon" | "now";

export interface CalendarEvent {
  id: string;
  title: string;
  startMs: number;
  endMs: number;
  /** The calendar's name. */
  calendar: string;
  /** The calendar's colour in Google (`#rrggbb`). */
  color: string | null;
  hasJoin: boolean;
  phase: EventPhase;
}

/** An open task due today or overdue. Tasks have dates, not times. */
export interface GoogleTask {
  id: string;
  title: string;
  /** `YYYY-MM-DD` */
  due: string;
  overdue: boolean;
  list: string;
}

/** `calendar:upcoming`: the next 24 h of events, soonest first, and due tasks. */
export interface CalendarUpcoming {
  events: CalendarEvent[];
  tasks: GoogleTask[];
  /** The pill's calendar icon shows. */
  pending: boolean;
}

// Phase 3: memory-heavy apps (mirrors `HeavyProcessesPayload` in ipc.rs).
export interface HeavyProcesses {
  /** `processes.enabled` in settings.json; no section while it's off. */
  enabled: boolean;
  /** Biggest first. */
  groups: ProcessGroup[];
}

/** One app: its processes (same exe, plus helpers such as WebView2 it started), summed. */
export interface ProcessGroup {
  /** Stable across refreshes (the lowercased exe path). */
  key: string;
  /** "Google Chrome", else the exe's name without `.exe`. */
  name: string;
  pids: ProcessId[];
  /** Private working set, as Task Manager's "Memory" column. */
  memoryBytes: number;
  /** PNG data URL. */
  icon: string | null;
  /** Cay never ends it (system, security, the desktop, services, Cay itself). */
  protected: boolean;
}

export interface ProcessId {
  pid: number;
  /** FILETIME ticks as a decimal string (too big for a JS number). */
  created: string;
}

/** `end_process_group`'s reply (Phase 3b; mirrors `EndResult` in ipc.rs). */
export interface EndResult {
  /** Cancel in the confirm box: nothing was done. */
  cancelled: boolean;
  /** One per pid asked for (none when cancelled). */
  outcomes: EndProcessOutcome[];
}

export interface EndProcessOutcome {
  pid: number;
  created: string;
  outcome: EndOutcome;
}

/** What happened to one process (mirrors `EndOutcome` in ipc.rs). */
export type EndOutcome =
  /** Cay ended it. */
  | "ended"
  /** It exited by itself after the app was asked to close. */
  | "closed"
  /** Runs as administrator: Cay can't end it. */
  | "accessDenied"
  /** Part of a protected app: Cay never ends it. */
  | "protected"
  /** It had already exited. */
  | "notFound"
  /** Not the process that was listed (pid reused), or not part of that app. */
  | "changed"
  /** The app is showing a dialog after being asked to close: left open. */
  | "prompted"
  /** Windows refused for another reason. */
  | "failed";
