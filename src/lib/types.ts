// Mirrors the payload structs in src-tauri/src/ipc.rs and src-tauri/src/error.rs.

export interface IslandState {
  expanded: boolean;
  hidden: boolean;
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

export interface AppError {
  kind: "windowGone" | "foregroundDenied" | "noIslandWindow" | "noMonitor" | "win32" | "tauri";
  message: string;
}
