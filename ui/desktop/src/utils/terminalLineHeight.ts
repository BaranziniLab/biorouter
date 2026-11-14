/**
 * The `lineHeight` an xterm terminal needs for design.md 9.15's code rhythm: 12px type on a
 * 20px line, as Codex sets code.
 *
 * xterm multiplies its `lineHeight` by the cell height it measures for the font at
 * `line-height: normal` (about 1.25 times the size: 15px for SF Mono at 12px), not by the font
 * size. `20 / 12` therefore drew 25px rows in the terminal dock in every theme (Phase C, F-12).
 * The multiplier is the wanted row over that measured height, measured the way xterm measures
 * it: one span of the font, at its natural line height.
 */

/** design.md 9.15: a 20px line for 12px type. */
export const TERMINAL_ROW_PER_FONT_PX = 20 / 12;

/** The natural cell height of 12px monospace when nothing can be measured (jsdom, no body). */
const FALLBACK_NATURAL_PER_FONT_PX = 15 / 12;

/** The cell height xterm will measure for `fontFamily` at `fontSize`, or 0 when nothing lays out. */
function naturalCellHeight(fontFamily: string, fontSize: number, doc: Document): number {
  const body = doc.body;
  if (!body) return 0;
  const span = doc.createElement('span');
  span.textContent = 'W';
  span.setAttribute('aria-hidden', 'true');
  Object.assign(span.style, {
    position: 'absolute',
    visibility: 'hidden',
    whiteSpace: 'pre',
    lineHeight: 'normal',
    fontFamily,
    fontSize: `${fontSize}px`,
  });
  body.appendChild(span);
  const height = span.getBoundingClientRect().height;
  span.remove();
  return Number.isFinite(height) ? height : 0;
}

/**
 * The xterm `lineHeight` that gives `fontSize` type a row of `fontSize * 20/12` pixels, for the
 * face `fontFamily` resolves to now. Ask again after a font registers or the size changes: the
 * measured cell height moves with both.
 */
export function terminalLineHeight(
  fontFamily: string,
  fontSize: number,
  doc: Document = document
): number {
  const row = fontSize * TERMINAL_ROW_PER_FONT_PX;
  const natural =
    naturalCellHeight(fontFamily, fontSize, doc) || fontSize * FALLBACK_NATURAL_PER_FONT_PX;
  // xterm refuses a lineHeight under 1.
  return Math.max(1, row / natural);
}
