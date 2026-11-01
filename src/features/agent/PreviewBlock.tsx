import type { ApprovalPreview, DiffLine } from "../../lib/types";

const SIGN: Record<DiffLine["op"], string> = { add: "+", del: "−", ctx: " ", gap: "⋯" };

/**
 * What an approval would change or run (2.8b), as Rust built it: a diff (header `file · +n −m`, then lines with a sign
 * column) or a shell command already wrapped to 48 columns (shown as-is, never re-wrapped). Spans only, so it can sit
 * inside the island's clickable well; the island and the command window size it (activities.css, command.css).
 */
export function PreviewBlock({ preview }: { preview: ApprovalPreview }) {
  const foot = [
    preview.moreLines > 0 ? `+${preview.moreLines} more ${preview.moreLines === 1 ? "line" : "lines"}` : null,
    preview.kind === "command" && preview.cut ? "cut" : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <span className="preview" data-kind={preview.kind}>
      {preview.kind === "diff" && (
        <span className="preview__head">
          <span className="preview__file">{preview.file}</span>
          <span className="preview__stat">
            {/* A cut input's counts would be wrong: say so instead. */}
            {preview.cut || preview.added === null || preview.removed === null ? (
              "cut"
            ) : (
              <>
                <span data-op="add">+{preview.added}</span> <span data-op="del">−{preview.removed}</span>
              </>
            )}
          </span>
          {preview.note && <span className="preview__note">{preview.note}</span>}
        </span>
      )}
      <span className="preview__lines">
        {preview.kind === "diff"
          ? preview.lines.map((line, i) => (
              <span key={i} className="preview__line" data-op={line.op}>
                <span className="preview__sign" aria-hidden={line.op === "ctx"}>
                  {SIGN[line.op]}
                </span>
                <span className="preview__text">{line.text}</span>
              </span>
            ))
          : preview.lines.map((line, i) => (
              <span key={i} className="preview__line">
                <span className="preview__text">{line}</span>
              </span>
            ))}
      </span>
      {foot && <span className="preview__more">{foot}</span>}
    </span>
  );
}
