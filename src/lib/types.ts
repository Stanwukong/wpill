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

export interface ClaudeDoneActivity {
  /** Ours (matches `ClaudeSession.sessionId`). */
  sessionId: number;
  /** The session's working folder name. */
  project: string;
  /** The turn ended on an API error. */
  failed: boolean;
}

export interface ClaudeNeedsInputActivity {
  sessionId: number;
  project: string;
}

export type AgentRisk = "read" | "change" | "input";
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
   * One of TopBar's desktop tools: with `risk: "read"` Claude looks (windows, screenshot, UI tree; an eye), with
   * `risk: "input"` it acts (focus, invoke, set a field, click, type, keys, launch; a pointer).
   */
  desktop: boolean;
  /** The full text an action would type or set, for the command window's card (the island shows `summary`). */
  detail: string | null;
  /** Set for a routine's run (2.7e): show it in place of `project`. */
  routineName: string | null;
}

/**
 * TopBar is acting on the desktop for a run (Phase 2.7d). Sticky, not interactive; Rust withdraws it when the
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

export type Activity =
  | { kind: "volume"; payload: VolumeActivity }
  | { kind: "brightness"; payload: BrightnessActivity }
  | { kind: "bluetooth"; payload: BluetoothActivity }
  | { kind: "power"; payload: PowerActivity }
  | { kind: "capsLock"; payload: CapsLockActivity }
  | { kind: "screenshot"; payload: ScreenshotActivity }
  | { kind: "claudeDone"; payload: ClaudeDoneActivity }
  | { kind: "claudeNeedsInput"; payload: ClaudeNeedsInputActivity }
  | { kind: "agentApproval"; payload: AgentApprovalActivity }
  | { kind: "agentActing"; payload: AgentActingActivity }
  | { kind: "agentFinished"; payload: AgentFinishedActivity };

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

export type ClaudeStatus = "idle" | "working" | "done" | "needsInput";

export interface ClaudeSession {
  /** Assigned by Rust, stable for the session's lifetime. */
  sessionId: number;
  project: string;
  status: ClaudeStatus;
  /** Unix epoch ms of the last status change. */
  updatedAtMs: number;
}

export interface ClaudeSessions {
  /** Most recently updated first. */
  sessions: ClaudeSession[];
}

export interface ClaudeHooksStatus {
  installed: boolean;
}

export interface AgentRun {
  runId: string;
  /** Folder name of `cwd`. */
  project: string;
  cwd: string;
  /** The first prompt, shortened. */
  prompt: string;
  status: AgentStatus;
  /** Set once the run has ended. */
  reason: AgentEndReason | null;
  /** What it's doing now ("Running git status"), while working. */
  step: string | null;
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
}

export interface AgentRunRoutine {
  id: string;
  name: string;
  trigger: RoutineTrigger;
}

/** How a routine's run started: Run now, or its schedule. */
export type RoutineTrigger = "manual" | "schedule";

export type Weekday = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

/** When a routine runs by itself, while TopBar is running. Times are local `HH:MM`. */
export type Schedule =
  | { kind: "off" }
  | { kind: "days"; time: string; days: Weekday[] }
  /** Every `hours` (1–24) hours, counted from `from` on the day the schedule was saved. */
  | { kind: "every"; hours: number; from: string }
  /** Two minutes after TopBar starts. */
  | { kind: "start" };

/** A due time missed while TopBar was closed or the PC slept: run once when back, or skip it. */
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
  /** TopBar's desktop looks and actions. Off by default. */
  desktopTools: boolean;
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

export type AgentEvent =
  | { kind: "user"; text: string }
  | { kind: "text"; text: string }
  | { kind: "tool"; id: string; name: string; label: string }
  | { kind: "toolResult"; id: string; isError: boolean; text: string }
  | { kind: "approval"; requestId: number; tool: string; summary: string; risk: AgentRisk }
  | { kind: "decision"; requestId: number; allow: boolean; by: "user" | "remembered" | "timeout" | "runEnded" }
  | { kind: "result"; status: AgentStatus; reason: AgentEndReason; turns: number; costUsd: number | null }
  | { kind: "error"; message: string };

export type AgentEntry = AgentEvent & { atMs: number };

/** `agent:output` (command window only). */
export interface AgentOutput {
  runId: string;
  entry: AgentEntry;
}

export interface AgentStartRequest {
  prompt: string;
  cwd: string;
  model?: string | null;
  maxTurns?: number | null;
  maxBudgetUsd?: number | null;
  claudePath?: string | null;
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
    | "claudeSessionGone"
    | "claudeWindowNotFound"
    | "claudeSettings"
    | "agentRunGone"
    | "agentApprovalGone"
    | "agentNoClaude"
    | "agentInvalid"
    | "agentStart"
    | "noCommandWindow"
    | "win32"
    | "tauri";
  message: string;
}
