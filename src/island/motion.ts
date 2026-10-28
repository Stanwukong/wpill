import type { Transition } from "framer-motion";

/** Gap above the island. Mirrors `PANEL_TOP` in src-tauri/src/window/mod.rs. */
export const PANEL_TOP_PX = 6;
/** Expanded width. Must fit inside `WINDOW_WIDTH` (680) in src-tauri/src/window/mod.rs, with room for the shadow. */
export const PANEL_WIDTH_PX = 640;

export const PILL_RADIUS = 18;
export const PANEL_RADIUS = 28;

/** Pill ⇄ panel morph. */
export const islandSpring: Transition = { type: "spring", stiffness: 420, damping: 34, mass: 0.9 };

/** Content crossfade inside the island. */
export const contentFade: Transition = { duration: 0.16, ease: "easeOut" };
