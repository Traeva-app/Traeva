// The TS mirror of `reader::structure_line` (contract §4.4, v23): which
// lines of a PPTX or XLSX reader's output the reader wrote itself, like
// "⟦slide 3 · id 257⟧". Only for those four readers; in TXT/MD and DOCX
// output a "⟦…⟧" line is the user's own text. The Rust function is the
// rule; this copy only helps the window show the lines.

export const STRUCTURE_READERS = ["pptx-1", "pptx-preview-1", "xlsx-1", "xlsx-preview-1"] as const;

export type CellField = "literal" | "formula" | "cache";
export type CellValueType = "number" | "text" | "boolean" | "error" | "date" | "blank";

export type StructureLine =
  | { kind: "slide"; number: number; id: number; hidden: boolean }
  | { kind: "role"; role: "title" | "text" | "notes" }
  | { kind: "sheet"; number: number; id: number; name: string; visibility: "visible" | "hidden" | "very_hidden" }
  | { kind: "cell"; address: string; field: CellField; valueType: CellValueType | null }
  // A reader line that fits no form: still never the document's text.
  | { kind: "unrecognized" };

export function hasStructureLines(reader: string): boolean {
  return (STRUCTURE_READERS as readonly string[]).includes(reader);
}

const PREVIEW_READERS = ["pptx-preview-1", "xlsx-preview-1"];
const VALUE_TYPES = ["number", "text", "boolean", "error", "date"];

/** The structure line `line` is in `reader`'s output, or null for the
 * document's own lines and for every other reader. */
export function structureLine(reader: string, line: string): StructureLine | null {
  if (!hasStructureLines(reader) || !line.startsWith("⟦")) return null;
  const preview = PREVIEW_READERS.includes(reader);
  const parsed = line.endsWith("⟧") && line.length >= 2
    ? (reader.startsWith("pptx") ? slideLine : sheetLine)(line.slice(1, -1).split(" · "), preview)
    : null;
  return parsed ?? { kind: "unrecognized" };
}

function slideLine(fields: string[], preview: boolean): StructureLine | null {
  if (fields.length === 1 && ["title", "text", "notes"].includes(fields[0])) {
    return { kind: "role", role: fields[0] as "title" | "text" | "notes" };
  }
  if (fields.length < 2 || fields.length > 3) return null;
  let hidden = false;
  if (fields.length === 3) {
    if (!(fields[2] === "hidden" && preview)) return null;
    hidden = true;
  }
  const number = counted(fields[0], "slide ");
  const id = counted(fields[1], "id ");
  return number === null || id === null ? null : { kind: "slide", number, id, hidden };
}

function sheetLine(fields: string[], preview: boolean): StructureLine | null {
  if (fields.length >= 2 && fields[0].startsWith("cell ")) {
    const address = fields[0].slice("cell ".length);
    if (!/^[A-Z]{1,3}[1-9][0-9]*$/.test(address)) return null;
    const [field, ...rest] = fields.slice(1);
    if (field === "literal" && rest.length === 1 && VALUE_TYPES.includes(rest[0])) {
      return { kind: "cell", address, field, valueType: rest[0] as CellValueType };
    }
    if (field === "formula" && rest.length === 0) return { kind: "cell", address, field, valueType: null };
    if (field === "cache" && rest.length === 2 && rest[1] === "not verified") {
      if (rest[0] === "missing") return { kind: "cell", address, field, valueType: null };
      if (rest[0] === "blank" || VALUE_TYPES.includes(rest[0])) {
        return { kind: "cell", address, field, valueType: rest[0] as CellValueType };
      }
    }
    return null;
  }
  if (fields.length < 3 || fields.length > 4) return null;
  let visibility: "visible" | "hidden" | "very_hidden" = "visible";
  if (fields.length === 4) {
    if (!preview) return null;
    if (fields[3] === "hidden") visibility = "hidden";
    else if (fields[3] === "very hidden") visibility = "very_hidden";
    else return null;
  }
  const number = counted(fields[0], "sheet ");
  const id = counted(fields[1], "id ");
  if (number === null || id === null || !fields[2].startsWith("name ")) return null;
  let name: unknown;
  try {
    name = JSON.parse(fields[2].slice("name ".length));
  } catch {
    return null;
  }
  return typeof name === "string" ? { kind: "sheet", number, id, name, visibility } : null;
}

/** A positive number without sign or leading zeros, after `prefix`. */
function counted(field: string, prefix: string): number | null {
  if (!field.startsWith(prefix)) return null;
  const text = field.slice(prefix.length);
  if (!/^[1-9][0-9]*$/.test(text)) return null;
  const value = Number(text);
  return value <= 0xffffffff ? value : null;
}

/** A source line as the user wrote it: the reader's escape taken off.
 * Positions (QuoteAt) still count the reader's output. */
export function unescapeSourceLine(reader: string, line: string): string {
  if (hasStructureLines(reader) && line.startsWith("\\") && (line[1] === "⟦" || line[1] === "\\")) {
    return line.slice(1);
  }
  return line;
}
