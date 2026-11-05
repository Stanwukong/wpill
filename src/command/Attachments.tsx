import { useCallback, useEffect, useState, type ClipboardEvent, type MutableRefObject } from "react";
import { commands, errorMessage } from "../lib/ipc";
import type { AgentAttachment, AttachmentPick } from "../lib/types";
import { formatBytes, mergePicks } from "./attachmentList";

/**
 * Attachments (2.10b.4): files picked for the next message (the Attach button, Ctrl+V of files or an image, or a
 * drop on the window). Only paths cross IPC; Rust describes them and checks the caps when the message is sent.
 */

/** Where a drop on the window goes: set by the composer that shows (the open run's, else the new-task form's). */
export type DropInto = MutableRefObject<((paths: string[]) => void) | null>;

export function useAttachments(picking: MutableRefObject<boolean>, onError: (message: string) => void) {
  const [items, setItems] = useState<AttachmentPick[]>([]);

  const add = useCallback((picks: AttachmentPick[]) => {
    if (picks.length > 0) setItems((list) => mergePicks(list, picks));
  }, []);

  const addPaths = useCallback(
    (paths: string[]) => {
      if (paths.length === 0) return;
      commands.agentDescribeAttachments(paths).then(add, (err: unknown) => onError(errorMessage(err)));
    },
    [add, onError],
  );

  /** The shell's file picker; Esc belongs to it while it's open. */
  const pick = async () => {
    picking.current = true;
    try {
      add(await commands.agentPickAttachments());
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      picking.current = false;
    }
  };

  /** Files or an image on the clipboard become chips (Rust reads them); text pastes as usual. */
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const data = e.clipboardData;
    if (!data.types.includes("Files") && data.files.length === 0) return;
    e.preventDefault();
    commands.agentPasteAttachments().then(add, (err: unknown) => onError(errorMessage(err)));
  };

  const remove = (path: string) => setItems((list) => list.filter((p) => p.path !== path));
  const clear = () => setItems([]);

  return { items, addPaths, pick, onPaste, remove, clear };
}

/** While `active`, drops on the window come here. */
export function useDropTarget(dropInto: DropInto, active: boolean, addPaths: (paths: string[]) => void) {
  useEffect(() => {
    if (!active) return;
    dropInto.current = addPaths;
    return () => {
      if (dropInto.current === addPaths) dropInto.current = null;
    };
  }, [dropInto, active, addPaths]);
}

/** Chips: a glyph, the name (ellipsized), the size; × removes one when `onRemove` is given (read-only in the transcript). */
export function AttachmentChips({
  items,
  onRemove,
}: {
  items: readonly (AgentAttachment & { path?: string })[];
  onRemove?: (path: string) => void;
}) {
  if (items.length === 0) return null;
  return (
    <ul className="attach-chips" aria-label="Attachments">
      {items.map((a, i) => (
        <li key={a.path ?? `${i}:${a.name}`} className="attach-chip" data-kind={a.kind} title={a.path ?? a.name}>
          {a.kind === "image" ? <ImageGlyph /> : <FileGlyph />}
          <span className="attach-chip__name">{a.name}</span>
          <span className="attach-chip__size">{formatBytes(a.bytes)}</span>
          {onRemove && a.path != null && (
            <button
              type="button"
              className="attach-chip__remove"
              aria-label={`Remove ${a.name}`}
              title="Remove"
              onClick={() => onRemove(a.path!)}
            >
              <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" aria-hidden="true">
                <path d="M5 5l6 6M11 5l-6 6" />
              </svg>
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

/** The quiet Attach button: a paperclip, with its word where there's room (`label`). */
export function AttachButton({ label = false, onClick }: { label?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      className={label ? "cmd-btn cmd-btn--text attach-btn" : "attach-btn attach-btn--icon"}
      aria-label="Attach files"
      title="Attach files (or paste, or drop them here)"
      onClick={onClick}
    >
      <PaperclipGlyph />
      {label && "Attach"}
    </button>
  );
}

/** The window's drop overlay while files hover it: a hairline amber ring and a word, no motion beyond a fade. */
export function DropOverlay() {
  return (
    <div className="drop-overlay" aria-hidden="true">
      <span className="drop-overlay__label">
        <PaperclipGlyph />
        Drop to attach
      </span>
    </div>
  );
}

function PaperclipGlyph() {
  return (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12.6 7.6 8 12.2a3 3 0 0 1-4.2-4.2L8.6 3.2a2 2 0 0 1 2.8 2.8L6.7 10.7a1 1 0 0 1-1.4-1.4l4.3-4.3" />
    </svg>
  );
}

function ImageGlyph() {
  return (
    <svg className="attach-chip__glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="2.5" y="3" width="11" height="10" rx="2" />
      <circle cx="6" cy="6.5" r="1.1" />
      <path d="M3 11.6l3.2-3 2.6 2.3 1.7-1.5 2.8 2.5" />
    </svg>
  );
}

function FileGlyph() {
  return (
    <svg className="attach-chip__glyph" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 2.5h4.8L12 5.7v7.8H4z" />
      <path d="M8.5 2.7v3.3H12" />
    </svg>
  );
}
