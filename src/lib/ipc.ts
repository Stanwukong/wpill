// Typed IPC layer: the only place command and event names appear on the TS side.
// Mirrors src-tauri/src/ipc.rs; change both together.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ActivityHidePayload,
  ActivityPayload,
  AppError,
  IslandState,
  MediaState,
  VisibilityPayload,
  WindowInfo,
} from "./types";

const EVENTS = {
  hoverEnter: "island:hover-enter",
  hoverLeave: "island:hover-leave",
  visibility: "island:visibility",
  mediaState: "media:state",
  activityShow: "activity:show",
  activityHide: "activity:hide",
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
};

export const on = {
  hoverEnter: (cb: () => void): Promise<UnlistenFn> => listen(EVENTS.hoverEnter, () => cb()),
  hoverLeave: (cb: () => void): Promise<UnlistenFn> => listen(EVENTS.hoverLeave, () => cb()),
  visibility: (cb: (payload: VisibilityPayload) => void): Promise<UnlistenFn> =>
    listen<VisibilityPayload>(EVENTS.visibility, (e) => cb(e.payload)),
  mediaState: (cb: (payload: MediaState) => void): Promise<UnlistenFn> =>
    listen<MediaState>(EVENTS.mediaState, (e) => cb(e.payload)),
  activityShow: (cb: (payload: ActivityPayload) => void): Promise<UnlistenFn> =>
    listen<ActivityPayload>(EVENTS.activityShow, (e) => cb(e.payload)),
  activityHide: (cb: (payload: ActivityHidePayload) => void): Promise<UnlistenFn> =>
    listen<ActivityHidePayload>(EVENTS.activityHide, (e) => cb(e.payload)),
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
