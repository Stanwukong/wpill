import { useRef, type CSSProperties } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ActivityView } from "../features/activities/ActivityView";
import { AppsList } from "../features/apps/AppsList";
import { useArtTheme } from "../features/media/artTheme";
import { MediaCard } from "../features/media/MediaCard";
import { Equalizer, PillArt } from "../features/media/PillMedia";
import { useMedia } from "../features/media/useMedia";
import { Clock } from "./Clock";
import { ACTIVITY_RADIUS, contentFade, islandSpring, PANEL_RADIUS, PILL_RADIUS } from "./motion";
import { useIslandState } from "./useIslandState";
import { usePanelSizeReporter } from "./usePanelSizeReporter";
import "./island.css";

const RADIUS = { pill: PILL_RADIUS, activity: ACTIVITY_RADIUS, panel: PANEL_RADIUS } as const;

export function Island() {
  const { expanded, hidden, activity } = useIslandState();
  const mode = expanded ? "panel" : activity ? "activity" : "pill";
  // Drives the orbiting charge ring on the island's edge (features/activities/activities.css).
  const charging = activity?.kind === "power" && activity.payload.event === "pluggedIn";
  const ref = useRef<HTMLDivElement>(null);
  usePanelSizeReporter(ref, mode === "pill" ? null : mode);
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
        data-mode={mode}
        data-charging={charging}
        transition={islandSpring}
        style={{ borderRadius: RADIUS[mode] }}
      >
        <AnimatePresence mode="popLayout" initial={false}>
          {mode === "activity" && activity ? (
            // Keyed by kind: an in-place update (another volume step) re-renders without a crossfade.
            <motion.div
              key={`activity-${activity.kind}`}
              className="island__activity"
              layout="position"
              initial={{ opacity: 0, filter: "blur(4px)" }}
              animate={{ opacity: 1, filter: "blur(0px)" }}
              exit={{ opacity: 0 }}
              transition={contentFade}
            >
              <ActivityView activity={activity} />
            </motion.div>
          ) : expanded ? (
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
