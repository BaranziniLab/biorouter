import type { Message } from '../../api';

/**
 * The Chat summary's Code figure: the lines this chat's file edits added and
 * removed, summed over every edit (an edit and its undo both count, as two
 * commits would).
 *
 * It is read off the edits themselves, never off text. The figure used to scan
 * every string in the transcript for things shaped like a diff (`+3 −1`,
 * `3 insertions(+)`, a `diff --git`, a fenced code block), so a chat that
 * wrote a file from a Code Execution script read `+0 −0`, and a chat that only
 * *read* code (a grep, a `git show`, an answer quoting a snippet) read `+0 −9`
 * (the final check, 2026-10-12).
 *
 * What counts is a successful `text_editor` edit (`write`/`create`,
 * `str_replace` with `old_str`/`new_str` or a unified `diff`, `insert`, and
 * `undo_edit`, which reverses the last counted edit of the same file):
 * - called directly, from the call's own arguments;
 * - inside a Code Execution script, from the `biorouter/code-delta` total the
 *   daemon measured from each sub-call's FULL arguments. The per-call records
 *   cannot be used for that: their `args` are cut at 2 KB (so a real file's
 *   write is no longer JSON) and records past the meta budget are dropped.
 *   Scripts recorded before the daemon wrote that total fall back to the
 *   records whose arguments still parse.
 *
 * A file a shell command writes (`>`, `sed -i`, a Python script) is not
 * counted: nothing in the transcript says how many lines it changed.
 *
 * The daemon's tally (`crates/biorouter/src/agents/code_delta.rs`) applies the
 * same rules; `codeDeltaCases.json` is the table both test suites run, so the
 * two cannot drift apart unnoticed.
 */
export interface CodeDelta {
  added: number;
  removed: number;
}

/** The script total the daemon attaches to a Code Execution result. */
export const CODE_DELTA_META_KEY = 'biorouter/code-delta';
const NESTED_CALLS_META_KEY = 'biorouter/tool-calls';

/** The same two names `countSessionToolCalls` reads nested calls under. */
const CODE_EXECUTION_TOOLS = new Set([
  'code_execution__execute_code',
  'multi_tool_use__execute_code',
]);
/** `text_editor` is the developer extension's; `str_replace_editor` takes the same arguments. */
const EDITOR_TOOLS = new Set(['text_editor', 'str_replace_editor']);

/**
 * Above this many line pairs (after the unchanged head and tail are set aside)
 * the changed region is counted whole rather than diffed. Keeps one enormous
 * replacement from stalling a render. Must equal `LCS_CELL_BUDGET` in the
 * daemon's tally.
 */
const LCS_CELL_BUDGET = 1_000_000;

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function baseToolName(name: string): string {
  const delimiter = name.lastIndexOf('__');
  return delimiter === -1 ? name : name.slice(delimiter + 2);
}

/** A text's lines: a final newline ends the last line rather than starting one. */
function splitLines(text: string): string[] {
  if (!text) return [];
  const lines = text.split('\n').map((line) => (line.endsWith('\r') ? line.slice(0, -1) : line));
  if (lines[lines.length - 1] === '') lines.pop();
  return lines;
}

/** Lines `after` has that `before` lacks, and the reverse, by longest common subsequence. */
export function lineDelta(before: string, after: string): CodeDelta {
  const a = splitLines(before);
  const b = splitLines(after);
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start += 1;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA -= 1;
    endB -= 1;
  }
  const n = endA - start;
  const m = endB - start;
  if (n === 0 || m === 0 || n * m > LCS_CELL_BUDGET) return { added: m, removed: n };

  let previous = new Uint32Array(m + 1);
  let current = new Uint32Array(m + 1);
  for (let i = 1; i <= n; i += 1) {
    const line = a[start + i - 1];
    for (let j = 1; j <= m; j += 1) {
      current[j] =
        line === b[start + j - 1] ? previous[j - 1] + 1 : Math.max(previous[j], current[j - 1]);
    }
    [previous, current] = [current, previous];
  }
  const common = previous[m];
  return { added: m - common, removed: n - common };
}

const HUNK_HEADER_RE = /^@@ -\d+(?:,(\d+))? \+\d+(?:,(\d+))? @@/;

/**
 * The `+` and `-` lines of a unified diff's hunks. A hunk header's counts say
 * where the hunk ends, so a removed `-- x` line (written `--- x`) is not taken
 * for the next file's header. A header without counts falls back to counting
 * every `+`/`-` line that is not a `+++ `/`--- ` header.
 */
export function patchDelta(diff: string): CodeDelta {
  let added = 0;
  let removed = 0;
  let state: 'header' | 'counted' | 'loose' = 'header';
  let oldLeft = 0;
  let newLeft = 0;
  for (const raw of diff.split('\n')) {
    const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
    if (state === 'counted') {
      const first = line.charAt(0);
      if (first === '\\') continue;
      if (first === '-' || first === '+' || first === ' ' || line === '') {
        if (first === '-') {
          removed += 1;
          oldLeft -= 1;
        } else if (first === '+') {
          added += 1;
          newLeft -= 1;
        } else {
          oldLeft -= 1;
          newLeft -= 1;
        }
        if (oldLeft <= 0 && newLeft <= 0) state = 'header';
        continue;
      }
      state = 'header';
    }
    if (line.startsWith('@@')) {
      const counts = HUNK_HEADER_RE.exec(line);
      if (counts) {
        oldLeft = counts[1] === undefined ? 1 : Number(counts[1]);
        newLeft = counts[2] === undefined ? 1 : Number(counts[2]);
        state = oldLeft > 0 || newLeft > 0 ? 'counted' : 'header';
      } else {
        state = 'loose';
      }
      continue;
    }
    if (state === 'loose') {
      if (line.startsWith('+++ ') || line.startsWith('--- ')) state = 'header';
      else if (line.startsWith('+')) added += 1;
      else if (line.startsWith('-')) removed += 1;
    }
  }
  return { added, removed };
}

