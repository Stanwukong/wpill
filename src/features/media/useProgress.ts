import { useEffect, useRef, useState, type RefObject } from "react";
import type { MediaSession } from "../../lib/types";

/** Mirrors `expected_position` in src-tauri/src/media/logic.rs. */
export function positionAt(s: MediaSession, nowMs: number): number {
  if (s.status !== "playing") return s.positionMs;
  const pos = s.positionMs + Math.max(0, nowMs - s.updatedAtMs) * s.playbackRate;
  return s.durationMs === null ? Math.max(0, pos) : Math.min(Math.max(0, pos), s.durationMs);
}

/**
 * Interpolates the position between Rust's (sparse) timeline updates. The bar is moved on every
 * animation frame through `barRef`; React re-renders only when the displayed second changes.
 * Runs only while the card is mounted, i.e. while the panel is open.
 */
export function useProgress(session: MediaSession, barRef: RefObject<HTMLElement | null>): number {
  const [second, setSecond] = useState(() => Math.floor(positionAt(session, Date.now()) / 1000));
  const sessionRef = useRef(session);
  sessionRef.current = session;

  useEffect(() => {
    let frame = 0;
    const tick = () => {
      const s = sessionRef.current;
      const pos = positionAt(s, Date.now());
      if (barRef.current) {
        const fraction = s.durationMs ? pos / s.durationMs : 0;
        barRef.current.style.transform = `scaleX(${fraction})`;
      }
      setSecond(Math.floor(pos / 1000));
      frame = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, [barRef]);

  return second * 1000;
}

export function formatTime(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}
