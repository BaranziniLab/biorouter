/**
 * `browser_read`: the page, or one element, as Markdown (BB-09, BB-14).
 *
 * The function runs inside the page through `Runtime.callFunctionOn` in an isolated world, so a
 * page cannot replace `getComputedStyle` or the DOM walkers it uses. It reads text only: the
 * values of form fields are never read (BB-13 keeps a password or a code away from the model
 * even when a page renders it into a field), and scripts, styles, frames and hidden elements are
 * skipped. `this` is the element to read; with no element it reads the document's body.
 *
 * Exported as source text, and as a callable so the Vitest suite can run it against jsdom.
 */

export function readAsMarkdown(
  this: unknown,
  maxChars: number
): { text: string; truncated: boolean } {
  const SKIP = new Set([
    'SCRIPT',
    'STYLE',
    'NOSCRIPT',
    'TEMPLATE',
    'SVG',
    'CANVAS',
    'IFRAME',
    'FRAME',
    'OBJECT',
    'EMBED',
    'HEAD',
    'META',
    'LINK',
    'INPUT',
    'TEXTAREA',
    'SELECT',
    'OPTION',
    'BUTTON',
  ]);
  const BLOCK = new Set([
    'P',
    'DIV',
    'SECTION',
    'ARTICLE',
    'MAIN',
    'HEADER',
    'FOOTER',
    'NAV',
    'ASIDE',
    'FORM',
    'FIELDSET',
    'FIGURE',
    'FIGCAPTION',
    'DETAILS',
    'SUMMARY',
    'ADDRESS',
    'DL',
    'DT',
    'DD',
    'CAPTION',
    'LABEL',
    'LEGEND',
  ]);
  const limit = Number.isFinite(maxChars) && maxChars > 0 ? Math.floor(maxChars) : 20000;
  const self = this as Node | undefined;
  const root: Element | null =
    self && (self as Node).nodeType === 1
      ? (self as Element)
      : (globalThis as unknown as { document: Document }).document.body ||
        (globalThis as unknown as { document: Document }).document.documentElement;
  const view = (globalThis as unknown as { getComputedStyle?: typeof getComputedStyle })
    .getComputedStyle;
  const blocks: string[] = [];
  let size = 0;
  let truncated = false;

  const hidden = (el: Element): boolean => {
    if ((el as HTMLElement).hidden) return true;
    if (el.getAttribute('aria-hidden') === 'true') return true;
    if (!view) return false;
    try {
      const style = view(el);
      return style.display === 'none' || style.visibility === 'hidden';
    } catch {
      return false;
    }
  };
  const squash = (text: string) => text.replace(/\s+/g, ' ');
  const push = (block: string) => {
    if (truncated) return;
    const trimmed = block.replace(/[ \t]+\n/g, '\n').trim();
    if (!trimmed) return;
    if (size + trimmed.length + 2 > limit) {
      const room = limit - size - 2;
      if (room > 20) blocks.push(trimmed.slice(0, room));
      truncated = true;
      return;
    }
    blocks.push(trimmed);
    size += trimmed.length + 2;
  };

  const inline = (node: Node): string => {
    if (node.nodeType === 3) return squash(node.nodeValue || '');
    if (node.nodeType !== 1) return '';
    const el = node as Element;
    if (SKIP.has(el.tagName) || hidden(el)) {
      // A button's label is still prose a reader needs ("Show more").
      if (el.tagName === 'BUTTON' && !hidden(el)) return squash(el.textContent || '');
      return '';
    }
    const inner = () => Array.from(el.childNodes).map(inline).join('');
    switch (el.tagName) {
      case 'BR':
        return '\n';
      case 'A': {
        const text = inner().trim();
        const href = (el as HTMLAnchorElement).href;
        if (!text) return '';
        return href && /^https?:/i.test(href) ? `[${text}](${href})` : text;
      }
      case 'STRONG':
      case 'B': {
        const text = inner().trim();
        return text ? `**${text}**` : '';
      }
      case 'EM':
      case 'I': {
        const text = inner().trim();
        return text ? `*${text}*` : '';
      }
      case 'CODE':
        return `\`${(el.textContent || '').trim()}\``;
      case 'IMG': {
        const alt = (el.getAttribute('alt') || '').trim();
        return alt ? `![${alt}]` : '';
      }
      default:
        return inner();
    }
  };

  const tableToMarkdown = (table: Element): string => {
    const rows = Array.from(table.querySelectorAll('tr')).slice(0, 200);
    const cells = rows.map((row) =>
      Array.from(row.children)
        .filter((cell) => cell.tagName === 'TD' || cell.tagName === 'TH')
        .map((cell) => inline(cell).trim().replace(/\|/g, '\\|').replace(/\n/g, ' '))
    );
    const width = Math.max(0, ...cells.map((row) => row.length));
    if (width === 0) return '';
    const padded = cells.map((row) => [...row, ...Array(width - row.length).fill('')]);
    const lines = [`| ${padded[0].join(' | ')} |`, `|${' --- |'.repeat(width)}`];
    for (const row of padded.slice(1)) lines.push(`| ${row.join(' | ')} |`);
    const more = table.querySelectorAll('tr').length - rows.length;
    if (more > 0) lines.push(`(${more} more rows)`);
    return lines.join('\n');
  };

  const listLines = (list: Element, depth: number): string[] => {
    const lines: string[] = [];
    let index = 0;
    for (const child of Array.from(list.children)) {
      if (child.tagName !== 'LI' || hidden(child)) continue;
      index += 1;
      const marker = list.tagName === 'OL' ? `${index}.` : '-';
      const own = Array.from(child.childNodes)
        .filter((node) => !(node.nodeType === 1 && /^(UL|OL)$/.test((node as Element).tagName)))
        .map(inline)
        .join('')
        .trim();
      lines.push(`${'  '.repeat(depth)}${marker} ${own}`);
      for (const nested of Array.from(child.children)) {
        if ((nested.tagName === 'UL' || nested.tagName === 'OL') && !hidden(nested)) {
          lines.push(...listLines(nested, depth + 1));
        }
      }
    }
    return lines;
  };

  const walk = (el: Element, listDepth: number) => {
    if (truncated) return;
    if (SKIP.has(el.tagName) || hidden(el)) return;
    const tag = el.tagName;
    const heading = /^H([1-6])$/.exec(tag);
    if (heading) {
      push(`${'#'.repeat(Number(heading[1]))} ${inline(el).trim()}`);
      return;
    }
    if (tag === 'TABLE') {
      push(tableToMarkdown(el));
      return;
    }
    if (tag === 'PRE') {
      push(`\`\`\`\n${(el.textContent || '').replace(/\n+$/, '')}\n\`\`\``);
      return;
    }
    if (tag === 'HR') {
      push('---');
      return;
    }
    if (tag === 'BLOCKQUOTE') {
      const quoted = inline(el).trim();
      if (quoted)
        push(
          quoted
            .split('\n')
            .map((line) => `> ${line}`)
            .join('\n')
        );
      return;
    }
    if (tag === 'UL' || tag === 'OL') {
      push(listLines(el, listDepth).join('\n'));
      return;
    }
    // A block of mixed content: its inline runs become paragraphs, its blocks recurse.
    let run = '';
    const flush = () => {
      if (run.trim()) push(run);
      run = '';
    };
    for (const child of Array.from(el.childNodes)) {
      if (child.nodeType === 1) {
        const childEl = child as Element;
        const isBlock =
          BLOCK.has(childEl.tagName) ||
          /^(H[1-6]|TABLE|PRE|HR|BLOCKQUOTE|UL|OL|LI)$/.test(childEl.tagName);
        if (isBlock) {
          flush();
          walk(childEl, listDepth);
          continue;
        }
      }
      run += inline(child);
    }
    flush();
  };

  if (root) walk(root, 0);
  return { text: blocks.join('\n\n'), truncated };
}

/** The function as source, for `Runtime.callFunctionOn`. */
export const READ_AS_MARKDOWN_FUNCTION = readAsMarkdown.toString();
