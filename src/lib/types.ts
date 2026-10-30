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

export type Activity =
  | { kind: "volume"; payload: VolumeActivity }
  | { kind: "brightness"; payload: BrightnessActivity }
  | { kind: "bluetooth"; payload: BluetoothActivity }
  | { kind: "power"; payload: PowerActivity }
  | { kind: "capsLock"; payload: CapsLockActivity }
  | { kind: "screenshot"; payload: ScreenshotActivity }
  | { kind: "claudeDone"; payload: ClaudeDoneActivity }
  | { kind: "claudeNeedsInput"; payload: ClaudeNeedsInputActivity };

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
    | "win32"
    | "tauri";
  message: string;
}
