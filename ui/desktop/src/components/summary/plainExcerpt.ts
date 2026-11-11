/**
 * A subagent's result line as plain text, for one muted line in the roster and
 * its peek (R2-5, live QA 2026-11-10).
 *
 * The daemon's result line is the first line of the child's answer, verbatim,
 * and an answer is markdown: a lab collector's began with a table, so its row
 * read "| Patient | sodium (Na) | potassium …". The excerpt keeps the words and
 * drops the markup: a table row's cells joined with " · ", a heading's `#`s, a
 * list or quote marker, emphasis, code ticks, and a link's target. A line that
 * is only markup (a table's `|---|---|` rule) has no excerpt, so the caller
 * shows the status word instead.
 */
export function plainExcerpt(line: string | null | undefined): string {
  let text = (line ?? '').replace(/\s+/g, ' ').trim();
  if (!text) return '';

  // A table row: its cells, without the rule between header and body.
  if (/^\|.*\|?$/.test(text)) {
    const cells = text
      .replace(/^\|/, '')
      .replace(/\|$/, '')
      .split('|')
      .map((cell) => cell.trim())
      .filter((cell) => cell && !/^:?-{2,}:?$/.test(cell));
    text = cells.join(' · ');
  }

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
