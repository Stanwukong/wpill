// Typed IPC layer: the only place command and event names appear on the TS side.
// Mirrors src-tauri/src/ipc.rs; change both together.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ActivityHidePayload,
  AgentEntry,
  AgentOptions,
  AgentOutput,
  AgentRuns,
  AgentStartRequest,
  ActivityPayload,
  Routine,
  RoutineDraft,
  Routines,
  AppError,
  ApprovalPreview,
  ClaudeAsks,
  QuestionAnswer,
  ClaudeHooksStatus,
  ClaudeSessions,
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
  claudeSessions: "claude:sessions",
  agentRuns: "agent:runs",
  agentOutput: "agent:output",
  commandFocusRun: "command:focus-run",
  commandFocusAsk: "command:focus-ask",
  claudeAsks: "claude:asks",
  routinesChanged: "routines:changed",
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
  getClaudeSessions: () => invoke<ClaudeSessions>("get_claude_sessions"),
  /** Brings the session's terminal or editor to the front and acknowledges its activity. */
  claudeFocusSession: (sessionId: number) => invoke<void>("claude_focus_session", { sessionId }),
  /** Answers a terminal session's permission request (2.8a). Errors `agentApprovalGone` once it's no longer held. */
  claudeDecide: (requestId: number, answer: "allow" | "deny") => invoke<void>("claude_decide", { requestId, answer }),
  /** Session requests the island holds (2.8b), for the command window. */
  claudeAsks: () => invoke<ClaudeAsks>("claude_asks"),
  /** A held session request's whole diff or command; `null` once it's answered or handed back. */
  claudeAskPreview: (requestId: number) => invoke<ApprovalPreview | null>("claude_ask_preview", { requestId }),
  /** One question of a held session question (2.8c). Errors `agentInvalid` (not an option), `agentApprovalGone`. */
  claudeAnswer: (requestId: number, answer: QuestionAnswer) => invoke<void>("claude_answer", { requestId, answer }),
  /** Approve a held session plan, or send it back with feedback (required then). */
  claudePlan: (requestId: number, approve: boolean, feedback?: string) =>
    invoke<void>("claude_plan", { requestId, approve, feedback: feedback ?? null }),
  claudeHooksStatus: () => invoke<ClaudeHooksStatus>("claude_hooks_status"),
  claudeHooksInstall: () => invoke<void>("claude_hooks_install"),
  claudeHooksRemove: () => invoke<void>("claude_hooks_remove"),
  agentStart: (request: AgentStartRequest) => invoke<{ runId: string }>("agent_start", { request }),
  /** A follow-up: queued into the running turn, or resumes a finished run. */
  agentSend: (runId: string, text: string) => invoke<void>("agent_send", { runId, text }),
  agentStop: (runId: string) => invoke<void>("agent_stop", { runId }),
  agentRuns: () => invoke<AgentRuns>("agent_runs"),
  agentTranscript: (runId: string) => invoke<AgentEntry[]>("agent_transcript", { runId }),
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
  claudeSessions: (cb: (payload: ClaudeSessions) => void): Promise<UnlistenFn> =>
    listen<ClaudeSessions>(EVENTS.claudeSessions, (e) => cb(e.payload)),
  agentRuns: (cb: (payload: AgentRuns) => void): Promise<UnlistenFn> =>
    listen<AgentRuns>(EVENTS.agentRuns, (e) => cb(e.payload)),
  /** Delivered to the command window only. */
  agentOutput: (cb: (payload: AgentOutput) => void): Promise<UnlistenFn> =>
    listen<AgentOutput>(EVENTS.agentOutput, (e) => cb(e.payload)),
  /** Command window only: which run to show. */
  commandFocusRun: (cb: (runId: string) => void): Promise<UnlistenFn> =>
    listen<string>(EVENTS.commandFocusRun, (e) => cb(e.payload)),
  /** Command window only: which held session request to show. */
  commandFocusAsk: (cb: (requestId: number) => void): Promise<UnlistenFn> =>
    listen<number>(EVENTS.commandFocusAsk, (e) => cb(e.payload)),
  claudeAsks: (cb: (payload: ClaudeAsks) => void): Promise<UnlistenFn> =>
    listen<ClaudeAsks>(EVENTS.claudeAsks, (e) => cb(e.payload)),
  routinesChanged: (cb: (payload: Routines) => void): Promise<UnlistenFn> =>
    listen<Routines>(EVENTS.routinesChanged, (e) => cb(e.payload)),
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
