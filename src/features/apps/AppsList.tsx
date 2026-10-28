import { motion } from "framer-motion";
import type { WindowInfo } from "../../lib/types";
import { useRunningApps } from "./useRunningApps";
import "./apps.css";

export function AppsList() {
  const { apps, error, activate } = useRunningApps();

  return (
    <section className="apps" aria-label="Running apps">
      <h2 className="apps__heading">Running apps</h2>
      {error && <p className="apps__error">{error}</p>}
      {apps === null ? null : apps.length === 0 ? (
        <p className="apps__empty">No open windows</p>
      ) : (
        <ul className="apps__grid">
          {apps.map((app, i) => (
            <motion.li
              key={app.hwnd}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: Math.min(i, 12) * 0.015, duration: 0.18 }}
            >
              <AppTile app={app} onActivate={activate} />
            </motion.li>
          ))}
        </ul>
      )}
    </section>
  );
}

function AppTile({ app, onActivate }: { app: WindowInfo; onActivate: (hwnd: number) => void }) {
  return (
    <button className="app-tile" title={app.exeName ? `${app.title}\n${app.exeName}` : app.title} onClick={() => onActivate(app.hwnd)}>
      {app.icon ? (
        <img className="app-tile__icon" src={app.icon} alt="" draggable={false} />
      ) : (
        <span className="app-tile__icon app-tile__icon--fallback" aria-hidden>
          {app.title.charAt(0).toUpperCase()}
        </span>
      )}
      <span className="app-tile__title">{app.title}</span>
    </button>
  );
}
