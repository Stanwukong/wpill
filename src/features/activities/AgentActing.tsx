import { motion, useReducedMotion } from "framer-motion";
import { actingHintIn } from "../../island/motion";
import type { AgentActingActivity } from "../../lib/types";
import { ClaudeMark } from "../claude/ClaudeGlyphs";
import { PointerIcon } from "./icons";

const HINT: Record<AgentActingActivity["phase"], string> = {
  waiting: "Waiting for you to let go…",
  acting: "Move the mouse or press a key to stop",
};

/**
 * TopBar is about to click or type for an agent run, or is doing it: ✻ · project · step, and a quiet line saying
 * what stops it. Not interactive (the island stays click-through). Keyed per run, so waiting → acting re-renders in
 * place: only the hint crossfades and the pointer at the end turns Claude orange. The edge holds steady
 * (`data-claude="acting"`); nothing loops.
 */
export function AgentActing({ acting }: { acting: AgentActingActivity }) {
  const { project, step, phase } = acting;
  const still = useReducedMotion() ?? false;

  return (
    <div className="activity activity--acting" data-phase={phase}>
      <ClaudeMark />
      <span className="acting__text">
        <span className="claude-activity__line">
          <span className="activity__title">{project}</span>
          <span className="acting__step">{step}</span>
        </span>
        <motion.span
          key={phase}
          className="acting__hint"
          initial={still ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={actingHintIn}
        >
          {HINT[phase]}
        </motion.span>
      </span>
      <PointerIcon />
    </div>
  );
}
