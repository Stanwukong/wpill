import type { AgentKind } from "../../lib/types";
import "./sessions.css";

/** True when any of these belongs to an agent other than Claude Code: then every session says whose it is (2.9a). */
export function mixedAgents(items: readonly { agent: AgentKind }[]): boolean {
  return items.some((i) => i.agent !== "claude");
}

/**
 * Which agent a session belongs to, as a quiet lowercase word after the project name (D47): plain text, no mark and
 * no per-agent colour (D30). Hidden while every session is Claude Code's, so a Claude-only setup looks as before.
 */
export function AgentTag({ agent, mixed }: { agent: AgentKind; mixed: boolean }) {
  if (agent === "claude" && !mixed) return null;
  return <span className="agent-tag">{agent}</span>;
}
