import type { ReactNode } from "react";

/**
 * A view's empty, loading or error line (2.10d.4): one quiet sentence, an error in `--danger` with its detail as a
 * tooltip, and at most one quiet action (Retry, New task). Loading fades in once after a short delay, so a fast load
 * shows nothing; nothing loops.
 */
export function StateLine({
  tone = "quiet",
  detail,
  action,
  children,
}: {
  tone?: "quiet" | "loading" | "error";
  /** The error as Rust worded it, on hover. */
  detail?: string;
  action?: { label: string; onClick: () => void };
  children: ReactNode;
}) {
  return (
    <div className="cmd-state" data-tone={tone} role={tone === "error" ? "alert" : "status"}>
      <span className="cmd-state__text" title={detail}>
        {children}
      </span>
      {action && (
        <button type="button" className="cmd-btn cmd-btn--quiet cmd-state__action" onClick={action.onClick}>
          {action.label}
        </button>
      )}
    </div>
  );
}
