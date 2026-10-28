import { useCallback, useEffect, useState } from "react";
import { commands, errorMessage } from "../../lib/ipc";
import type { WindowInfo } from "../../lib/types";

/** Fetches the Alt+Tab list on mount (the panel mounts on every expand). */
export function useRunningApps() {
  const [apps, setApps] = useState<WindowInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setApps(await commands.listWindows());
    } catch (err) {
      setError(errorMessage(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const activate = useCallback(
    async (hwnd: number) => {
      setError(null);
      try {
        await commands.activateWindow(hwnd); // Rust collapses the island on success
      } catch (err) {
        setError(errorMessage(err));
        void refresh();
      }
    },
    [refresh],
  );

  return { apps, error, activate };
}
