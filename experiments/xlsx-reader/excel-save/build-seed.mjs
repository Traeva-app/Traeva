import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { SpreadsheetFile, Workbook } from "@oai/artifact-tool";

const dir = fileURLToPath(new URL(".", import.meta.url));
const protocolBytes = await fs.readFile(`${dir}/protocol.json`);
const protocol = JSON.parse(protocolBytes);
const wb = Workbook.create();
for (const name of protocol.sheet_order) wb.worksheets.add(name);
for (const name of protocol.sheet_order) {
  const sheet = wb.worksheets.getItem(name);
  for (const [address, literal] of Object.entries(protocol.fixed_literals[name])) {
    sheet.getRange(address).values = [[literal.type === "number" ? Number(literal.value) : literal.value]];
  }
  for (const range of protocol.merged_ranges[name]) sheet.mergeCells(range);
  sheet.showGridLines = false;
  sheet.getRange(name === "F Data" ? "A1:C6" : "A1:B2").format.font = { name: "Arial", size: 11 };
  sheet.getRange(name === "F Data" ? "A1:C6" : "A1:B2").format.columnWidth = 22;
  sheet.getRange(name === "F Data" ? "A1:C6" : "A1:B2").format.rowHeight = 24;
}
const data = wb.worksheets.getItem("F Data");
data.getRange("B2").values = [[Number(protocol.stages.baseline.input)]];
data.getRange("C2").formulas = [[`=${protocol.stages.baseline.formula}`]];
wb.recalculate();
await fs.mkdir(`${dir}/local`, { recursive: true });
const inspections = [];
for (const name of protocol.sheet_order) {
  const range = name === "F Data" ? "A1:C6" : "A1:B2";
  inspections.push((await wb.inspect({ kind: "table", range: `'${name}'!${range}`, include: "values,formulas", tableMaxRows: 6, tableMaxCols: 3, maxChars: 2500 })).ndjson);
  const blob = await wb.render({ sheetName: name, range, scale: 1, format: "png" });
  await fs.writeFile(`${dir}/local/${name === "F Data" ? "data" : "chinese"}.png`, new Uint8Array(await blob.arrayBuffer()));
}
inspections.push((await wb.inspect({ kind: "match", searchTerm: "#REF!|#DIV/0!|#VALUE!|#NAME\\?|#NUM!|#NULL!", options: { useRegex: true, maxResults: 10 }, maxChars: 1500 })).ndjson);
await fs.writeFile(`${dir}/local/artifact-inspect.ndjson`, inspections.join("\n"));
await (await SpreadsheetFile.exportXlsx(wb)).save(`${dir}/artifact-candidate.xlsx`);
const bytes = await fs.readFile(`${dir}/artifact-candidate.xlsx`);
await fs.writeFile(`${dir}/local/authoring.json`, JSON.stringify({
  writer: "@oai/artifact-tool", protocol_sha256: createHash("sha256").update(protocolBytes).digest("hex"),
  seed_sha256: createHash("sha256").update(bytes).digest("hex"), seed_bytes: bytes.length,
  seed_cache_calculation: "Artifact Tool, not Microsoft Excel or Reader",
  excel_save_close_reopen: "NOT_EXECUTED"
}, null, 2) + "\n");
console.log(JSON.stringify({ seed_bytes: bytes.length, excel_execution: "NOT_EXECUTED" }));
