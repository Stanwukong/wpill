/**
 * The Markdown parser behind `Markdown.tsx` (2.8c; tables, links, strikethrough and fence languages since 2.10a).
 * Pure: text in, a small tree out, rendered as React elements by `Markdown.tsx` (never HTML strings). Anything it
 * doesn't know reads as plain text. Tested with `pnpm test` (`markdownParse.test.ts`).
 */

export type Block =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "code"; text: string; lang: string }
  | { kind: "pre"; text: string }
  | { kind: "quote"; text: string }
  | { kind: "rule" }
  | { kind: "list"; list: List }
  | { kind: "table"; align: Align[]; head: string[]; rows: string[][] };

export type Align = "left" | "center" | "right" | null;

export interface List {
  ordered: boolean;
  start: number;
  indent: number;
  items: Item[];
}

export interface Item {
  text: string;
  children: List[];
}

export type Inline =
  | string
  | { kind: "code"; text: string }
  | { kind: "link"; href: string; children: Inline[] }
  | { kind: "strong"; children: Inline[] }
  | { kind: "em"; children: Inline[] }
  | { kind: "del"; children: Inline[] };

const FENCE = /^\s*(`{3,}|~{3,})\s*([^\s`]*)/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const TABLE = /^\s*\|/;
/** A table's delimiter row: `|---|:--:|--:|` (outer pipes optional). */
const DELIMITER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

export function parse(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      i++;
      continue;
    }
    const fence = FENCE.exec(line);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence[1])) body.push(lines[i++]);
      i++; // the closing fence (or the end)
      out.push({ kind: "code", text: body.join("\n"), lang: fence[2].toLowerCase() });
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      out.push({ kind: "heading", level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    if (RULE.test(line)) {
      out.push({ kind: "rule" });
      i++;
      continue;
    }
    if (isTableStart(lines, i)) {
      const head = cells(line);
      const align = cells(lines[i + 1]).map(alignOf);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim() !== "" && lines[i].includes("|")) {
        const row = cells(lines[i++]);
        rows.push(head.map((_, c) => row[c] ?? ""));
      }
      out.push({ kind: "table", align: head.map((_, c) => align[c] ?? null), head, rows });
      continue;
    }
    if (TABLE.test(line)) {
      // Pipes without a delimiter row: kept as their monospace lines.
      const body: string[] = [];
      while (i < lines.length && TABLE.test(lines[i])) body.push(lines[i++].trim());
      out.push({ kind: "pre", text: body.join("\n") });
      continue;
    }
    if (QUOTE.test(line)) {
      const body: string[] = [];
      for (let m; i < lines.length && (m = QUOTE.exec(lines[i])); i++) body.push(m[1]);
      out.push({ kind: "quote", text: body.join(" ") });
      continue;
    }
    if (ITEM.test(line)) {
      const [list, next] = parseList(lines, i);
      out.push({ kind: "list", list });
      i = next;
      continue;
    }
    const body: string[] = [];
    while (i < lines.length && lines[i].trim() !== "" && !startsBlock(lines, i)) body.push(lines[i++].trim());
    out.push({ kind: "paragraph", text: body.join(" ") });
  }
  return out;
}

function isTableStart(lines: string[], i: number): boolean {
  const next = lines[i + 1];
  return lines[i].includes("|") && next !== undefined && next.includes("-") && DELIMITER.test(next);
}

/** A row's cells: outer pipes dropped, split on pipes that aren't escaped or inside backticks. */
export function cells(row: string): string[] {
  let text = row.trim();
  if (text.startsWith("|")) text = text.slice(1);
  if (text.endsWith("|") && !text.endsWith("\\|")) text = text.slice(0, -1);
  const out: string[] = [];
  let cell = "";
  let ticks = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "\\" && text[i + 1] === "|") {
      cell += "|";
      i++;
    } else if (c === "`") {
      let run = 1;
      while (text[i + run] === "`") run++;
      ticks = ticks === 0 ? run : ticks === run ? 0 : ticks;
      cell += "`".repeat(run);
      i += run - 1;
    } else if (c === "|" && ticks === 0) {
      out.push(cell.trim());
      cell = "";
    } else cell += c;
  }
  out.push(cell.trim());
  return out;
}

function alignOf(delimiter: string): Align {
  const left = delimiter.startsWith(":");
  const right = delimiter.endsWith(":");
  return left && right ? "center" : right ? "right" : left ? "left" : null;
}

