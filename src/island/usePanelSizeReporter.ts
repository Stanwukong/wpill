import { useEffect, type RefObject } from "react";
import { commands, type RectMode } from "../lib/ipc";

/**
 * While expanded or showing an activity, tells Rust where the island is, so its hit tests (leaving the
 * panel, hovering an activity) match what's on screen. Each report names its shape; Rust ignores
 * reports for a shape it isn't showing.
 * Reports physical px: CSS px × devicePixelRatio, which folds in Windows' text-size zoom
 * (WebView2 applies it; Rust's monitor scale factor doesn't include it).
 * Uses offset* (layout box), which ignores Framer Motion's in-flight transforms.
 */
export function usePanelSizeReporter(ref: RefObject<HTMLElement | null>, mode: RectMode | null) {
  useEffect(() => {
    const el = ref.current;
    if (!mode || !el) return;

    let last = { width: 0, bottom: 0 };
    const report = () => {
      // The first observation can still see the previous shape (DOM not yet updated); never report that.
      if (el.dataset.mode !== mode) return;
      const dpr = window.devicePixelRatio;
      const rect = { width: el.offsetWidth * dpr, bottom: (el.offsetTop + el.offsetHeight) * dpr };
      if (Math.abs(rect.width - last.width) < 1 && Math.abs(rect.bottom - last.bottom) < 1) return;
      last = rect;
      void commands.setPanelRect(rect.width, rect.bottom, mode);
    };

    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, mode]);
}
