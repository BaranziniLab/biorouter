import { deflateRawSync } from 'node:zlib';
import { describe, expect, it } from 'vitest';
import { sheetNamesFromWorkbookXml, xlsxSheetNames } from './xlsxSheetNames';

const WORKBOOK = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="r">
  <bookViews><workbookView/></bookViews>
  <sheets>
    <sheet name="Counts" sheetId="1" r:id="rId1"/>
    <sheet name="Q&amp;A &#x2014; notes" sheetId="2" r:id="rId2"/>
    <sheet sheetId="3" name='Raw' r:id="rId3"/>
  </sheets>
  <definedNames><definedName name="NotASheet">Counts!$A$1</definedName></definedNames>
</workbook>`;

/** A minimal zip with one entry per file, stored (0) or deflated (8). CRCs are not checked. */
function zip(files: Array<{ name: string; data: Uint8Array; method: 0 | 8 }>): ArrayBuffer {
  const encoder = new TextEncoder();
  const chunks: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  for (const file of files) {
    const name = encoder.encode(file.name);
    const body = file.method === 8 ? new Uint8Array(deflateRawSync(file.data)) : file.data;
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(8, file.method, true);
    local.setUint32(18, body.length, true);
    local.setUint32(22, file.data.length, true);
    local.setUint16(26, name.length, true);
    chunks.push(new Uint8Array(local.buffer), name, body);
    const header = new DataView(new ArrayBuffer(46));
    header.setUint32(0, 0x02014b50, true);
    header.setUint16(10, file.method, true);
    header.setUint32(20, body.length, true);
    header.setUint32(24, file.data.length, true);
    header.setUint16(28, name.length, true);
    header.setUint32(42, offset, true);
    central.push(new Uint8Array(header.buffer), name);
    offset += 30 + name.length + body.length;
  }
  const centralSize = central.reduce((sum, part) => sum + part.length, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, files.length, true);
  end.setUint16(10, files.length, true);
  end.setUint32(12, centralSize, true);
  end.setUint32(16, offset, true);
  const parts = [...chunks, ...central, new Uint8Array(end.buffer)];
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out.buffer;
}

const encode = (text: string) => new TextEncoder().encode(text);

describe('sheetNamesFromWorkbookXml', () => {
  it('lists every sheet name in tab order, decoded, and nothing else', () => {
    expect(sheetNamesFromWorkbookXml(WORKBOOK)).toEqual(['Counts', 'Q&A — notes', 'Raw']);
  });

  it('answers nothing for a document without sheets', () => {
    expect(sheetNamesFromWorkbookXml('<workbook/>')).toEqual([]);
  });
});

describe('xlsxSheetNames', () => {
  it('reads a stored workbook index from the zip', async () => {
    const data = zip([
      { name: '[Content_Types].xml', data: encode('<Types/>'), method: 0 },
      { name: 'xl/workbook.xml', data: encode(WORKBOOK), method: 0 },
    ]);
    expect(await xlsxSheetNames(data)).toEqual(['Counts', 'Q&A — notes', 'Raw']);
  });

  it.runIf(typeof DecompressionStream !== 'undefined')(
    'inflates a deflated workbook index',
    async () => {
      const data = zip([{ name: 'xl/workbook.xml', data: encode(WORKBOOK), method: 8 }]);
      expect(await xlsxSheetNames(data)).toEqual(['Counts', 'Q&A — notes', 'Raw']);
    }
  );

  it('answers null for bytes that are not a zip, or a zip with no index', async () => {
    expect(await xlsxSheetNames(encode('not a zip').buffer as ArrayBuffer)).toBeNull();
    expect(
      await xlsxSheetNames(zip([{ name: 'xl/other.xml', data: encode('<x/>'), method: 0 }]))
    ).toBeNull();
  });
});
