/**
 * Attachment chips' words and list rules (2.10b.4): sizes, and adding picks without repeats. Pure; tested with
 * `pnpm test` (`attachmentList.test.ts`).
 */
import type { AttachmentPick } from "../lib/types.ts";

/** "512 B", "14 KB", "1.2 MB" (1024-based, like Explorer). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${Math.max(0, Math.round(bytes))} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

/** `list` with `picks` added at the end; a path already there (Windows paths: any case) isn't added again. */
export function mergePicks(list: AttachmentPick[], picks: AttachmentPick[]): AttachmentPick[] {
  const seen = new Set(list.map((p) => p.path.toLowerCase()));
  const out = [...list];
  for (const p of picks) {
    const key = p.path.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(p);
  }
  return out;
}
