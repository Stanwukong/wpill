/**
 * A run's transcript as turns (2.10a): each message you sent opens a turn, and Claude's side of it is a list of
 * items. Consecutive tool calls fold into one group with a summary ("Read 3 files, ran 2 commands"); each call's
 * result is matched to it by id wherever it arrived. Approvals and answers stay where they happened. Pure; tested
 * with `pnpm test` (`transcriptTurns.test.ts`).
 */
import type { AgentEntry, AgentEvent } from "../lib/types.ts";

type Of<K extends AgentEvent["kind"]> = Extract<AgentEntry, { kind: K }>;
export type Decision = Of<"decision">;

export interface Call {
  /** Missing only for a result whose call isn't in the transcript (cut by the 400-entry cap). */
  tool: Of<"tool"> | null;
  result: Of<"toolResult"> | null;
}

export type TurnItem =
  | { kind: "text"; entry: Of<"text"> }
  | { kind: "tools"; calls: Call[]; summary: string; failed: boolean }
  | { kind: "approval"; entry: Of<"approval">; decision: Decision | null }
  | { kind: "decision"; entry: Decision }
  | { kind: "result"; entry: Of<"result"> }
  | { kind: "error"; entry: Of<"error"> };

export interface Turn {
  /** Missing for what came before the first message (a transcript cut by the cap). */
  user: Of<"user"> | null;
  items: TurnItem[];
}

export function turns(entries: AgentEntry[]): Turn[] {
  const decisions = new Map<number, Decision>();
  const asked = new Set<number>();
  for (const e of entries) {
    if (e.kind === "decision") decisions.set(e.requestId, e);
    if (e.kind === "approval") asked.add(e.requestId);
  }
  const out: Turn[] = [];
  let turn: Turn | null = null;
  let group: Extract<TurnItem, { kind: "tools" }> | null = null;
  let calls = new Map<string, Call>();
  const add = (item: TurnItem) => {
    if (!turn) out.push((turn = { user: null, items: [] }));
    turn.items.push(item);
  };
  for (const e of entries) {
    if (e.kind === "user") {
      out.push((turn = { user: e, items: [] }));
      group = null;
      calls = new Map();
      continue;
    }
    if (e.kind === "tool" || e.kind === "toolResult") {
      if (e.kind === "toolResult") {
        const known = calls.get(e.id);
        if (known) {
          known.result = e;
          continue;
        }
      }
      if (!group) add((group = { kind: "tools", calls: [], summary: "", failed: false }));
      const call: Call = e.kind === "tool" ? { tool: e, result: null } : { tool: null, result: e };
      group.calls.push(call);
      if (e.kind === "tool") calls.set(e.id, call);
      continue;
    }
    group = null;
    switch (e.kind) {
      case "text":
        add({ kind: "text", entry: e });
        break;
      case "approval":
        add({ kind: "approval", entry: e, decision: decisions.get(e.requestId) ?? null });
        break;
      case "decision":
        // Shown with its approval; on its own only if the approval isn't in the transcript.
        if (!asked.has(e.requestId)) add({ kind: "decision", entry: e });
        break;
      case "result":
        add({ kind: "result", entry: e });
        break;
      case "error":
        add({ kind: "error", entry: e });
        break;
    }
  }
  for (const t of out)
    for (const item of t.items)
      if (item.kind === "tools") {
        item.summary = summarize(item.calls);
        item.failed = item.calls.some((c) => c.result?.isError);
      }
  return out;
}

type Verb = { key: string; one: string; many: (n: number) => string };

const VERBS: Record<string, Verb> = {
  Read: { key: "read", one: "read a file", many: (n) => `read ${n} files` },
  Edit: { key: "edit", one: "edited a file", many: (n) => `made ${n} edits` },
  MultiEdit: { key: "edit", one: "edited a file", many: (n) => `made ${n} edits` },
  Write: { key: "write", one: "wrote a file", many: (n) => `wrote ${n} files` },
  NotebookEdit: { key: "edit", one: "edited a file", many: (n) => `made ${n} edits` },
  Bash: { key: "run", one: "ran a command", many: (n) => `ran ${n} commands` },
  PowerShell: { key: "run", one: "ran a command", many: (n) => `ran ${n} commands` },
  Grep: { key: "search", one: "searched", many: (n) => `searched ${n} times` },
  Glob: { key: "search", one: "searched", many: (n) => `searched ${n} times` },
  WebSearch: { key: "web", one: "searched the web", many: (n) => `searched the web ${n} times` },
  WebFetch: { key: "fetch", one: "fetched a page", many: (n) => `fetched ${n} pages` },
  TodoWrite: { key: "todo", one: "updated the to-do list", many: () => "updated the to-do list" },
  Task: { key: "task", one: "ran a subagent", many: (n) => `ran ${n} subagents` },
};

function verbOf(name: string): Verb {
  const known = VERBS[name];
  if (known) return known;
  if (name.startsWith("desktop · "))
    return { key: "desktop", one: "used the desktop", many: (n) => `used the desktop ${n} times` };
  return { key: `tool:${name}`, one: `used ${name}`, many: (n) => `used ${name} ${n} times` };
}

/** "Read 3 files, ran 2 commands": verbs in the order they first came, counted. */
export function summarize(calls: Call[]): string {
  const counts = new Map<string, { verb: Verb; n: number }>();
  for (const c of calls) {
    const verb = c.tool ? verbOf(c.tool.name) : { key: "output", one: "an output", many: (n: number) => `${n} outputs` };
    const slot = counts.get(verb.key);
    if (slot) slot.n++;
    else counts.set(verb.key, { verb, n: 1 });
  }
  const text = [...counts.values()].map(({ verb, n }) => (n === 1 ? verb.one : verb.many(n))).join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}
