/**
 * Words for a run's transcript (2.10a): the end-of-run summary, decisions, durations, and the whole run as Markdown
 * for "Copy run". Pure; tested with `pnpm test` (`runMarkdown.test.ts`).
 */
import { endLabel } from "../features/agent/labels.ts";
import type { AgentEntry } from "../lib/types.ts";
import { turns, type Decision } from "./transcriptTurns.ts";

type Result = Extract<AgentEntry, { kind: "result" }>;

export function decisionText(d: Decision): string {
  switch (d.by) {
    case "user":
      return d.allow ? "Allowed by you" : "Denied by you";
    case "remembered":
      return d.allow ? "Allowed for this run" : "Denied (remembered)";
    case "timeout":
      return "Denied (timed out)";
    case "runEnded":
      return "Cancelled (run ended)";
    case "answered":
      return "Answered by you";
    case "approved":
      return "Plan approved by you";
    case "feedback":
      return "Sent back with your feedback";
  }
}

/** "0.8 s", "42 s", "3 min 5 s", "1 h 2 min". */
export function formatDuration(ms: number): string {
  if (ms < 10_000) return `${(Math.max(ms, 0) / 1000).toFixed(1)} s`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  if (s < 3600) return s % 60 === 0 ? `${s / 60} min` : `${Math.floor(s / 60)} min ${s % 60} s`;
  const min = Math.floor((s % 3600) / 60);
  return min === 0 ? `${Math.floor(s / 3600)} h` : `${Math.floor(s / 3600)} h ${min} min`;
}

/** The summary's parts: outcome, then turns, time and cost when known. */
export function resultParts(r: Result): { outcome: string; details: string[] } {
  const details = [`${r.turns} ${r.turns === 1 ? "turn" : "turns"}`];
  if (r.durationMs != null) details.push(formatDuration(r.durationMs));
  if (r.costUsd != null) details.push(`$${r.costUsd.toFixed(4)}`);
  return { outcome: endLabel(r.status, r.reason), details };
}

/**
 * The whole run as Markdown: your messages (and what you attached, by name), Claude's text, tool calls as list items,
 * notes and the outcome.
 */
export function toMarkdown(entries: AgentEntry[], title?: string): string {
  const out: string[] = [];
  if (title) out.push(`# ${title}`);
  for (const turn of turns(entries)) {
    if (turn.user) {
      out.push(`## You\n\n${turn.user.text.trim()}`);
      const attached = turn.user.attachments ?? [];
      if (attached.length > 0) out.push(`Attached: ${attached.map((a) => a.name).join(", ")}`);
    }
    if (turn.items.length > 0) out.push("## Claude");
    for (const item of turn.items) {
      switch (item.kind) {
        case "text":
          out.push(item.entry.text.trim());
          break;
        case "tools":
          out.push(
            item.calls
              .map((c) => {
                const line = `- ${c.tool ? code(c.tool.label) : "Output"}`;
                return c.result?.isError ? `${line}\n  - Error: ${firstLine(c.result.text)}` : line;
              })
              .join("\n"),
          );
          break;
        case "approval":
          out.push(
            `> Asked: ${item.entry.tool} ${item.entry.summary}. ${item.decision ? decisionText(item.decision) : "Waiting for you"}.`,
          );
          break;
        case "decision":
          out.push(`> ${decisionText(item.entry)}.`);
          break;
        case "result": {
          const { outcome, details } = resultParts(item.entry);
          out.push(`---\n\n**${outcome}** · ${details.join(" · ")}`);
          break;
        }
        case "error":
          out.push(`> Error: ${item.entry.message.trim()}`);
          break;
      }
    }
  }
  return `${out.join("\n\n")}\n`;
}

/** Inline code that survives backticks inside it. */
function code(text: string): string {
  const longest = Math.max(0, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  const fence = "`".repeat(longest + 1);
  const pad = text.startsWith("`") || text.endsWith("`") ? " " : "";
  return `${fence}${pad}${text}${pad}${fence}`;
}

function firstLine(text: string): string {
  const line = text.trim().split("\n")[0] ?? "";
  return line.length > 200 ? `${line.slice(0, 199)}…` : line;
}
