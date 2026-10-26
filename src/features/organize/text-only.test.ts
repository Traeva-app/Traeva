// File text, names and the model's words are shown as plain text: React
// escapes them, and no page code turns a string into HTML or code
// (organize-spec.md §9, O25). A source check, run with `npm test`.

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

/** Every .ts and .tsx file under `dir`, tests left out. */
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sources(path);
    const page = /\.tsx?$/.test(entry.name) && !entry.name.endsWith(".test.ts");
    return page ? [path] : [];
  });
}

test("no page code renders a string as HTML or runs it", () => {
  const files = sources("src");
  assert.ok(files.some((file) => file.endsWith("OrganizePage.tsx")), "found the pages");
  const unsafe = [
    "dangerouslySetInnerHTML",
    "innerHTML",
    "outerHTML",
    "insertAdjacentHTML",
    "document.write",
    "eval(",
    "new Function(",
  ];
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    for (const word of unsafe) {
      assert.ok(!text.includes(word), `${file} uses ${word}`);
    }
  }
});
