/**
 * Draw only the Dictionary rows near the viewport ("Page layout" in
 * `docs/dictation/dictionary.md`), free of React and the DOM.
 *
 * The page holds and arranges every row the listing answers, up to the store's
 * 5,000 active rows a tier plus the suggestions, but a row is a context menu, a
 * dropdown and two tooltips: 4,991 of them took 7.9 s to mount in the
 * development build, and every write, menu or dialog after that paid seconds
 * restyling them. So the page draws the rows in view and an overscan on each
 * side, and folds every run of rows outside them into one placeholder exactly
 * their height, so the scroll height and every drawn row's place are what the
 * real rows would give. The approach is the sidebar chat list's (R-28,
 * `BioRouterSidebar/sidebarChatView.ts`), for a list whose rows all have one
 * height.
 *
 * Lines are counted down the page: each group is one header line, then one
 * line per row.
 */

/** The lines drawn for real, `[from, to)`. */
export interface DrawRange {
  from: number;
  to: number;
}

/** Lines drawn before the list has been measured: more than a tall window shows. */
export const DICTIONARY_FIRST_DRAW = 48;
/** Lines drawn past each edge of the viewport, so a scroll has rows to show. */
export const DICTIONARY_OVERSCAN = 12;
/**
 * A row's height and the space below it before one has been measured: two lines
 * of text, 10px of padding above and below, and the row's 1px bottom edge. The
 * real pitch is read off a drawn row, because the Text size preference scales it.
 */
export const DICTIONARY_ROW_PITCH_GUESS = 57;

export const FIRST_DRAW: DrawRange = { from: 0, to: DICTIONARY_FIRST_DRAW };

/** One line of a group as the list draws it: a row, or a run of rows held but not drawn. */
export type DrawnLine<T> = { kind: 'row'; entry: T } | { kind: 'gap'; rows: number; key: string };

/** The line of each group's first row (its header is the line before it). */
export function rowsLines(groups: ReadonlyArray<{ entries: readonly unknown[] }>): number[] {
  const lines: number[] = [];
  let line = 0;
  for (const group of groups) {
    line += 1;
    lines.push(line);
    line += group.entries.length;
  }
  return lines;
}

/**
 * A group's rows as drawn: those whose line is in `range`, and every row `keep`
 * names (the row that holds focus), each on its own line between the gaps, so
 * the drawn rows read in the list's order.
 */
export function sliceRows<T>(
  entries: readonly T[],
  rowsLine: number,
  range: DrawRange,
  keep: (entry: T) => boolean = () => false
): DrawnLine<T>[] {
  const lines: DrawnLine<T>[] = [];
  let gap = 0;
  let gapStart = 0;
  const closeGap = () => {
    if (gap > 0) lines.push({ kind: 'gap', rows: gap, key: `gap:${gapStart}` });
    gap = 0;
  };
  entries.forEach((entry, index) => {
    const line = rowsLine + index;
    if ((line >= range.from && line < range.to) || keep(entry)) {
      closeGap();
      lines.push({ kind: 'row', entry });
    } else {
      if (gap === 0) gapStart = index;
      gap += 1;
    }
  });
  closeGap();
  return lines;
}

/** Where a drawn group sits, in the scroll container's content coordinates (px). */
export interface GroupBox {
  /** The line of the group's first row. */
  rowsLine: number;
  /** Rows the group holds, drawn or not. */
  rows: number;
  /** The top of the group's box, and of its rows' list. */
  top: number;
  rowsTop: number;
}

/**
 * The first and last line inside `[viewTop, viewBottom]`, from the groups'
 * boxes (in page order): rows and gaps sit on one pitch below their group's
 * first row, so a line is exact without measuring every row. Null when nothing
 * can be measured.
 */
export function linesInView(
  boxes: readonly GroupBox[],
  viewTop: number,
  viewBottom: number,
  pitch: number
): { first: number; last: number } | null {
  if (boxes.length === 0 || !(viewBottom > viewTop) || !(pitch > 0)) return null;
  const lineAt = (y: number) => {
    let found = 0;
    for (let index = 0; index < boxes.length; index++) {
      if (boxes[index].top <= y) found = index;
      else break;
    }
    const box = boxes[found];
    if (y < box.rowsTop || box.rows === 0) return box.rowsLine - 1;
    return box.rowsLine + Math.min(box.rows - 1, Math.floor((y - box.rowsTop) / pitch));
  };
  return { first: lineAt(viewTop), last: lineAt(viewBottom) };
}

/**
 * The range to draw for the lines in view: those lines plus the overscan on
 * each side, kept as it is (the same object, so the list does not re-render)
 * while the lines in view stay at least half an overscan inside it. A null
 * `inView` (the list could not be measured) keeps whatever was drawn.
 */
export function nextDrawRange(
  current: DrawRange,
  inView: { first: number; last: number } | null,
  overscan: number = DICTIONARY_OVERSCAN
): DrawRange {
  if (!inView) return current;
  const wanted = { from: Math.max(0, inView.first - overscan), to: inView.last + 1 + overscan };
  const margin = Math.floor(overscan / 2);
  const coversTop = current.from <= Math.max(0, inView.first - margin);
  const coversBottom = current.to >= inView.last + 1 + margin;
  const bounded = current.to - current.from <= wanted.to - wanted.from + overscan;
  if (coversTop && coversBottom && bounded) return current;
  return wanted;
}