function lineCount(text: string): number {
  return splitLines(text).length;
}

type EditorEdit = CodeDelta | 'undo' | null;

/** What one `text_editor` call changed, from its arguments alone. */
function editorEdit(args: Record<string, unknown>): EditorEdit {
  switch (args.command) {
    case 'write':
    case 'create':
      return typeof args.file_text === 'string'
        ? { added: lineCount(args.file_text), removed: 0 }
        : null;
    case 'str_replace':
      if (typeof args.diff === 'string') return patchDelta(args.diff);
      return typeof args.old_str === 'string' && typeof args.new_str === 'string'
        ? lineDelta(args.old_str, args.new_str)
        : null;
    case 'insert':
      return typeof args.new_str === 'string'
        ? { added: lineCount(args.new_str), removed: 0 }
        : null;
    case 'undo_edit':
      return 'undo';
    default:
      return null;
  }
}

/** A direct call's arguments are one object for the life of the transcript. */
const editCache = new WeakMap<object, EditorEdit>();

function cachedEditorEdit(args: Record<string, unknown>): EditorEdit {
  if (editCache.has(args)) return editCache.get(args) ?? null;
  const edit = editorEdit(args);
  editCache.set(args, edit);
  return edit;
}

class CodeDeltaTally {
  added = 0;
  removed = 0;
  /** Each file's counted edits, newest last, for `undo_edit` to take back. */
  private readonly history = new Map<string, CodeDelta[]>();

  add(delta: CodeDelta) {
    this.added += delta.added;
    this.removed += delta.removed;
  }

  /** One successful `text_editor` call. */
  editorCall(args: unknown) {
    const record = asRecord(args);
    if (!record) return;
    const path = typeof record.path === 'string' ? record.path : '';
    const edit = cachedEditorEdit(record);
    if (edit === null) return;
    if (edit === 'undo') {
      const last = this.history.get(path)?.pop();
      if (last) this.add({ added: last.removed, removed: last.added });
      return;
    }
    this.add(edit);
    const edits = this.history.get(path) ?? [];
    edits.push(edit);
    this.history.set(path, edits);
  }

  /** A Code Execution result's `_meta`, whatever the script's own outcome. */
  script(meta: Record<string, unknown> | null) {
    const measured = scriptTotal(meta?.[CODE_DELTA_META_KEY]);
    if (measured) {
      this.add(measured);
      return;
    }
    const records = meta?.[NESTED_CALLS_META_KEY];
    if (!Array.isArray(records)) return;
    for (const candidate of records) {
      const record = asRecord(candidate);
      if (record?.status !== 'ok' || typeof record.tool !== 'string') continue;
      if (!EDITOR_TOOLS.has(baseToolName(record.tool)) || typeof record.args !== 'string') continue;
      let args: unknown;
      try {
        args = JSON.parse(record.args);
      } catch {
        // Cut at the record's 2 KB cap: the edit happened, but its size is lost.
        continue;
      }
      this.editorCall(args);
    }
  }
}

function count(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function scriptTotal(value: unknown): CodeDelta | null {
  const record = asRecord(value);
  const added = count(record?.added);
  const removed = count(record?.removed);
  return added === null || removed === null ? null : { added, removed };
}

/** A sequence of successful `text_editor` calls' arguments, in order. */
export function editorCallsDelta(calls: readonly unknown[]): CodeDelta {
  const tally = new CodeDeltaTally();
  for (const args of calls) tally.editorCall(args);
  return { added: tally.added, removed: tally.removed };
}

export function collectCodeDelta(messages: readonly Message[]): CodeDelta {
  const results = new Map<string, Record<string, unknown>>();
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type === 'toolResponse' && !results.has(content.id)) {
        results.set(content.id, content.toolResult);
      }
    }
  }

  const tally = new CodeDeltaTally();
  const seen = new Set<string>();
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type !== 'toolRequest' || seen.has(content.id)) continue;
      seen.add(content.id);
      const call = asRecord(content.toolCall);
      const request = asRecord(call?.value);
      if (call?.status !== 'success' || typeof request?.name !== 'string') continue;
      const result = results.get(content.id);
      const value = asRecord(result?.value);
      if (result?.status !== 'success' || !value) continue;

      if (CODE_EXECUTION_TOOLS.has(request.name)) {
        // A script that failed part-way still made the edits before the
        // failure, so its outcome does not gate them; each sub-call's does.
        tally.script(asRecord(value._meta));
        continue;
      }
      if (!EDITOR_TOOLS.has(baseToolName(request.name))) continue;
      // `isError` is the wire spelling; `is_error` is tolerated, as elsewhere.
      if (value.isError === true || value.is_error === true) continue;
      tally.editorCall(request.arguments);
    }
  }
  return { added: tally.added, removed: tally.removed };
}
