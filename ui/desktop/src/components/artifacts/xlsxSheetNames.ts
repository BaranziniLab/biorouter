/**
 * A workbook's sheet names, in order, read from its own index.
 *
 * The spreadsheet preview renders each sheet with `xlsx-preview`, which returns
 * the sheets' HTML and not their names, so the picker could only say "Sheet 1",
 * "Sheet 2" (spec 3.8: "the sheet picker with sheet names"). An `.xlsx` is a zip;
 * its `xl/workbook.xml` lists `<sheet name="…">` in tab order, the order the
 * renderer emits them in.
 *
 * Read-only and bounded: the zip's central directory is walked from its end
 * record, only `xl/workbook.xml` is inflated (with the platform's own
 * `DecompressionStream`), and an index larger than 2 MB is refused. Any surprise
 * resolves `null`, and the picker falls back to numbers.
 */

const EOCD_SIGNATURE = 0x06054b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const LOCAL_SIGNATURE = 0x04034b50;
const WORKBOOK_PATH = 'xl/workbook.xml';
const MAX_INDEX_BYTES = 2 * 1024 * 1024;
const MAX_COMMENT = 0xffff;

function findEndOfCentralDirectory(view: DataView): number {
  const last = view.byteLength - 22;
  const first = Math.max(0, last - MAX_COMMENT);
  for (let offset = last; offset >= first; offset -= 1) {
    if (view.getUint32(offset, true) === EOCD_SIGNATURE) return offset;
  }
  return -1;
}

async function inflateRaw(bytes: Uint8Array<ArrayBuffer>): Promise<Uint8Array | null> {
  if (typeof DecompressionStream === 'undefined') return null;
  // `Response` rather than `Blob.stream()`: both engines this runs in (Chromium,
  // and Node under the tests) give a Response a byte stream.
  const body = new Response(bytes).body;
  if (!body) return null;
  const stream = body.pipeThrough(new DecompressionStream('deflate-raw'));
  const buffer = await new Response(stream).arrayBuffer();
  return new Uint8Array(buffer);
}

const ENTITIES: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
};

function decodeXmlText(value: string): string {
  return value.replace(/&(#x[0-9a-f]+|#\d+|\w+);/gi, (match, entity: string) => {
    if (entity[0] === '#') {
      const code =
        entity[1] === 'x' || entity[1] === 'X'
          ? Number.parseInt(entity.slice(2), 16)
          : Number.parseInt(entity.slice(1), 10);
      return Number.isFinite(code) && code > 0 && code <= 0x10ffff
        ? String.fromCodePoint(code)
        : match;
    }
    return ENTITIES[entity] ?? match;
  });
}

/** The `name` of every `<sheet>` in a workbook index, in document order. */
export function sheetNamesFromWorkbookXml(xml: string): string[] {
  const sheets = xml.match(/<(?:\w+:)?sheets\b[\s\S]*?<\/(?:\w+:)?sheets>/)?.[0] ?? '';
  const names: string[] = [];
  for (const match of sheets.matchAll(/<(?:\w+:)?sheet\b[^>]*?\bname\s*=\s*(["'])(.*?)\1/g)) {
    names.push(decodeXmlText(match[2]));
  }
  return names;
}

export async function xlsxSheetNames(data: ArrayBuffer): Promise<string[] | null> {
  try {
    const view = new DataView(data);
    const end = findEndOfCentralDirectory(view);
    if (end < 0) return null;
    const entries = view.getUint16(end + 10, true);
    let cursor = view.getUint32(end + 16, true);
    const decoder = new TextDecoder();
    for (let index = 0; index < entries; index += 1) {
      if (cursor + 46 > view.byteLength || view.getUint32(cursor, true) !== CENTRAL_SIGNATURE) {
        return null;
      }
      const method = view.getUint16(cursor + 10, true);
      const compressedSize = view.getUint32(cursor + 20, true);
      const size = view.getUint32(cursor + 24, true);
      const nameLength = view.getUint16(cursor + 28, true);
      const extraLength = view.getUint16(cursor + 30, true);
      const commentLength = view.getUint16(cursor + 32, true);
      const localOffset = view.getUint32(cursor + 42, true);
      const name = decoder.decode(new Uint8Array(data, cursor + 46, nameLength));
      cursor += 46 + nameLength + extraLength + commentLength;
      if (name !== WORKBOOK_PATH) continue;

      if (size > MAX_INDEX_BYTES || compressedSize > MAX_INDEX_BYTES) return null;
      if (
        localOffset + 30 > view.byteLength ||
        view.getUint32(localOffset, true) !== LOCAL_SIGNATURE
      ) {
        return null;
      }
      const start =
        localOffset +
        30 +
        view.getUint16(localOffset + 26, true) +
        view.getUint16(localOffset + 28, true);
      if (start + compressedSize > view.byteLength) return null;
      const raw = new Uint8Array(data.slice(start, start + compressedSize));
      const bytes = method === 0 ? raw : method === 8 ? await inflateRaw(raw) : null;
      if (!bytes) return null;
      const names = sheetNamesFromWorkbookXml(decoder.decode(bytes));
      return names.length > 0 ? names : null;
    }
    return null;
  } catch {
    return null;
  }
}
