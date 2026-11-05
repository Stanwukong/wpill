import { useEffect, useState, type MouseEvent, type ReactNode } from "react";
import { commands } from "../lib/ipc";
import { inline, openable, parse, type Block, type Inline, type List } from "./markdownParse";

/**
 * Renders Markdown from `markdownParse.ts` as React elements only (never HTML strings): headings, paragraphs,
 * nested lists, fenced code with a Copy button, block quotes, rules, tables, and inline code, links, bold, italic and
 * strikethrough. Links open only when `openable` (https), through Rust; anything else shows as its text.
 */
export function Markdown({ source }: { source: string }) {
  return <div className="md">{blocks(parse(source))}</div>;
}

function blocks(list: Block[]): ReactNode[] {
  return list.map((b, i) => {
    switch (b.kind) {
      case "heading":
        return (
          <p key={i} className="md__h" data-level={Math.min(b.level, 3)} role="heading" aria-level={b.level}>
            {spans(inline(b.text))}
          </p>
        );
      case "paragraph":
        return <p key={i}>{spans(inline(b.text))}</p>;
      case "code":
        return <CodeBlock key={i} text={b.text} lang={b.lang} />;
      case "pre":
        return (
          <pre key={i} className="md__pre">
            {b.text}
          </pre>
        );
      case "quote":
        return (
          <blockquote key={i} className="md__quote">
            {spans(inline(b.text))}
          </blockquote>
        );
      case "rule":
        return <hr key={i} className="md__rule" />;
      case "list":
        return <ListView key={i} list={b.list} />;
      case "table":
        return (
          <div key={i} className="md__tablewrap">
            <table className="md__table">
              <thead>
                <tr>
                  {b.head.map((cell, c) => (
                    <th key={c} style={{ textAlign: b.align[c] ?? undefined }}>
                      {spans(inline(cell))}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {b.rows.map((row, r) => (
                  <tr key={r}>
                    {row.map((cell, c) => (
                      <td key={c} style={{ textAlign: b.align[c] ?? undefined }}>
                        {spans(inline(cell))}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        );
    }
  });
}

function ListView({ list }: { list: List }) {
  const items = list.items.map((item, i) => (
    <li key={i}>
      {spans(inline(item.text))}
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

/** A fenced block: its language (if any) and Copy above the code. "Copied" is local feedback for 1.5 s. */
function CodeBlock({ text, lang }: { text: string; lang: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(t);
  }, [copied]);
  const copy = () => {
    navigator.clipboard.writeText(text).then(
      () => setCopied(true),
      () => {},
    );
  };
  return (
    <div className="md__codeblock">
      <div className="md__codehead">
        <span className="md__lang">{lang}</span>
        <button type="button" className="md__copy" aria-label="Copy code" onClick={copy}>
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="md__code">{text}</pre>
    </div>
  );
}

function openLink(e: MouseEvent<HTMLAnchorElement>, href: string) {
  e.preventDefault();
  commands.agentOpenLink(href).catch(() => {});
}

function spans(tokens: Inline[]): ReactNode[] {
  return tokens.map((t, i) => {
    if (typeof t === "string") return t;
    switch (t.kind) {
      case "code":
        return <code key={i}>{t.text}</code>;
      case "strong":
        return <strong key={i}>{spans(t.children)}</strong>;
      case "em":
        return <em key={i}>{spans(t.children)}</em>;
      case "del":
        return <del key={i}>{spans(t.children)}</del>;
      case "link":
        return openable(t.href) ? (
          <a key={i} className="md__link" href={t.href} title={t.href} onClick={(e) => openLink(e, t.href)}>
            {spans(t.children)}
          </a>
        ) : (
          <span key={i}>{spans(t.children)}</span>
        );
    }
  });
}
