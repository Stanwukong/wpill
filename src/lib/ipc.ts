// Typed IPC layer: the only place command and event names appear on the TS side.
// Mirrors src-tauri/src/ipc.rs; change both together.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ActivityHidePayload,
  AgentEntry,
  AgentMode,
  AgentOptions,
  AgentOutput,
  AgentTextDraft,
  AgentRuns,
  AgentStartRequest,
  AgentBackendsPayload,
  ActivityPayload,
  Routine,
  RoutineDraft,
  Routines,
  AppError,
  ApprovalPreview,
  CalendarUpcoming,
  GoogleState,
  EndResult,
  HeavyProcesses,
  ProcessId,
  SessionAsks,
  QuestionAnswer,
  ClaudeHooksStatus,
  AgentSessions,
  CommandFocus,
  IslandState,
  MediaState,
  MonitorChangedPayload,
  VisibilityPayload,
  WindowInfo,
} from "./types";

const EVENTS = {
  hoverEnter: "island:hover-enter",
  hoverLeave: "island:hover-leave",
  visibility: "island:visibility",
  monitorChanged: "monitor:changed",
  mediaState: "media:state",
  activityShow: "activity:show",
  activityHide: "activity:hide",
  agentSessions: "sessions:changed",
  agentRuns: "agent:runs",
  agentOutput: "agent:output",
  agentTextDraft: "agent:textDraft",
  commandFocusRun: "command:focus-run",
  commandFocusAsk: "command:focus-ask",
  sessionAsks: "sessions:asks",
  routinesChanged: "routines:changed",
  googleState: "google:state",
  calendarUpcoming: "calendar:upcoming",
  processesHeavy: "processes:heavy",
} as const;

/** Which island shape a reported rect measures. */
export type RectMode = "panel" | "activity";

