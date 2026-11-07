/**
 * What a pasted or dropped list holds, read the way the daemon reads it, so
 * **Paste a list…** can name the format it sends and show the terms it found
 * before the daemon's dry run answers ("Dialogs" in
 * `docs/dictation/dictionary.md`).
 *
 * The daemon decides what is new, merged, duplicate or invalid
 * (`POST /dictionary/import` with `dry_run: true`); nothing here validates a
 * term. The grammar mirrors `crates/biorouter-dictation/src/store/transfer.rs`:
 *
 * - lines: one term per line, `EPAS1 = epas one, e pass one` for spoken forms,
 *   and `#` starts a comment at the start of a line or after a space or a tab,
 *   so `C#` keeps its `#`;
 * - CSV and TSV: `written,spoken_forms,note`, a first row naming the columns is
 *   read as a header, spoken forms split on `;`, quoted fields honoured;
 * - JSON: the export (`"format": "biorouter.dictionary"`, version 1).
 */
import type { DictionaryImportFormat } from '../../api';

/** `POST /dictionary/import` reads at most this much content (`MAX_IMPORT_BYTES`). */
export const MAX_IMPORT_BYTES = 1024 * 1024;
/** …and at most this many rows (`MAX_IMPORT_ROWS`). */
export const MAX_IMPORT_ROWS = 5_000;

export const EXPORT_FORMAT = 'biorouter.dictionary';
export const EXPORT_VERSION = 1;

export interface ParsedTerm {
  /** 1 based, as the daemon numbers a skipped line. */
  line: number;
  written: string;
  spokenForms: string[];
  note: string | null;
  /** JSON rows only: the row says `"tier": "private"`. */
  private?: boolean;
}

export type ParseResult =
  | { ok: true; format: DictionaryImportFormat; terms: ParsedTerm[] }
  | { ok: false; format: DictionaryImportFormat; problem: 'not_an_export' | 'unparsable' };

/** The format a file's extension names, or `null` for an extension that names none. */
export function formatForFileName(name: string): DictionaryImportFormat | null {
  const extension = name.toLowerCase().split('.').pop() ?? '';
  switch (extension) {
    case 'txt':
      return 'lines';
    case 'csv':
      return 'csv';
    case 'tsv':
      return 'tsv';
    case 'json':
      return 'json';
    default:
      return null;
  }
}

/**
 * The format to send: a file's extension when it names one; otherwise JSON for
 * text that is a JSON object, TSV for text where every non empty, non comment
 * line holds a tab (a range copied from a spreadsheet), and lines for anything
 * else. Pasted CSV is read as lines, because a comma there is how a line lists
 * its spoken forms; a CSV file is named by its extension.
 */
export function importFormatFor(
  fileName: string | null | undefined,
  content: string
): DictionaryImportFormat {
  const named = fileName ? formatForFileName(fileName) : null;
  if (named) return named;
  const trimmed = content.trimStart();
  if (trimmed.startsWith('{')) return 'json';
  const lines = content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith('#'));
  if (lines.length > 0 && lines.every((line) => line.includes('\t'))) return 'tsv';
  return 'lines';
}

/** UTF-8 bytes of `content`, the unit the daemon's limit counts. */
export function utf8Bytes(content: string): number {
  return new TextEncoder().encode(content).length;
}

/**
 * Whether `content` is over {@link MAX_IMPORT_BYTES} in UTF-8, encoding it only
 * when its length leaves the answer open. One UTF-16 unit is at least one UTF-8
 * byte and at most three (a surrogate pair is two units and four bytes), so a
 * string longer than the limit is over it and one at most a third of it is
 * under it. A pasted list far over the limit therefore costs nothing to refuse.
 */
export function overImportBytes(content: string): boolean {
  if (content.length > MAX_IMPORT_BYTES) return true;
  if (content.length * 3 <= MAX_IMPORT_BYTES) return false;
  return utf8Bytes(content) > MAX_IMPORT_BYTES;
}

/** Whether a file is over the limit by its size, which is in bytes, before it is read. */
export function fileOverImportBytes(file: { size: number }): boolean {
  return file.size > MAX_IMPORT_BYTES;
}

/** Whether the daemon would refuse the content as too large (413 `too_large`). */
export function exceedsImportLimits(content: string, terms: number): boolean {
  return overImportBytes(content) || terms > MAX_IMPORT_ROWS;
}

function splitForms(text: string, separator: string): string[] {
  return text
    .split(separator)
    .map((form) => form.trim())
    .filter((form) => form.length > 0);
}

/** A line's text before its comment (`transfer.rs` `before_comment`). */
export function beforeComment(line: string): string {
  let previous: string | null = null;
  for (let at = 0; at < line.length; at++) {
    const c = line[at];
    if (c === '#' && (previous === null || /\s/.test(previous))) return line.slice(0, at);
    previous = c;
  }
  return line;
}

