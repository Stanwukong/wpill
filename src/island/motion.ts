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
export const claudeStatusIn: Transition = { duration: 0.32, ease: [0.22, 1, 0.36, 1] };

/** Claude activities: the check draws at the right end, a beat after the status text starts. */
export const claudeCheckDraw: Transition = {
  pathLength: { duration: 0.36, delay: 0.12, ease: [0.33, 1, 0.68, 1] },
  opacity: { duration: 0.08, delay: 0.12 },
};

/** Claude activities: the waiting dots fade out (faster than things come in) when needs-input turns into done. */
export const claudeDotsOut: Transition = { duration: 0.12, ease: "easeIn" };

/** Agent acting: the hint line crossfades in place when waiting turns into acting (the row itself stays). */
export const actingHintIn: Transition = { duration: 0.2, ease: "easeOut" };
