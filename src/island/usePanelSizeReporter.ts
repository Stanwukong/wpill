import { useEffect, type RefObject } from "react";
import { commands, on, unlistenAll, type RectMode } from "../lib/ipc";

/**
 * While expanded or showing an activity, tells Rust where the island is, so its hit tests (leaving the
 * panel, hovering an activity) match what's on screen. Each report names its shape; Rust ignores
 * reports for a shape it isn't showing.
 * Reports physical px: CSS px × devicePixelRatio, which folds in Windows' text-size zoom
 * (WebView2 applies it; Rust's monitor scale factor doesn't include it).
 * Uses offset* (layout box), which ignores Framer Motion's in-flight transforms.
 * Reports again when the pixel ratio changes (a new display scale or Text size) and on `monitor:changed`, since
 * the CSS size, and so the ResizeObserver, can stay the same while the physical size doesn't.
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
    // Rust forgot the old rect, so send it even if this side thinks nothing changed.
    const reportAgain = () => {
      last = { width: 0, bottom: 0 };
      report();
    };

    const observer = new ResizeObserver(report);
    observer.observe(el);

    // A media query on the current ratio fires once when it stops matching; watch the new ratio after that.
    let ratio: MediaQueryList | null = null;
    const watchRatio = () => {
      ratio?.removeEventListener("change", onRatio);
      ratio = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      ratio.addEventListener("change", onRatio);
    };
    const onRatio = () => {
      watchRatio();
      reportAgain();
    };
    watchRatio();

    const pending = [on.monitorChanged(reportAgain)];
    return () => {
      observer.disconnect();
      ratio?.removeEventListener("change", onRatio);
      unlistenAll(pending);
    };
  }, [ref, mode]);
}
