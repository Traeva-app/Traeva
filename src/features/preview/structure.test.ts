import test from "node:test";
import assert from "node:assert/strict";
import { structureLine, unescapeSourceLine } from "./structure.ts";

// The same cases as reader_tests.rs, so the copy and the rule agree.

test("slide lines only for the PPTX readers", () => {
  for (const reader of ["pptx-1", "pptx-preview-1"]) {
    assert.deepEqual(structureLine(reader, "⟦slide 3 · id 257⟧"), { kind: "slide", number: 3, id: 257, hidden: false });
    assert.deepEqual(structureLine(reader, "⟦notes⟧"), { kind: "role", role: "notes" });
    assert.equal(structureLine(reader, "Q3 results"), null);
    assert.equal(structureLine(reader, "\\⟦notes⟧"), null);
  }
  assert.deepEqual(structureLine("pptx-preview-1", "⟦slide 4 · id 260 · hidden⟧"), { kind: "slide", number: 4, id: 260, hidden: true });
  assert.deepEqual(structureLine("pptx-1", "⟦slide 4 · id 260 · hidden⟧"), { kind: "unrecognized" });
  for (const reader of ["txt-md-1", "docx-2", "none"]) assert.equal(structureLine(reader, "⟦notes⟧"), null);
});

test("sheet and cell lines only for the XLSX readers", () => {
  const reader = "xlsx-1";
  assert.deepEqual(structureLine(reader, '⟦sheet 1 · id 1 · name "F Data"⟧'), { kind: "sheet", number: 1, id: 1, name: "F Data", visibility: "visible" });
  assert.deepEqual(structureLine(reader, "⟦cell C2 · literal · number⟧"), { kind: "cell", address: "C2", field: "literal", valueType: "number" });
  assert.deepEqual(structureLine(reader, "⟦cell C2 · formula⟧"), { kind: "cell", address: "C2", field: "formula", valueType: null });
  assert.deepEqual(structureLine(reader, "⟦cell C2 · cache · number · not verified⟧"), { kind: "cell", address: "C2", field: "cache", valueType: "number" });
  assert.deepEqual(structureLine(reader, "⟦cell C2 · cache · blank · not verified⟧"), { kind: "cell", address: "C2", field: "cache", valueType: "blank" });
  assert.deepEqual(structureLine(reader, "⟦cell C2 · cache · missing · not verified⟧"), { kind: "cell", address: "C2", field: "cache", valueType: null });
  assert.deepEqual(structureLine("xlsx-preview-1", '⟦sheet 3 · id 4 · name "Old" · very hidden⟧'), { kind: "sheet", number: 3, id: 4, name: "Old", visibility: "very_hidden" });
  for (const odd of [
    '⟦sheet 3 · id 4 · name "Old" · hidden⟧', "⟦cell C2 · cache · number⟧", "⟦cell C2 · formula · number⟧",
    "⟦cell C2 · literal · blank⟧", "⟦cell c2 · formula⟧", "⟦cell C02 · formula⟧", "⟦cell ABCD1 · formula⟧",
    '⟦sheet 0 · id 1 · name "x"⟧', "⟦sheet 1 · id 1 · name x⟧", "⟦slide 1 · id 256⟧", "⟦cell C2 · formula", "⟦⟧",
    "⟦cell C2 · formula⟧ ",
  ]) assert.deepEqual(structureLine(reader, odd), { kind: "unrecognized" }, odd);
  assert.equal(structureLine(reader, " ⟦cell C2 · formula⟧"), null);
});

test("a sheet name written with \\u escapes reads back", () => {
  const line = '⟦sheet 2 · id 7 · name "Q3 \\u00b7 \\u0022final\\u0022 \\u27e6draft\\u27e7"⟧';
  assert.deepEqual(structureLine("xlsx-1", line), { kind: "sheet", number: 2, id: 7, name: 'Q3 · "final" ⟦draft⟧', visibility: "visible" });
});

test("the escape is taken off for showing, only for these readers", () => {
  assert.equal(unescapeSourceLine("pptx-1", "\\⟦slide 1 · id 256⟧"), "⟦slide 1 · id 256⟧");
  assert.equal(unescapeSourceLine("pptx-1", "\\\\already"), "\\already");
  assert.equal(unescapeSourceLine("pptx-1", "\\n"), "\\n");
  assert.equal(unescapeSourceLine("txt-md-1", "\\⟦x⟧"), "\\⟦x⟧");
});
