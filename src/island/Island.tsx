import { useRef, type CSSProperties } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ActivityView } from "../features/activities/ActivityView";
import { AgentRunItems } from "../features/agent/AgentRunItems";
import { isLive, recentRuns, useAgentRuns } from "../features/agent/useAgentRuns";
import { AppsList } from "../features/apps/AppsList";
import { ClaudeSpinner } from "../features/claude/ClaudeGlyphs";
import { ClaudeSection } from "../features/claude/ClaudeSection";
import { useClaudeSessions } from "../features/claude/useClaudeSessions";
import { useArtTheme } from "../features/media/artTheme";
import { MediaCard } from "../features/media/MediaCard";
import { Equalizer, PillArt } from "../features/media/PillMedia";
import { useMedia } from "../features/media/useMedia";
import { Clock } from "./Clock";
import { ACTIVITY_RADIUS, contentFade, islandSpring, PANEL_RADIUS, PILL_RADIUS } from "./motion";
import { useIslandState } from "./useIslandState";
import { usePanelSizeReporter } from "./usePanelSizeReporter";
import type { ActivityPayload } from "../lib/types";
import "./island.css";

const RADIUS = { pill: PILL_RADIUS, activity: ACTIVITY_RADIUS, panel: PANEL_RADIUS } as const;

export function Island() {
  const { expanded, hidden, activity } = useIslandState();
  const mode = expanded ? "panel" : activity ? "activity" : "pill";
  // Drives the orbiting charge ring on the island's edge (features/activities/activities.css).
  const charging = activity?.kind === "power" && activity.payload.event === "pluggedIn";
  // Drives the Claude orange edge (activities.css): steady when done or acting, breathing while it needs the user.
  const claude =
    activity?.kind === "claudeDone" || activity?.kind === "agentFinished" || activity?.kind === "agentActing"
      ? "done"
      : activity?.kind === "claudeNeedsInput" || activity?.kind === "agentApproval"
        ? "needsInput"
        : undefined;
  const ref = useRef<HTMLDivElement>(null);
  usePanelSizeReporter(ref, mode === "pill" ? null : mode);
  const media = useMedia();
  const claudeSessions = useClaudeSessions();
  const agentRuns = useAgentRuns();
  const claudeWorking = claudeSessions.some((s) => s.status === "working") || agentRuns.some((r) => isLive(r.status));
  // Computed at render (no timer): the panel remounts on every expand and re-renders on run events.
  const panelRuns = expanded ? recentRuns(agentRuns) : [];
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
        data-claude={claude}
        transition={islandSpring}
        style={{ borderRadius: RADIUS[mode] }}
      >
        <AnimatePresence mode="popLayout" initial={false}>
          {mode === "activity" && activity ? (
            // Keyed by kind: an in-place update (another volume step) re-renders without a crossfade. Claude's two
            // kinds share a key per session, so needs-input → done for the same session switches in place. Agent
            // activities are keyed per approval / run (activityKey).
            <motion.div
              key={activityKey(activity)}
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
              <ClaudeSection
                sessions={claudeSessions}
                agent={panelRuns.length > 0 ? <AgentRunItems runs={panelRuns} /> : null}
              />
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
              {claudeWorking && <ClaudeSpinner />}
              <Clock variant="pill" />
              {nowPlaying && <Equalizer />}
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </div>
  );
}

function activityKey(activity: ActivityPayload): string {
  if (activity.kind === "claudeDone" || activity.kind === "claudeNeedsInput") {
    return `activity-claude-${activity.payload.sessionId}`;
  }
  // Each approval is its own card (the next one crossfades in); the finished notice never morphs out of one.
  if (activity.kind === "agentApproval") return `activity-agent-approval-${activity.payload.requestId}`;
  // Per run, so waiting → acting (same activity) updates in place.
  if (activity.kind === "agentActing") return `activity-agent-acting-${activity.payload.runId}`;
  if (activity.kind === "agentFinished") return `activity-agent-finished-${activity.payload.runId}`;
  return `activity-${activity.kind}`;
}
