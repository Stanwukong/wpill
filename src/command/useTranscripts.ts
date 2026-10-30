import { useCallback, useEffect, useReducer, useRef } from "react";
import { commands, errorMessage, on, unlistenAll } from "../lib/ipc";
import type { AgentEntry } from "../lib/types";

export interface Transcript {
  entries: AgentEntry[];
  loading: boolean;
  error: string | null;
}

interface Slot extends Transcript {
  /** Live entries that arrived while the snapshot was loading. */
  buffered: AgentEntry[];
}

/**
 * Per-run transcripts: a run's snapshot (`agentTranscript`) is loaded the first time it's shown, then live
 * `agent:output` entries are appended. Entries for runs not loaded yet are skipped; their snapshot will hold them.
 * Entries that arrive while a snapshot loads are kept and appended unless the snapshot already ends with them.
 */
export function useTranscripts() {
  const store = useRef(new Map<string, Slot>());
  const [, bump] = useReducer((n: number) => n + 1, 0);

  useEffect(() => {
    const subscriptions = [
      on.agentOutput(({ runId, entry }) => {
        const slot = store.current.get(runId);
        if (!slot) return;
        if (slot.loading) slot.buffered.push(entry);
        else slot.entries = [...slot.entries, entry];
        bump();
      }),
    ];
    return () => unlistenAll(subscriptions);
  }, []);

  const load = useCallback((runId: string) => {
    if (store.current.has(runId)) return;
    const slot: Slot = { entries: [], loading: true, error: null, buffered: [] };
    store.current.set(runId, slot);
    bump();
    commands
      .agentTranscript(runId)
      .then((entries) => {
        const tail = new Set(entries.slice(-64).map((e) => JSON.stringify(e)));
        slot.entries = [...entries, ...slot.buffered.filter((e) => !tail.has(JSON.stringify(e)))];
      })
      .catch((err: unknown) => {
        slot.error = errorMessage(err);
        slot.entries = slot.buffered;
      })
      .finally(() => {
        slot.loading = false;
        slot.buffered = [];
        bump();
      });
  }, []);

  const get = useCallback((runId: string): Transcript | undefined => store.current.get(runId), []);

  return { get, load };
}
