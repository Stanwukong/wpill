import { useRef, type PointerEvent } from "react";
import { commands } from "../../lib/ipc";
import type { ScreenshotActivity as Shot } from "../../lib/types";

/** Movement (CSS px) that turns a press into a drag rather than a click. */
const DRAG_SLOP = 4;

/**
 * The only interactive activity: Rust makes the island clickable (and pauses the countdown) while the
 * cursor is on it. Dragging hands off to a native shell drag (`activity_drag_screenshot`), which needs
 * the mouse button to still be down, so it starts on the first move past the slop. A click opens the file.
 */
export function ScreenshotActivity({ id, shot }: { id: number; shot: Shot }) {
  const press = useRef<{ x: number; y: number } | null>(null);

  const onPointerDown = (e: PointerEvent) => {
    if (e.button === 0) press.current = { x: e.clientX, y: e.clientY };
  };
  const onPointerMove = (e: PointerEvent) => {
    const start = press.current;
    if (!start || !(e.buttons & 1)) return;
    if (Math.hypot(e.clientX - start.x, e.clientY - start.y) < DRAG_SLOP) return;
    // The native drag loop swallows the pointerup, so forget the press now.
    press.current = null;
    void commands.activityDragScreenshot(id).catch(() => {});
  };
  const onPointerUp = () => {
    if (press.current) void commands.activityOpenScreenshot(id).catch(() => {});
    press.current = null;
  };

  return (
    <div
      className="activity activity--screenshot"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerLeave={() => (press.current = null)}
      title={shot.fileName}
    >
      <img className="activity__thumb" src={shot.thumbnail} alt="" draggable={false} />
      <div className="activity__text">
        <span className="activity__title">Screenshot saved</span>
        <span className="activity__detail">Drag to share · click to open</span>
      </div>
    </div>
  );
}