export const commands = {
  listWindows: () => invoke<WindowInfo[]>("list_windows"),
  activateWindow: (hwnd: number) => invoke<void>("activate_window", { hwnd }),
  /** Physical px: the island's width and its bottom edge from the window top, for the shape `mode`. */
  setPanelRect: (width: number, bottom: number, mode: RectMode) =>
    invoke<void>("set_panel_rect", { width, bottom, mode }),
  requestCollapse: () => invoke<void>("request_collapse"),
  getIslandState: () => invoke<IslandState>("get_island_state"),
  getMediaState: () => invoke<MediaState>("get_media_state"),
  mediaPlayPause: (sessionId: number) => invoke<void>("media_play_pause", { sessionId }),
  mediaNext: (sessionId: number) => invoke<void>("media_next", { sessionId }),
  mediaPrev: (sessionId: number) => invoke<void>("media_prev", { sessionId }),
  mediaSeek: (sessionId: number, positionMs: number) =>
    invoke<void>("media_seek", { sessionId, positionMs: Math.round(positionMs) }),
  /** Call while the mouse button is still down; resolves when the drop or cancel is done. */
  activityDragScreenshot: (id: number) => invoke<void>("activity_drag_screenshot", { id }),
  activityOpenScreenshot: (id: number) => invoke<void>("activity_open_screenshot", { id }),
  getSessions: () => invoke<AgentSessions>("get_sessions"),
  /** Brings the session's terminal or editor to the front and acknowledges its activity. */
  sessionFocus: (sessionId: number) => invoke<void>("session_focus", { sessionId }),
  /** Answers a terminal session's permission request (2.8a). Errors `agentApprovalGone` once it's no longer held. */
  sessionDecide: (requestId: number, answer: "allow" | "deny") => invoke<void>("session_decide", { requestId, answer }),
  /** Session requests the island holds (2.8b), for the command window. */
  sessionAsks: () => invoke<SessionAsks>("session_asks"),
  /** A held session request's whole diff or command; `null` once it's answered or handed back. */
  sessionAskPreview: (requestId: number) => invoke<ApprovalPreview | null>("session_ask_preview", { requestId }),
  /** One question of a held session question (2.8c). Errors `agentInvalid` (not an option), `agentApprovalGone`. */
  sessionAnswer: (requestId: number, answer: QuestionAnswer) => invoke<void>("session_answer", { requestId, answer }),
  /** Approve a held session plan, or send it back with feedback (required then). */
  sessionPlan: (requestId: number, approve: boolean, feedback?: string) =>
    invoke<void>("session_plan", { requestId, approve, feedback: feedback ?? null }),
  claudeHooksStatus: () => invoke<ClaudeHooksStatus>("claude_hooks_status"),
  claudeHooksInstall: () => invoke<void>("claude_hooks_install"),
  claudeHooksRemove: () => invoke<void>("claude_hooks_remove"),
  agentStart: (request: AgentStartRequest) => invoke<{ runId: string }>("agent_start", { request }),
  /** 2.9c: the agents the picker offers; the first call may take a few seconds (OpenCode's model list). */
  agentBackends: () => invoke<AgentBackendsPayload>("agent_backends"),
  /** A follow-up: queued into the running turn, or resumes a finished run. */
  agentSend: (runId: string, text: string) => invoke<void>("agent_send", { runId, text }),
  agentStop: (runId: string) => invoke<void>("agent_stop", { runId }),
  /** Removes an ended run from the list. Errors `agentInvalid` while it still runs, `agentRunGone`. */
  agentRemove: (runId: string) => invoke<void>("agent_remove", { runId }),
  agentRuns: () => invoke<AgentRuns>("agent_runs"),
  agentTranscript: (runId: string) => invoke<AgentEntry[]>("agent_transcript", { runId }),
  /** Opens a transcript link in the browser; Rust refuses anything but https (2.10a). */
  agentOpenLink: (url: string) => invoke<void>("agent_open_link", { url }),
  agentDecide: (requestId: number, decision: "allow" | "deny", remember: "once" | "run") =>
    invoke<void>("agent_decide", { requestId, decision, remember }),
  /** A waiting approval's whole diff or command (2.8b); `null` once answered or for tools without one. */
  agentApprovalPreview: (requestId: number) =>
    invoke<ApprovalPreview | null>("agent_approval_preview", { requestId }),
  /** One question of a run's waiting question (2.8c). */
  agentAnswer: (requestId: number, answer: QuestionAnswer) => invoke<void>("agent_answer", { requestId, answer }),
  /** Approve a run's waiting plan, or send it back with feedback (required then). */
  agentPlan: (requestId: number, approve: boolean, feedback?: string) =>
    invoke<void>("agent_plan", { requestId, approve, feedback: feedback ?? null }),
  /** Sets a run's permission mode (2.8e): at once while it runs, else for your next message. */
  agentSetMode: (runId: string, mode: AgentMode) => invoke<void>("agent_set_mode", { runId, mode }),
  /** The chip's click: the next mode in Claude Code's cycle (Ask → Accept edits → Plan → Auto), chosen in Rust. */
  agentCycleMode: (runId: string) => invoke<void>("agent_cycle_mode", { runId }),
  agentOptions: () => invoke<AgentOptions>("agent_options"),
  /** The shell's folder picker; `null` if cancelled. */
  agentPickFolder: () => invoke<string | null>("agent_pick_folder"),
  /** Opens (and focuses) the command window, optionally on one run. */
  commandWindowShow: (runId?: string) => invoke<void>("command_window_show", { runId: runId ?? null }),
  /** Opens the command window on a terminal session's held request (2.8b). */
  commandWindowShowAsk: (requestId: number) => invoke<void>("command_window_show_ask", { requestId }),
  /** What a newly created command window should select, once (it missed the focus event). */
  commandTakeFocus: () => invoke<CommandFocus | null>("command_take_focus"),
  commandWindowHide: () => invoke<void>("command_window_hide"),
  routinesList: () => invoke<Routines>("routines_list"),
  /** Creates (no `id`) or edits a routine; errors `agentInvalid` with a message to show. */
  routineSave: (routine: RoutineDraft) => invoke<Routine>("routine_save", { routine }),
  routineDelete: (id: string) => invoke<void>("routine_delete", { id }),
  /** Run now. Errors `agentInvalid` if it's already running. */
  routineRun: (id: string) => invoke<{ runId: string }>("routine_run", { id }),
  routinesPause: (paused: boolean) => invoke<void>("routines_pause", { paused }),
  /** Phase 6: the Google sign-in state, for an initial sync. */
  googleState: () => invoke<GoogleState>("google_state"),
  calendarUpcoming: () => invoke<CalendarUpcoming>("calendar_upcoming"),
  /** Opens Google's sign-in in the browser; `google:state` follows. Errors `google` (no client in this build). */
  googleConnect: () => invoke<void>("google_connect"),
  /** Revokes at Google and forgets the sign-in; confirm first. */
  googleDisconnect: () => invoke<void>("google_disconnect"),
  /** Opens an event or task in the browser. Errors `calendarItemGone`. */
  calendarOpen: (id: string) => invoke<void>("calendar_open", { id }),
  /** Opens an event's meeting link (only when `hasJoin`). Errors `calendarItemGone`. */
  calendarJoin: (id: string) => invoke<void>("calendar_join", { id }),
  /** Phase 3: the latest memory-heavy apps (empty until the panel has been open once) and whether they're on. */
  listHeavyProcesses: (limit?: number) => invoke<HeavyProcesses>("list_heavy_processes", { limit }),
  /** Asks in a native confirm box first; Rust only touches pids it listed for `key`. */
  endProcessGroup: (key: string, pids: ProcessId[]) =>
    invoke<EndResult>("end_process_group", { key, pids }),
};

