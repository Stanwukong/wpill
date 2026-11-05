import type { Transition } from "framer-motion";

/** Gap above the island. Mirrors `PANEL_TOP` in src-tauri/src/window/mod.rs. */
export const PANEL_TOP_PX = 6;
/** Expanded width. Must fit inside `WINDOW_WIDTH` (680) in src-tauri/src/window/mod.rs, with room for the shadow. */
export const PANEL_WIDTH_PX = 640;

export const PILL_RADIUS = 18;
export const PANEL_RADIUS = 28;
export const ACTIVITY_RADIUS = 22;

/** Pill ⇄ panel morph. */
export const islandSpring: Transition = { type: "spring", stiffness: 420, damping: 34, mass: 0.9 };

/** Content crossfade inside the island. */
export const contentFade: Transition = { duration: 0.16, ease: "easeOut" };

/** Claude activities: "Done"/"Stopped" slides out from behind the project name (ease-out quint). */
export const sessionStatusIn: Transition = { duration: 0.32, ease: [0.22, 1, 0.36, 1] };

/** Claude activities: the check draws at the right end, a beat after the status text starts. */
export const sessionCheckDraw: Transition = {
  pathLength: { duration: 0.36, delay: 0.12, ease: [0.33, 1, 0.68, 1] },
  opacity: { duration: 0.08, delay: 0.12 },
};

/** Claude activities: the waiting dots fade out (faster than things come in) when needs-input turns into done. */
export const sessionDotsOut: Transition = { duration: 0.12, ease: "easeIn" };

/** Agent acting: the hint line crossfades in place when waiting turns into acting (the row itself stays). */
export const actingHintIn: Transition = { duration: 0.2, ease: "easeOut" };

/** Memory rows (Phase 3): a row that changes place on a refresh glides there; well inside the 2 s refresh. */
export const memoryReorder: Transition = { duration: 0.32, ease: [0.22, 1, 0.36, 1] };

/** Memory rows: a new app in the list fades in (like the running apps' tiles, without the rise). */
export const memoryRowIn: Transition = { duration: 0.18, ease: "easeOut" };

/** Memory rows (3b): an app that was ended fades out, faster than rows come in; the rest then glide up (memoryReorder). */
export const memoryRowOut: Transition = { duration: 0.14, ease: "easeIn" };
