/**
 * Unsent follow-up text per run, kept while the command window lives (2.10d): `RunView` remounts per run, so the
 * composer reads its draft from here and writes back on every change. Memory only; attachments aren't kept.
 */
export interface DraftStore {
  get(runId: string): string | undefined;
  /** Empty or whitespace-only text deletes the draft. */
  set(runId: string, text: string): void;
  clear(runId: string): void;
  readonly size: number;
}

/** A store holding at most `cap` drafts; the one written longest ago goes first. */
export function draftStore(cap = 50): DraftStore {
  // A Map keeps insertion order: re-inserting on write makes its first key the oldest.
  const drafts = new Map<string, string>();
  return {
    get: (runId) => drafts.get(runId),
    set(runId, text) {
      drafts.delete(runId);
      if (text.trim() === "") return;
      drafts.set(runId, text);
      while (drafts.size > cap) {
        const oldest = drafts.keys().next().value;
        if (oldest === undefined) break;
        drafts.delete(oldest);
      }
    },
    clear(runId) {
      drafts.delete(runId);
    },
    get size() {
      return drafts.size;
    },
  };
}

/** The command window's drafts. */
export const composerDrafts = draftStore();
