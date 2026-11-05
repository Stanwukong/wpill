import { test } from "node:test";
import assert from "node:assert/strict";
import type { AttachmentPick } from "../lib/types.ts";
import { formatBytes, mergePicks } from "./attachmentList.ts";

const pick = (path: string): AttachmentPick => ({ path, name: path.split("\\").pop() ?? path, kind: "file", bytes: 1 });

test("sizes", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(14_336), "14 KB");
  assert.equal(formatBytes(1.2 * 1024 * 1024), "1.2 MB");
  assert.equal(formatBytes(25 * 1024 * 1024), "25 MB");
  assert.equal(formatBytes(3 * 1024 ** 3), "3.0 GB");
});

test("picks are added once per path", () => {
  const list = [pick("C:\\a\\x.png")];
  const merged = mergePicks(list, [pick("c:\\A\\X.PNG"), pick("C:\\a\\y.txt"), pick("C:\\a\\y.txt")]);
  assert.deepEqual(
    merged.map((p) => p.path),
    ["C:\\a\\x.png", "C:\\a\\y.txt"],
  );
  assert.equal(list.length, 1);
});