export const on = {
  hoverEnter: (cb: () => void): Promise<UnlistenFn> => listen(EVENTS.hoverEnter, () => cb()),
  hoverLeave: (cb: () => void): Promise<UnlistenFn> => listen(EVENTS.hoverLeave, () => cb()),
  visibility: (cb: (payload: VisibilityPayload) => void): Promise<UnlistenFn> =>
    listen<VisibilityPayload>(EVENTS.visibility, (e) => cb(e.payload)),
  monitorChanged: (cb: (payload: MonitorChangedPayload) => void): Promise<UnlistenFn> =>
    listen<MonitorChangedPayload>(EVENTS.monitorChanged, (e) => cb(e.payload)),
  mediaState: (cb: (payload: MediaState) => void): Promise<UnlistenFn> =>
    listen<MediaState>(EVENTS.mediaState, (e) => cb(e.payload)),
  activityShow: (cb: (payload: ActivityPayload) => void): Promise<UnlistenFn> =>
    listen<ActivityPayload>(EVENTS.activityShow, (e) => cb(e.payload)),
  activityHide: (cb: (payload: ActivityHidePayload) => void): Promise<UnlistenFn> =>
    listen<ActivityHidePayload>(EVENTS.activityHide, (e) => cb(e.payload)),
  agentSessions: (cb: (payload: AgentSessions) => void): Promise<UnlistenFn> =>
    listen<AgentSessions>(EVENTS.agentSessions, (e) => cb(e.payload)),
  agentRuns: (cb: (payload: AgentRuns) => void): Promise<UnlistenFn> =>
    listen<AgentRuns>(EVENTS.agentRuns, (e) => cb(e.payload)),
  /** Delivered to the command window only. */
  agentOutput: (cb: (payload: AgentOutput) => void): Promise<UnlistenFn> =>
    listen<AgentOutput>(EVENTS.agentOutput, (e) => cb(e.payload)),
  /** Command window only (2.10a): Claude's text so far while it's written; the run's next entry replaces it. */
  agentTextDraft: (cb: (payload: AgentTextDraft) => void): Promise<UnlistenFn> =>
    listen<AgentTextDraft>(EVENTS.agentTextDraft, (e) => cb(e.payload)),
  /** Command window only: which run to show. */
  commandFocusRun: (cb: (runId: string) => void): Promise<UnlistenFn> =>
    listen<string>(EVENTS.commandFocusRun, (e) => cb(e.payload)),
  /** Command window only: which held session request to show. */
  commandFocusAsk: (cb: (requestId: number) => void): Promise<UnlistenFn> =>
    listen<number>(EVENTS.commandFocusAsk, (e) => cb(e.payload)),
  sessionAsks: (cb: (payload: SessionAsks) => void): Promise<UnlistenFn> =>
    listen<SessionAsks>(EVENTS.sessionAsks, (e) => cb(e.payload)),
  routinesChanged: (cb: (payload: Routines) => void): Promise<UnlistenFn> =>
    listen<Routines>(EVENTS.routinesChanged, (e) => cb(e.payload)),
  googleState: (cb: (payload: GoogleState) => void): Promise<UnlistenFn> =>
    listen<GoogleState>(EVENTS.googleState, (e) => cb(e.payload)),
  calendarUpcoming: (cb: (payload: CalendarUpcoming) => void): Promise<UnlistenFn> =>
    listen<CalendarUpcoming>(EVENTS.calendarUpcoming, (e) => cb(e.payload)),
  /** Every 2 s while the panel is open and `processes.enabled` is on. */
  processesHeavy: (cb: (payload: HeavyProcesses) => void): Promise<UnlistenFn> =>
    listen<HeavyProcesses>(EVENTS.processesHeavy, (e) => cb(e.payload)),
};

/** Unsubscribes listeners registered with `on.*` (safe to call before they resolve). */
export function unlistenAll(pending: Promise<UnlistenFn>[]) {
  for (const p of pending) void p.then((unlisten) => unlisten());
}

export function errorMessage(err: unknown): string {
  if (typeof err === "object" && err !== null && "message" in err) {
    return String((err as AppError).message);
  }
  return String(err);
}
