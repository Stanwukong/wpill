// Typed IPC layer: the only place command and event names appear on the TS side.
// Mirrors src-tauri/src/ipc.rs; change both together.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppError, IslandState, VisibilityPayload, WindowInfo } from "./types";

const EVENTS = {
  hoverEnter: "island:hover-enter",
  hoverLeave: "island:hover-leave",
  visibility: "island:visibility",
} as const;

export const commands = {
  listWindows: () => invoke<WindowInfo[]>("list_windows"),
  activateWindow: (hwnd: number) => invoke<void>("activate_window", { hwnd }),
  /** Physical px: panel width and its bottom edge from the window top. */
  setPanelRect: (width: number, bottom: number) => invoke<void>("set_panel_rect", { width, bottom }),
  requestCollapse: () => invoke<void>("request_collapse"),
  getIslandState: () => invoke<IslandState>("get_island_state"),
};

export const on = {
  hoverEnter: (cb: () => void): Promise<UnlistenFn> => listen(EVENTS.hoverEnter, () => cb()),
  hoverLeave: (cb: () => void): Promise<UnlistenFn> => listen(EVENTS.hoverLeave, () => cb()),
  visibility: (cb: (payload: VisibilityPayload) => void): Promise<UnlistenFn> =>
    listen<VisibilityPayload>(EVENTS.visibility, (e) => cb(e.payload)),
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
