import { useRef } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AppsList } from "../features/apps/AppsList";
import { Clock } from "./Clock";
import { contentFade, islandSpring, PANEL_RADIUS, PILL_RADIUS } from "./motion";
import { useIslandState } from "./useIslandState";
import { usePanelSizeReporter } from "./usePanelSizeReporter";
import "./island.css";

export function Island() {
  const { expanded, hidden } = useIslandState();
  const ref = useRef<HTMLDivElement>(null);
  usePanelSizeReporter(ref, expanded);

  if (hidden) return null;

  return (
    <div className="stage">
      <motion.div
        ref={ref}
        layout
        className="island"
        data-expanded={expanded}
        transition={islandSpring}
        style={{ borderRadius: expanded ? PANEL_RADIUS : PILL_RADIUS }}
      >
        <AnimatePresence mode="popLayout" initial={false}>
          {expanded ? (
            <motion.div
              key="panel"
              className="island__panel"
              layout="position"
              initial={{ opacity: 0, filter: "blur(6px)" }}
              animate={{ opacity: 1, filter: "blur(0px)" }}
              exit={{ opacity: 0 }}
              transition={contentFade}
            >
              <header className="island__header">
                <Clock variant="panel" />
              </header>
              <AppsList />
            </motion.div>
          ) : (
            <motion.div
              key="pill"
              className="island__pill"
              layout="position"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={contentFade}
            >
              <Clock variant="pill" />
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </div>
  );
}
