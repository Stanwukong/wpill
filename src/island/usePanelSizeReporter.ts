import { useEffect, type RefObject } from "react";
import { commands } from "../lib/ipc";

/**
 * While expanded, tells Rust where the panel is so its leave test matches what's on screen.
 * Reports physical px: CSS px × devicePixelRatio, which folds in Windows' text-size zoom
 * (WebView2 applies it; Rust's monitor scale factor doesn't include it).
 * Uses offset* (layout box), which ignores Framer Motion's in-flight transforms.
 */
export function usePanelSizeReporter(ref: RefObject<HTMLElement | null>, expanded: boolean) {
  useEffect(() => {
    const el = ref.current;
    if (!expanded || !el) return;

    let last = { width: 0, bottom: 0 };
    const report = () => {
      // The first observation can still see the pill (DOM not yet updated); never report that.
      if (el.dataset.expanded !== "true") return;
      const dpr = window.devicePixelRatio;
      const rect = { width: el.offsetWidth * dpr, bottom: (el.offsetTop + el.offsetHeight) * dpr };
      if (Math.abs(rect.width - last.width) < 1 && Math.abs(rect.bottom - last.bottom) < 1) return;
      last = rect;
      void commands.setPanelRect(rect.width, rect.bottom);
    };

    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, expanded]);
}
