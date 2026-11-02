import type { ReactNode } from "react";

/**
 * A small Markdown renderer for Claude's plans (2.8c). It builds React elements only (never HTML strings): headings,
 * paragraphs, lists nested by indent, fenced code, block quotes, rules, and inline code, bold and italic. Links show
 * their text; tables stay as their monospace lines. Anything it doesn't know reads as plain text.
 */
export function Markdown({ source }: { source: string }) {
  return <div className="md">{blocks(parse(source.replace(/\r\n?/g, "\n").split("\n")))}</div>;
}

type Block =
  | { kind: "heading"; level: number; text: string }
  | { kind: "paragraph"; text: string }
  | { kind: "code"; text: string }
  | { kind: "pre"; text: string }
  | { kind: "quote"; text: string }
  | { kind: "rule" }
  | { kind: "list"; list: List };

interface List {
  ordered: boolean;
  start: number;
  indent: number;
  items: Item[];
}

interface Item {
  text: string;
  children: List[];
}

const FENCE = /^\s*(`{3,}|~{3,})/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const TABLE = /^\s*\|/;

function parse(lines: string[]): Block[] {
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
      out.push({ kind: "code", text: body.join("\n") });
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
    if (TABLE.test(line)) {
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
    while (i < lines.length && lines[i].trim() !== "" && !startsBlock(lines[i])) body.push(lines[i++].trim());
    out.push({ kind: "paragraph", text: body.join(" ") });
  }
  return out;
}

function startsBlock(line: string): boolean {
  return (
    FENCE.test(line) || HEADING.test(line) || RULE.test(line) || ITEM.test(line) || QUOTE.test(line) || TABLE.test(line)
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
      if (startsBlock(line)) break;
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

function blocks(list: Block[]): ReactNode[] {
  return list.map((b, i) => {
    switch (b.kind) {
      case "heading":
        return (
          <p key={i} className="md__h" data-level={Math.min(b.level, 3)} role="heading" aria-level={b.level}>
            {inline(b.text)}
          </p>
        );
      case "paragraph":
        return <p key={i}>{inline(b.text)}</p>;
      case "code":
        return (
          <pre key={i} className="md__code">
            {b.text}
          </pre>
        );
      case "pre":
        return (
          <pre key={i} className="md__pre">
            {b.text}
          </pre>
        );
      case "quote":
        return (
          <blockquote key={i} className="md__quote">
            {inline(b.text)}
          </blockquote>
        );
      case "rule":
        return <hr key={i} className="md__rule" />;
      case "list":
        return <ListView key={i} list={b.list} />;
    }
  });
}

function ListView({ list }: { list: List }) {
  const items = list.items.map((item, i) => (
    <li key={i}>
      {inline(item.text)}
      {item.children.map((child, j) => (
        <ListView key={j} list={child} />
      ))}
    </li>
  ));
  return list.ordered ? (
    <ol className="md__list" start={list.start}>
      {items}
    </ol>
  ) : (
    <ul className="md__list">{items}</ul>
  );
}

// Code first (nothing inside it is Markdown), then links, bold, italic. Underscores only count at word edges.
const INLINE =
  /(`+)([\s\S]*?[^`])\1(?!`)|\[([^\]]*)\]\([^)]*\)|\*\*(?=\S)([\s\S]*?\S)\*\*|__(?=\S)([\s\S]*?\S)__(?!\w)|\*(?=[^\s*])([\s\S]*?[^\s*])\*|(?<!\w)_(?=[^\s_])([\s\S]*?[^\s_])_(?!\w)/g;

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let key = 0;
  for (const m of text.matchAll(INLINE)) {
    const at = m.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    if (m[2] !== undefined)
      out.push(<code key={key++}>{m[2].trim() === "" ? m[2] : m[2].replace(/^ (.*) $/, "$1")}</code>);
    else if (m[3] !== undefined) out.push(<span key={key++}>{inline(m[3])}</span>);
    else if (m[4] !== undefined || m[5] !== undefined) out.push(<strong key={key++}>{inline(m[4] ?? m[5])}</strong>);
    else out.push(<em key={key++}>{inline(m[6] ?? m[7])}</em>);
    last = at + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