function parseLines(content: string): ParsedTerm[] {
  const terms: ParsedTerm[] = [];
  content.split(/\r\n|\n|\r/).forEach((raw, index) => {
    const line = beforeComment(raw).trim();
    if (!line) return;
    const equals = line.indexOf('=');
    if (equals === -1) {
      terms.push({ line: index + 1, written: line, spokenForms: [], note: null });
      return;
    }
    terms.push({
      line: index + 1,
      written: line.slice(0, equals).trim(),
      spokenForms: splitForms(line.slice(equals + 1), ','),
      note: null,
    });
  });
  return terms;
}

/** One record of delimited text, with the line it starts on. */
interface DelimitedRecord {
  line: number;
  cells: string[];
}

/**
 * Split delimited text into records: `"` quotes a field, `""` inside one is a
 * quote, and a quoted field may hold the delimiter and line breaks. Records may
 * have any number of fields (the reader is flexible). Answers `null` for a quote
 * that never closes, the one shape the daemon's reader refuses outright.
 */
export function splitDelimited(content: string, delimiter: string): DelimitedRecord[] | null {
  const records: DelimitedRecord[] = [];
  let cells: string[] = [];
  let cell = '';
  let quoted = false;
  let line = 1;
  let recordLine = 1;
  let touched = false;
  for (let i = 0; i < content.length; i++) {
    const c = content[i];
    if (quoted) {
      if (c === '"') {
        if (content[i + 1] === '"') {
          cell += '"';
          i++;
        } else {
          quoted = false;
        }
      } else {
        if (c === '\n') line++;
        cell += c;
      }
      continue;
    }
    if (c === '"' && cell.trim() === '') {
      quoted = true;
      cell = '';
      touched = true;
    } else if (c === delimiter) {
      cells.push(cell);
      cell = '';
      touched = true;
    } else if (c === '\r' || c === '\n') {
      if (c === '\r' && content[i + 1] === '\n') i++;
      if (touched || cell !== '') {
        cells.push(cell);
        records.push({ line: recordLine, cells });
      }
      cells = [];
      cell = '';
      touched = false;
      line++;
      recordLine = line;
    } else {
      cell += c;
      touched = true;
    }
  }
  if (quoted) return null;
  if (touched || cell !== '') {
    cells.push(cell);
    records.push({ line: recordLine, cells });
  }
  return records;
}

function parseDelimited(content: string, delimiter: string): ParsedTerm[] | null {
  const records = splitDelimited(content, delimiter);
  if (!records) return null;
  let columns: { written: number; forms: number | null; note: number | null } = {
    written: 0,
    forms: 1,
    note: 2,
  };
  const terms: ParsedTerm[] = [];
  records.forEach((record, index) => {
    const cells = record.cells.map((value) => value.trim());
    if (index === 0 && cells.some((value) => value.toLowerCase() === 'written')) {
      const find = (name: string) => {
        const at = cells.findIndex((value) => value.toLowerCase() === name);
        return at === -1 ? null : at;
      };
      columns = { written: find('written') ?? 0, forms: find('spoken_forms'), note: find('note') };
      return;
    }
    const cell = (at: number | null) => (at === null ? '' : (cells[at] ?? ''));
    const written = cell(columns.written);
    if (!written && cells.every((value) => value === '')) return;
    const note = cell(columns.note);
    terms.push({
      line: record.line,
      written,
      spokenForms: splitForms(cell(columns.forms), ';'),
      note: note || null,
    });
  });
  return terms;
}

function parseJson(content: string): ParsedTerm[] | null {
  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object') return null;
  const document = value as { format?: unknown; version?: unknown; entries?: unknown };
  if (document.format !== EXPORT_FORMAT || document.version !== EXPORT_VERSION) return null;
  if (!Array.isArray(document.entries)) return null;
  return document.entries.map((entry, index): ParsedTerm => {
    const row = (entry && typeof entry === 'object' ? entry : {}) as Record<string, unknown>;
    const forms = Array.isArray(row.spoken_forms)
      ? row.spoken_forms.filter((form): form is string => typeof form === 'string')
      : [];
    return {
      line: index + 1,
      written: typeof row.written === 'string' ? row.written : '',
      spokenForms: forms,
      note: typeof row.note === 'string' ? row.note : null,
      private: row.tier === 'private',
    };
  });
}

/** The terms `content` holds in `format`, or why it cannot be read as a whole. */
export function parseTermList(content: string, format: DictionaryImportFormat): ParseResult {
  switch (format) {
    case 'lines':
      return { ok: true, format, terms: parseLines(content) };
    case 'csv':
    case 'tsv': {
      const terms = parseDelimited(content, format === 'csv' ? ',' : '\t');
      return terms ? { ok: true, format, terms } : { ok: false, format, problem: 'unparsable' };
    }
    case 'json': {
      const terms = parseJson(content);
      return terms ? { ok: true, format, terms } : { ok: false, format, problem: 'not_an_export' };
    }
  }
}
