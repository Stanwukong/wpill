import { useRef, type CSSProperties } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AppsList } from "../features/apps/AppsList";
import { useArtTheme } from "../features/media/artTheme";
import { MediaCard } from "../features/media/MediaCard";
import { Equalizer, PillArt } from "../features/media/PillMedia";
import { useMedia } from "../features/media/useMedia";
import { Clock } from "./Clock";
import { contentFade, islandSpring, PANEL_RADIUS, PILL_RADIUS } from "./motion";
import { useIslandState } from "./useIslandState";
import { usePanelSizeReporter } from "./usePanelSizeReporter";
import "./island.css";

export function Island() {
  const { expanded, hidden } = useIslandState();
  const ref = useRef<HTMLDivElement>(null);
  usePanelSizeReporter(ref, expanded);
  const media = useMedia();
  const nowPlaying = media.active?.status === "playing" ? media.active : null;
  // Tinted while there is an active session with art (playing or paused); plain black otherwise.
  const theme = useArtTheme(media.active?.artwork ?? null);
  const themeVars = theme
    ? ({ "--tint-from": theme.from, "--tint-to": theme.to, "--accent": theme.accent } as CSSProperties)
    : undefined;

  if (hidden) return null;

  return (
    <div className="stage" style={themeVars}>
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
              <MediaCard media={media} />
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
              {nowPlaying && <PillArt session={nowPlaying} />}
              <Clock variant="pill" />
              {nowPlaying && <Equalizer />}
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </div>
  );
}
