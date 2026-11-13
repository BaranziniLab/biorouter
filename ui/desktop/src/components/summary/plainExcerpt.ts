/**
 * A subagent's result line as plain text, for one muted line in the roster and
 * its peek (R2-5, live QA 2026-11-10 and 2026-11-11).
 *
 * The daemon's result line is the first line of the child's answer, verbatim
 * (`result_line` in `crates/biorouter/src/agents/subagent_roster.rs`), and an
 * answer is markdown. The excerpt keeps the words and drops the markup: a
 * heading's `#`s, a list or quote marker, emphasis, code ticks, and a link's
 * target.
 *
 * Some first lines are not a summary at all, and have no excerpt, so the
 * caller shows the status word ("Completed", "Failed") instead:
 *
 * - **a table row.** A review's answer began with a table, and its row read
 *   "Abstract · Flaw type(s) · Evidence · Bri…": the column headings, which say
 *   nothing about how the run went. The first fix (R2-5) joined the cells, which
 *   only made the markup quieter. A row is recognised by a leading `|`, or by two
 *   or more `|` in one line (GFM allows a table without the outer pipes);
 * - **a code fence** (```` ``` ```` or `~~~`, with or without a language);
 * - **a line that is only markup**: a table's `|---|---|` rule, a `---` break.
 */
export function plainExcerpt(line: string | null | undefined): string {
  let text = (line ?? '').replace(/\s+/g, ' ').trim();
  if (!text) return '';

  if (isTableRow(text) || /^(?:`{3,}|~{3,})/.test(text)) return '';

  text = text
    // Heading, quote and list markers at the start.
    .replace(/^#{1,6}\s+/, '')
    .replace(/^(?:>\s*)+/, '')
    .replace(/^(?:[-*+]|\d+[.)])\s+/, '')
    // Images and links keep their words.
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    // Emphasis and code.
    .replace(/(\*\*|__)(.+?)\1/g, '$2')
    .replace(/(^|[^\w*])\*(?!\s)([^*]+?)\*(?!\w)/g, '$1$2')
    .replace(/(^|[^\w_])_(?!\s)([^_]+?)_(?!\w)/g, '$1$2')
    .replace(/~~(.+?)~~/g, '$1')
    .replace(/`+([^`]*)`+/g, '$1')
    .trim();

  // Nothing but rules and markers left.
  return /^[\s|:\-*_#>`~=]*$/.test(text) ? '' : text;
}

/**
 * A markdown table row or rule. A pipe inside code ticks is not a cell
 * boundary, so code spans are set aside before counting.
 */
function isTableRow(text: string): boolean {
  if (text.startsWith('|')) return true;
  const outsideCode = text.replace(/`+[^`]*`+/g, '');
  return (outsideCode.match(/\|/g) ?? []).length >= 2;
}