function startsBlock(lines: string[], i: number): boolean {
  const line = lines[i];
  return (
    FENCE.test(line) ||
    HEADING.test(line) ||
    RULE.test(line) ||
    ITEM.test(line) ||
    QUOTE.test(line) ||
    TABLE.test(line) ||
    isTableStart(lines, i)
  );
}

/** A list from `start`: deeper items nest under the one before them, shallower ones end it. */
function parseList(lines: string[], start: number): [List, number] {
  const first = ITEM.exec(lines[start])!;
  const root: List = {
    ordered: /\d/.test(first[2]),
    start: parseInt(first[2], 10) || 1,
    indent: width(first[1]),
    items: [],
  };
  const stack: List[] = [root];
  let i = start;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      // A blank line ends the list unless an item (or its continuation) follows.
      const next = lines[i + 1];
      if (next !== undefined && (ITEM.test(next) || (/^\s+\S/.test(next) && !FENCE.test(next)))) {
        i++;
        continue;
      }
      break;
    }
    const m = ITEM.exec(line);
    if (!m) {
      // A continuation of the last item (indented or lazy), unless another block starts.
      if (startsBlock(lines, i)) break;
      const items = stack[stack.length - 1].items;
      if (items.length > 0) items[items.length - 1].text += ` ${line.trim()}`;
      i++;
      continue;
    }
    const indent = width(m[1]);
    while (stack.length > 1 && indent < stack[stack.length - 1].indent) stack.pop();
    if (indent < root.indent) break;
    let list = stack[stack.length - 1];
    const parent = list.items[list.items.length - 1];
    if (indent > list.indent && parent) {
      const child: List = { ordered: /\d/.test(m[2]), start: parseInt(m[2], 10) || 1, indent, items: [] };
      parent.children.push(child);
      stack.push(child);
      list = child;
    }
    list.items.push({ text: m[3], children: [] });
    i++;
  }
  return [root, i];
}

function width(space: string): number {
  return space.replace(/\t/g, "    ").length;
}

// Code first (nothing inside it is Markdown), then links (`[text](url "title")`, `<https://…>`), bold, strikethrough,
// italic. Underscores only count at word edges.
const INLINE =
  /(`+)([\s\S]*?[^`])\1(?!`)|\[([^\]]*)\]\(\s*<?([^)\s>]*)>?(?:\s+"[^"]*")?\s*\)|<(https:\/\/[^\s<>]+)>|\*\*(?=\S)([\s\S]*?\S)\*\*|__(?=\S)([\s\S]*?\S)__(?!\w)|~~(?=\S)([\s\S]*?\S)~~|\*(?=[^\s*])([\s\S]*?[^\s*])\*|(?<!\w)_(?=[^\s_])([\s\S]*?[^\s_])_(?!\w)/g;

/** A bare https URL in text, without the punctuation that usually follows one. */
const BARE_URL = /https:\/\/[^\s<>()[\]]*[^\s<>()[\].,;:!?'"*_~]/g;

export function inline(text: string): Inline[] {
  const out: Inline[] = [];
  let last = 0;
  for (const m of text.matchAll(INLINE)) {
    const at = m.index ?? 0;
    if (at > last) out.push(...autolinks(text.slice(last, at)));
    if (m[2] !== undefined)
      out.push({ kind: "code", text: m[2].trim() === "" ? m[2] : m[2].replace(/^ (.*) $/, "$1") });
    else if (m[3] !== undefined) out.push({ kind: "link", href: m[4], children: inline(m[3]) });
    else if (m[5] !== undefined) out.push({ kind: "link", href: m[5], children: [m[5]] });
    else if (m[6] !== undefined || m[7] !== undefined) out.push({ kind: "strong", children: inline(m[6] ?? m[7]) });
    else if (m[8] !== undefined) out.push({ kind: "del", children: inline(m[8]) });
    else out.push({ kind: "em", children: inline(m[9] ?? m[10]) });
    last = at + m[0].length;
  }
  if (last < text.length) out.push(...autolinks(text.slice(last)));
  return out;
}

function autolinks(text: string): Inline[] {
  const out: Inline[] = [];
  let last = 0;
  for (const m of text.matchAll(BARE_URL)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    out.push({ kind: "link", href: m[0], children: [m[0]] });
    last = at + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Whether a link may be opened (Rust checks again): https with a host. Anything else shows as its text. */
export function openable(href: string): boolean {
  return /^https:\/\/[^\s/?#@]+/i.test(href);
}
