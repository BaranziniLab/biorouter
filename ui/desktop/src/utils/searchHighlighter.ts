/**
 * SearchHighlighter provides overlay-based text search highlighting
 * with support for navigation and scrolling control.
 *
 * ⚠ **A match is what the user can see, or scroll to — not every occurrence in
 * the DOM.** The overlay used to get one mark per regex hit, positioned from the
 * hit's client rects with no regard for what clips them. On Settings → Skills a
 * description is `line-clamp-1` (clientHeight 16, scrollHeight 48), so the hits
 * on its two clamped-away lines were painted over the path line and the row
 * padding below, and the counter ("1/6") counted them — navigating to one
 * showed a mark over unrelated text, or nothing. The rule now, per match:
 *
 * - **A clip the page imposes hides it.** An ancestor with `overflow: hidden` or
 *   `clip` — a line clamp, a truncated line, a folded message — has taken that
 *   text off screen, and nothing the user does at the search bar brings it back.
 *   A match counts only if at least half its height and some of its width are
 *   inside every such box, and only that part is painted. Text that is not
 *   rendered (`display: none`, `visibility: hidden`) does not count either.
 * - **An ellipsis hides what it replaced.** A truncated line (`text-overflow:
 *   ellipsis`) and the last line of a clamped block do not just clip at the box
 *   edge: the line truncator stops painting glyphs early to make room for the
 *   "…", and every glyph it dropped keeps a rect under or before the ellipsis.
 *   Clipping at the box edge alone counted those, and painted a sliver of a mark
 *   over the "…" — Settings → Skills, `align`: "alignment-structural" at left
 *   1114.9 in a box ending at 1120 read as match 7 of 13. See
 *   {@link trimRectToEllipsis}, whose rule was checked against the pixels
 *   Chromium paints, not inferred from rects.
 * - **A scroll container does not.** The transcript's own scroller is where the
 *   overlay lives, so a match below the fold counts and is painted in place. An
 *   inner scroller (a code block with `overflow-x: auto`) is the same promise one
 *   level down: its hidden text counts, is painted only inside the block, and
 *   navigating to it scrolls the block until it shows.
 *
 * So the counter agrees with the marks: every counted match is either painted
 * or one scroll away, and nothing is painted outside the box that shows it.
 * Every approximation below errs toward showing a match rather than hiding one.
 *
 * - **The search UI is not content.** The bar is rendered inside the container
 *   it searches, so its own text is skipped — see {@link SEARCH_UI_ATTRIBUTE}.
 */

/**
 * Marks the search UI, which lives inside the container it searches, as not
 * content. Put it on the root of anything the search renders there.
 *
 * ⚠ **An attribute this module exports, not a class name each side spells.**
 * The walker used to skip `.search-bar, .search-results`, and no element carried
 * either class: `SearchBar`'s root is `search-bar-enter` / `search-bar-exit`. So
 * the bar's "Aa" toggle was walked, counted and marked under the button, where
 * nobody sees a mark — ⌘F then `aa` in a chat read "1/1" with nothing
 * highlighted, and on Settings → Skills `a` read 1/18 with the first two ⌘G
 * stops on that label. `SearchView.searchUi.test.tsx` drives the real bar over
 * the real walker, so a rename on either side fails there.
 */
export const SEARCH_UI_ATTRIBUTE = 'data-search-ui';

/** Whether `node` belongs to the search UI rather than to what it searches. */
function isSearchUi(node: Node): boolean {
  const element = node instanceof Element ? node : node.parentElement;
  return Boolean(element?.closest(`[${SEARCH_UI_ATTRIBUTE}]`));
}

/** A rectangle in viewport coordinates. */
export interface ViewportRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/**
 * What a box does to the text past one of its edges, per axis.
 *
 * `scrolls` (`auto` / `scroll`): that text is one scroll away, so the box limits
 * where a mark is painted but not whether the match counts. `hides` (`hidden` /
 * `clip`): the page has taken that text off screen, and a match wholly behind
 * it is not a match. Per axis because `overflow-x: hidden; overflow-y: auto` is
 * a common pair, and scrolling the hidden axis to "reveal" a match would shift
 * content the user has no way to scroll back.
 */
export type OverflowMode = 'visible' | 'hides' | 'scrolls';

/** A box that clips the text inside it, in viewport coordinates. */
export interface ClipBox extends ViewportRect {
  x: OverflowMode;
  y: OverflowMode;
}

/**
 * The share of a match's height that must be inside a hiding clip for it to
 * count. A glyph box can overhang its line box, so the line after a clamp can
 * poke a pixel or two into the visible box; half the height separates that
 * sliver from a line the clip really cuts through.
 */
export const MIN_VISIBLE_HEIGHT_FRACTION = 0.5;

/** A match cut sideways (a truncated line) counts while any of it shows. */
export const MIN_VISIBLE_WIDTH_PX = 1;

/** `rect` limited to `clip` on the axes where the clip's mode is `mode`. */
const intersect = (rect: ViewportRect, clip: ClipBox, mode: OverflowMode): ViewportRect => ({
  left: clip.x === mode ? Math.max(rect.left, clip.left) : rect.left,
  right: clip.x === mode ? Math.min(rect.right, clip.right) : rect.right,
  top: clip.y === mode ? Math.max(rect.top, clip.top) : rect.top,
  bottom: clip.y === mode ? Math.min(rect.bottom, clip.bottom) : rect.bottom,
});

/**
 * Where one of a match's client rects stands against the boxes that clip it.
 *
 * `counts` — the rect survives every hiding clip (see
 * {@link MIN_VISIBLE_HEIGHT_FRACTION}). `paint` — the part to draw, further
 * limited by scroll containers; `null` when a scroller has it out of view.
 */
export function clipMatchRect(
  rect: ViewportRect,
  clips: readonly ClipBox[]
): { counts: boolean; paint: ViewportRect | null } {
  const height = rect.bottom - rect.top;
  if (height <= 0 || rect.right - rect.left <= 0) return { counts: false, paint: null };

  let shown = rect;
  for (const clip of clips) shown = intersect(shown, clip, 'hides');
  const counts =
    shown.right - shown.left >= MIN_VISIBLE_WIDTH_PX &&
    shown.bottom - shown.top >= height * MIN_VISIBLE_HEIGHT_FRACTION;
  if (!counts) return { counts: false, paint: null };

  let paint = shown;
  for (const clip of clips) paint = intersect(paint, clip, 'scrolls');
  const visible = paint.right - paint.left >= 1 && paint.bottom - paint.top >= 1;
  return { counts: true, paint: visible ? paint : null };
}

/**
 * Where a left-to-right line's ellipsis leaves off painting glyphs: its end
 * edge, moved left by the width of the "…".
 *
 * Left-to-right only, deliberately. In a right-to-left block the truncator
 * drops glyphs from the LOGICAL end, which is the visual right of a Latin run
 * and the visual left of a Hebrew one, so no single edge describes it. Such a
 * block gets no ellipsis model at all and falls back to the overflow clip,
 * which can paint a stray mark but never hides a real one.
 */
export interface EllipsisCut {
  at: number;
}

/**
 * Slack between a glyph edge and the cut. Chromium lays out in 1/64px units and
 * the ellipsis width is measured here on a canvas. In the app the last painted
 * glyph ended 0.47px or more before the cut and the first dropped one 2.4px or
 * more after it, so this only absorbs rounding.
 */
export const ELLIPSIS_EPSILON_PX = 0.02;

/** A glyph's horizontal extent. */
export interface GlyphSpan {
  left: number;
  right: number;
}

/** Whether the line truncator kept (and so painted) this glyph. */
export function glyphPaintedBeforeEllipsis(glyph: GlyphSpan, cut: EllipsisCut): boolean {
  return glyph.right <= cut.at + ELLIPSIS_EPSILON_PX;
}

/**
 * The part of one match rect an ellipsized line still paints, or `null` when
 * the ellipsis took all of it.
 *
 * ⚠ **Whole glyphs, not pixels.** The line truncator keeps the longest run of
 * glyphs that fits in the line minus the ellipsis's width — a glyph is painted
 * exactly when its end edge is at or before `lineEnd − width("…")` — and then
 * places the "…" right after the last glyph it kept. So the "…" can start
 * several pixels before the box edge, and a glyph wholly inside the box can
 * still be gone: the hyphen of "receiving-code-review" at x 1071–1075 sat under
 * an ellipsis in a box ending at 1084. Clipping the rect at a pixel would have
 * painted that hyphen; asking each glyph does not.
 *
 * `glyphs` are the match's glyph rects on this rect's line. What stays runs from
 * the rect's start edge to the far edge of the last glyph the truncator kept.
 * Call this only for a line that really carries an ellipsis: on a line that
 * fits, the last few glyphs before the edge are painted.
 */
export function trimRectToEllipsis(
  rect: ViewportRect,
  glyphs: readonly GlyphSpan[],
  cut: EllipsisCut
): ViewportRect | null {
  if (glyphPaintedBeforeEllipsis(rect, cut)) return rect;
  const kept = glyphs.filter((glyph) => glyphPaintedBeforeEllipsis(glyph, cut));
  if (kept.length === 0) return null;
  const right = Math.min(rect.right, Math.max(...kept.map((glyph) => glyph.right)));
  return right > rect.left ? { ...rect, right } : null;
}

/** Half a CSS pixel: bounding rects are fractional. */
const SAME_RECT_PX = 0.5;

/**
 * `rects` with near-duplicates removed. A truncated line keeps the glyphs it
 * shows as a second fragment beside the original one, so a match on it reports
 * the same rect twice — and was painted twice, reading darker than every other
 * mark on the page.
 */
export function withoutDuplicateRects<T extends ViewportRect>(rects: readonly T[]): T[] {
  const kept: T[] = [];
  for (const rect of rects) {
    const duplicate = kept.some(
      (other) =>
        Math.abs(other.left - rect.left) < SAME_RECT_PX &&
        Math.abs(other.right - rect.right) < SAME_RECT_PX &&
        Math.abs(other.top - rect.top) < SAME_RECT_PX &&
        Math.abs(other.bottom - rect.bottom) < SAME_RECT_PX
    );
    if (!duplicate) kept.push(rect);
  }
  return kept;
}

/**
 * Displays on which `overflow` does nothing: it applies to block, flex and
 * grid containers, and an inline box's rect is the union of its lines, not a
 * clip. Skipping a box that does clip only paints a stray mark; treating a box
 * that does not as a clip would hide a visible match — so the list errs short.
 */
const NON_CLIPPING_DISPLAYS = new Set([
  'inline',
  'contents',
  'none',
  'table',
  'inline-table',
  'table-row',
  'table-row-group',
  'table-header-group',
  'table-footer-group',
  'table-column',
  'table-column-group',
]);

/** Displays whose box does not own the lines of the text inside it. */
const INLINE_DISPLAYS = new Set(['inline', 'contents']);

/**
 * Block containers: the boxes that lay text out in line boxes of their own, and
 * so the only ones a `text-overflow` or a line clamp can put an ellipsis on.
 * Text directly in a flex or grid container is wrapped in an anonymous item,
 * which does not inherit `text-overflow` — so a `truncate` on a flex row clips
 * but never ellipsizes, and is left to the overflow rule.
 */
const BLOCK_CONTAINER_DISPLAYS = new Set([
  'block',
  'inline-block',
  'flow-root',
  'list-item',
  'table-cell',
  'table-caption',
  '-webkit-box',
  '-webkit-inline-box',
]);

function overflowMode(value: string): OverflowMode {
  if (value === 'visible') return 'visible';
  return value === 'auto' || value === 'scroll' || value === 'overlay' ? 'scrolls' : 'hides';
}

/** The padding box without scrollbars — what `overflow` clips to. */
function paddingBoxOf(element: Element): ViewportRect {
  const box = element.getBoundingClientRect();
  const left = box.left + element.clientLeft;
  const top = box.top + element.clientTop;
  return { left, top, right: left + element.clientWidth, bottom: top + element.clientHeight };
}

/**
 * The content box: the edges line boxes are laid out between.
 *
 * ⚠ **From the fractional border box, not `clientWidth`.** `clientWidth` is
 * rounded to a whole pixel, and the truncator cuts against the real width: in
 * the app the last glyph it kept ended 0.47px before the cut, so a rounded edge
 * can move the cut past it. Only the scrollbar gutter is taken from the rounded
 * sizes, and a box an ellipsis can sit in usually has none.
 */
function contentBoxOf(element: Element, style: CSSStyleDeclaration): ViewportRect {
  const box = element.getBoundingClientRect();
  const px = (value: string) => parseFloat(value) || 0;
  const borderLeft = px(style.borderLeftWidth);
  const borderRight = px(style.borderRightWidth);
  const borderTop = px(style.borderTopWidth);
  const borderBottom = px(style.borderBottomWidth);
  const html = element as HTMLElement;
  const gutter = (outer: number, inner: number, borders: number) =>
    typeof outer === 'number' && outer - inner - borders >= 1 ? outer - inner - borders : 0;
  return {
    left: box.left + borderLeft + px(style.paddingLeft),
    top: box.top + borderTop + px(style.paddingTop),
    right:
      box.right -
      borderRight -
      gutter(html.offsetWidth, element.clientWidth, borderLeft + borderRight) -
      px(style.paddingRight),
    bottom:
      box.bottom -
      borderBottom -
      gutter(html.offsetHeight, element.clientHeight, borderTop + borderBottom) -
      px(style.paddingBottom),
  };
}

interface ElementLayout {
  position: string;
  rendered: boolean;
  clip: ClipBox | null;
  style: CSSStyleDeclaration;
}

type LayoutCache = Map<Element, ElementLayout>;

function layoutOf(element: Element, cache: LayoutCache): ElementLayout {
  const cached = cache.get(element);
  if (cached) return cached;

  const style = getComputedStyle(element);
  const x = overflowMode(style.overflowX);
  const y = overflowMode(style.overflowY);
  let clip: ClipBox | null = null;
  if ((x !== 'visible' || y !== 'visible') && !NON_CLIPPING_DISPLAYS.has(style.display)) {
    clip = { ...paddingBoxOf(element), x, y };
  }
  const layout = {
    position: style.position,
    rendered: style.visibility === 'visible',
    clip,
    style,
  };
  cache.set(element, layout);
  return layout;
}

interface ClipChain {
  clips: ClipBox[];
  /** Scroll containers between the text and the overlay host, innermost first. */
  scrollers: { element: HTMLElement; x: boolean; y: boolean }[];
}

/**
 * The boxes that clip `start`'s content, walking up to (not including) `host`.
 *
 * `host` is excluded because the overlay lives inside it and is clipped by it
 * like the text is. An absolutely positioned box escapes the clips of the
 * static ancestors between it and its containing block, and a fixed one
 * escapes all of them; transforms that would re-capture either are not
 * modelled, which can only under-clip.
 */
function clipChainOf(start: Element, host: Element, cache: LayoutCache): ClipChain {
  const clips: ClipBox[] = [];
  const scrollers: ClipChain['scrollers'] = [];
  let escapingToPositioned = false;
  for (let el: Element | null = start; el && el !== host; el = el.parentElement) {
    const layout = layoutOf(el, cache);
    const isContainingBlock = layout.position !== 'static';
    if (!escapingToPositioned || isContainingBlock) {
      escapingToPositioned = false;
      if (layout.clip) {
        clips.push(layout.clip);
        const { x, y } = layout.clip;
        if ((x === 'scrolls' || y === 'scrolls') && el instanceof HTMLElement) {
          scrollers.push({ element: el, x: x === 'scrolls', y: y === 'scrolls' });
        }
      }
    }
    if (layout.position === 'fixed') break;
    if (layout.position === 'absolute') escapingToPositioned = true;
  }
  return { clips, scrollers };
}

/** The lines one block container lays out, and the ellipses it may draw on them. */
interface LineBlock {
  element: Element;
  content: ViewportRect;
  cut: EllipsisCut;
  /** `text-overflow: ellipsis` with a clipping `overflow-x`: any line that overflows gets one. */
  ellipsizesOverflow: boolean;
  /**
   * A clamp that cut content off: its last shown line gets an ellipsis whether
   * or not that line overflows. `lastLineBottom` is the clamp box's content
   * bottom, which that line ends at.
   */
  clampedLastLineBottom: number | null;
  /** Per line (keyed by its rounded middle): does its content run past the end edge? */
  overflowByLine: Map<number, boolean>;
}

let ellipsisCanvas: CanvasRenderingContext2D | null | undefined;
const ellipsisWidths = new Map<string, number>();

/**
 * The advance of "…" in `style`'s font — the space the truncator reserves.
 * `null` where nothing can measure it (jsdom has no canvas), in which case no
 * ellipsis is modelled and matches are left to the overflow rule.
 */
function ellipsisWidthIn(style: CSSStyleDeclaration): number | null {
  const font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  const cached = ellipsisWidths.get(font);
  if (cached !== undefined) return cached;
  if (ellipsisCanvas === undefined) {
    try {
      ellipsisCanvas =
        typeof navigator !== 'undefined' && /jsdom/i.test(navigator.userAgent)
          ? null
          : (document.createElement('canvas').getContext('2d') ?? null);
    } catch {
      ellipsisCanvas = null;
    }
  }
  if (!ellipsisCanvas) return null;
  ellipsisCanvas.font = font;
  const width = ellipsisCanvas.measureText('…').width;
  ellipsisWidths.set(font, width);
  return width;
}

const lineClampOf = (style: CSSStyleDeclaration): boolean => {
  const value = style.webkitLineClamp || style.getPropertyValue('line-clamp');
  return Boolean(value) && value !== 'none';
};

/**
 * The block container whose line boxes hold `start`'s text, if it can draw an
 * ellipsis on them. `null` for text whose lines can carry none.
 */
function lineBlockOf(start: Element, host: Element, cache: LayoutCache): LineBlock | null {
  let el: Element | null = start;
  while (el && el !== host && INLINE_DISPLAYS.has(layoutOf(el, cache).style.display)) {
    el = el.parentElement;
  }
  if (!el || el === host) return null;
  const { style } = layoutOf(el, cache);
  if (!BLOCK_CONTAINER_DISPLAYS.has(style.display) || style.direction !== 'ltr') return null;

  const ellipsizesOverflow = style.textOverflow === 'ellipsis' && style.overflowX !== 'visible';

  // A clamp counts lines through nested blocks, so it may sit further up.
  let clampedLastLineBottom: number | null = null;
  for (let up: Element | null = el; up && up !== host; up = up.parentElement) {
    const upStyle = layoutOf(up, cache).style;
    if (!lineClampOf(upStyle)) continue;
    // No ellipsis unless the clamp actually cut something off.
    if (up.scrollHeight > up.clientHeight + 1) {
      clampedLastLineBottom = contentBoxOf(up, upStyle).bottom;
    }
    break;
  }
  if (!ellipsizesOverflow && clampedLastLineBottom === null) return null;

  const width = ellipsisWidthIn(style);
  if (width === null) return null;
  const content = contentBoxOf(el, style);
  return {
    element: el,
    content,
    cut: { at: content.right - width },
    ellipsizesOverflow,
    clampedLastLineBottom,
    overflowByLine: new Map(),
  };
}

/** Whether the line through `rect` ends in an ellipsis. */
function lineCarriesEllipsis(block: LineBlock, rect: ViewportRect): boolean {
  const height = rect.bottom - rect.top;
  const last = block.clampedLastLineBottom;
  // The clamp's last shown line ends at the box's content bottom; a line whose
  // lower half reaches it is that line (the one above ends a whole line higher).
  if (last !== null && rect.top < last && rect.bottom + height / 2 > last) {
    return true;
  }
  if (!block.ellipsizesOverflow) return false;

  const middle = (rect.top + rect.bottom) / 2;
  const key = Math.round(middle);
  const known = block.overflowByLine.get(key);
  if (known !== undefined) return known;

  // A truncated line's content keeps its untruncated rects, so the line
  // overflows exactly when one of them runs past the end edge.
  const range = document.createRange();
  range.selectNodeContents(block.element);
  let overflows = false;
  for (const line of Array.from(range.getClientRects())) {
    if (line.top > middle || line.bottom < middle) continue;
    if (line.right > block.content.right + SAME_RECT_PX) {
      overflows = true;
      break;
    }
  }
  block.overflowByLine.set(key, overflows);
  return overflows;
}

/** The rects of the glyphs in `node[start, end)` whose middle is on `rect`'s line. */
function glyphsOnLine(node: Text, start: number, end: number, rect: ViewportRect): GlyphSpan[] {
  const range = document.createRange();
  const glyphs: GlyphSpan[] = [];
  for (let offset = start; offset < end; ) {
    const length = (node.data.codePointAt(offset) ?? 0) > 0xffff ? 2 : 1;
    range.setStart(node, offset);
    range.setEnd(node, Math.min(offset + length, end));
    const glyph = range.getBoundingClientRect();
    const middle = (glyph.top + glyph.bottom) / 2;
    if (glyph.width > 0 && middle >= rect.top && middle <= rect.bottom) {
      glyphs.push({ left: glyph.left, right: glyph.right });
    }
    offset += length;
  }
  return glyphs;
}

interface MatchRecord {
  node: Text;
  startOffset: number;
  endOffset: number;
  scrollers: ClipChain['scrollers'];
}

export class SearchHighlighter {
  private readonly container: HTMLElement;
  private readonly overlay: HTMLElement;
  private highlights: HTMLElement[] = [];
  private matches: MatchRecord[] = [];
  private resizeObserver: ResizeObserver;
  private mutationObserver: MutationObserver;
  private scrollContainer: HTMLElement | null = null;
  private currentTerm: string = '';
  private caseSensitive: boolean = false;
  private onMatchesChange?: (count: number) => void;
  private currentMatchIndex: number = -1;
  private isScrollingProgrammatically: boolean = false;
  private highlightTimeout?: ReturnType<typeof setTimeout>;
  private scrollEndTimeout?: ReturnType<typeof setTimeout>;
  private readonly handleScroll = () => {
    if (!this.isScrollingProgrammatically) {
      this.updateHighlightPositions();
    }
  };

  constructor(container: HTMLElement, onMatchesChange?: (count: number) => void) {
    this.container = container;
    this.onMatchesChange = onMatchesChange;

    // Create overlay
    this.overlay = document.createElement('div');
    this.overlay.className = 'search-highlight-overlay';
    this.overlay.style.cssText = `
      position: absolute;
      pointer-events: none;
      top: 0;
      left: 0;
      right: 0;
      bottom: 0;
      z-index: 1;
    `;

    // Find scroll container (look for our custom data attribute first, then fallback to radix)
    const searchScrollArea = container.closest('[data-search-scroll-area]');
    this.scrollContainer =
      searchScrollArea?.querySelector('[data-radix-scroll-area-viewport]') ||
      (searchScrollArea as HTMLElement) ||
      container.closest('[data-radix-scroll-area-viewport]');

    if (this.scrollContainer) {
      this.scrollContainer.style.position = 'relative';
      this.scrollContainer.appendChild(this.overlay);
    } else {
      container.style.position = 'relative';
      container.appendChild(this.overlay);
    }

    // ⚠ Capture phase: `scroll` does not bubble, and an inner scroller (a code
    // block) moving its text moves the part of a mark that should show. The
    // host's own scroll still arrives here, at the target.
    this.overlayHost.addEventListener('scroll', this.handleScroll, {
      passive: true,
      capture: true,
    });

    // Handle content changes
    this.resizeObserver = new ResizeObserver(() => {
      if (this.highlights.length > 0) {
        this.updateHighlightPositions();
      }
    });
    this.resizeObserver.observe(container);

    // Watch for DOM changes (new messages)
    this.mutationObserver = new MutationObserver((mutations) => {
      let shouldUpdate = false;
      for (const mutation of mutations) {
        if (mutation.type === 'childList' && mutation.addedNodes.length > 0) {
          // Ignore mutations from our own overlay
          if (mutation.target === this.overlay || this.overlay.contains(mutation.target as Node)) {
            continue;
          }
          // Nor from the search bar: its counter appearing is not new content,
          // and re-walking the transcript for it cost a full highlight pass.
          if (
            isSearchUi(mutation.target) ||
            Array.from(mutation.addedNodes).every((node) => isSearchUi(node))
          ) {
            continue;
          }
          // Ignore mutations that only add/remove our highlight elements
          const isOnlyHighlights = Array.from(mutation.addedNodes).every(
            (node) =>
              node instanceof HTMLElement &&
              (node.classList.contains('search-highlight') ||
                node.classList.contains('search-highlight-container'))
          );
          if (isOnlyHighlights) {
            continue;
          }
          shouldUpdate = true;
          break;
        }
      }
      if (shouldUpdate && this.currentTerm) {
        // Debounce the highlight update to avoid rapid re-highlighting
        if (this.highlightTimeout) {
          clearTimeout(this.highlightTimeout);
        }
        this.highlightTimeout = setTimeout(() => {
          this.highlight(this.currentTerm, this.caseSensitive);
        }, 100);
      }
    });
    this.mutationObserver.observe(container, { childList: true, subtree: true });
  }

  /** The element the overlay is positioned in, and whose own clip it shares. */
  private get overlayHost(): HTMLElement {
    return this.scrollContainer ?? this.container;
  }

  highlight(term: string, caseSensitive = false) {
    // Store the current match index and count before clearing
    const currentIndex = this.currentMatchIndex;
    const oldHighlightCount = this.highlights.length;

    this.clearHighlights();
    this.currentTerm = term;
    this.caseSensitive = caseSensitive;

    if (!term.trim()) return [];

    const range = document.createRange();
    const regex = new RegExp(
      term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'),
      caseSensitive ? 'g' : 'gi'
    );

    // Find all text nodes in the container
    const walker = document.createTreeWalker(this.container, NodeFilter.SHOW_TEXT, {
      acceptNode: (node) =>
        isSearchUi(node) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
    });

    const hits: { node: Text; startOffset: number; endOffset: number }[] = [];
    let node: Text | null;

    // Find all matches
    while ((node = walker.nextNode() as Text)) {
      const text = node.textContent || '';
      let match;

      // Reset lastIndex to ensure we find all matches
      regex.lastIndex = 0;
      while ((match = regex.exec(text)) !== null) {
        hits.push({
          node,
          startOffset: match.index,
          endOffset: match.index + match[0].length,
        });
      }
    }

    // Measure every hit before writing any mark: appending to the overlay
    // invalidates layout, and interleaving the two forced a reflow per match.
    const host = this.overlayHost;
    const hostRect = host.getBoundingClientRect();
    // The overlay's `top: 0; left: 0` is the host's padding-box corner, which
    // scrolls with its content.
    const originLeft = hostRect.left + host.clientLeft - host.scrollLeft;
    const originTop = hostRect.top + host.clientTop - host.scrollTop;
    const layouts: LayoutCache = new Map();
    const lineBlocks = new Map<Element, LineBlock | null>();

    const measured: { record: MatchRecord; paint: ViewportRect[] }[] = [];
    for (const hit of hits) {
      const parent = hit.node.parentElement;
      if (!parent || !layoutOf(parent, layouts).rendered) continue;

      range.setStart(hit.node, hit.startOffset);
      range.setEnd(hit.node, hit.endOffset);
      const { clips, scrollers } = clipChainOf(parent, host, layouts);
      if (!lineBlocks.has(parent)) lineBlocks.set(parent, lineBlockOf(parent, host, layouts));
      const lineBlock = lineBlocks.get(parent) ?? null;

      let counts = false;
      const paint: ViewportRect[] = [];
      // ⚠ Copied out of the DOMRects: their edges are prototype getters, so
      // `{ ...domRect, right }` is `{ right }` and every other edge is lost.
      const rects = Array.from(range.getClientRects(), ({ left, top, right, bottom }) => ({
        left,
        top,
        right,
        bottom,
      }));
      for (const rect of withoutDuplicateRects(rects)) {
        let shown: ViewportRect | null = rect;
        // Cheap test first: a rect wholly before the cut is painted whether or
        // not its line was truncated.
        if (
          lineBlock &&
          !glyphPaintedBeforeEllipsis(rect, lineBlock.cut) &&
          lineCarriesEllipsis(lineBlock, rect)
        ) {
          shown = trimRectToEllipsis(
            rect,
            glyphsOnLine(hit.node, hit.startOffset, hit.endOffset, rect),
            lineBlock.cut
          );
        }
        if (!shown) continue;
        const verdict = clipMatchRect(shown, clips);
        if (!verdict.counts) continue;
        counts = true;
        if (verdict.paint) paint.push(verdict.paint);
      }
      if (counts)
        measured.push({ record: { ...hit, scrollers }, paint: withoutDuplicateRects(paint) });
    }

    this.matches = measured.map(({ record }) => record);
    this.highlights = measured.map(({ paint }) => {
      const highlight = document.createElement('div');
      highlight.className = 'search-highlight-container';

      // Handle multi-line highlights
      paint.forEach((rect) => {
        const highlightRect = document.createElement('div');
        highlightRect.className = 'search-highlight';
        highlightRect.style.cssText = `
          position: absolute;
          pointer-events: none;
          top: ${rect.top - originTop}px;
          left: ${rect.left - originLeft}px;
          width: ${rect.right - rect.left}px;
          height: ${rect.bottom - rect.top}px;
        `;
        highlight.appendChild(highlightRect);
      });

      this.overlay.appendChild(highlight);
      return highlight;
    });

    // Only notify about count changes if the number of matches has actually changed
    if (this.highlights.length !== oldHighlightCount) {
      this.onMatchesChange?.(this.highlights.length);
    }

    // Restore current match if we have the same number of highlights
    if (currentIndex >= 0 && this.highlights.length === oldHighlightCount) {
      this.setCurrentMatch(currentIndex, false);
    }
    // Otherwise, if we have highlights but the count changed, start from the beginning
    else if (this.highlights.length > 0) {
      this.setCurrentMatch(0, false);
    }

    return this.highlights;
  }

  setCurrentMatch(index: number, shouldScroll = true) {
    if (!this.highlights.length) return;

    // Ensure index wraps around
    const wrappedIndex =
      ((index % this.highlights.length) + this.highlights.length) % this.highlights.length;

    // Store the current match index
    this.currentMatchIndex = wrappedIndex;

    // A match inside an inner scroller may have no mark at all until that
    // scroller shows it; scroll it there and re-measure before painting.
    if (shouldScroll && this.revealInInnerScrollers(this.matches[wrappedIndex])) {
      this.holdScrollUpdates();
      this.highlight(this.currentTerm, this.caseSensitive);
      if (!this.highlights.length) return;
    }

    // Remove current class from all highlights
    this.overlay.querySelectorAll('.search-highlight').forEach((el) => {
      el.classList.remove('current');
    });

    // Add current class to all parts of the highlight
    const currentHighlight = this.highlights[this.currentMatchIndex];
    const highlightElements = currentHighlight.querySelectorAll('.search-highlight');
    highlightElements.forEach((el) => {
      el.classList.add('current');
    });

    // Only scroll if explicitly requested
    if (shouldScroll && this.scrollContainer) {
      const firstHighlight = highlightElements[0] as HTMLElement;
      if (firstHighlight) {
        // Calculate the target scroll position
        const containerRect = this.scrollContainer.getBoundingClientRect();
        const highlightRect = firstHighlight.getBoundingClientRect();

        // Calculate the target position that would center the highlight
        const targetScrollTop =
          this.scrollContainer.scrollTop +
          (highlightRect.top - containerRect.top) -
          (containerRect.height - highlightRect.height) / 2;

        this.holdScrollUpdates();

        // Perform the scroll
        this.scrollContainer.scrollTop = targetScrollTop;
      }
    }
  }

  /** Ignore the scroll events our own scrolling is about to fire. */
  private holdScrollUpdates() {
    this.isScrollingProgrammatically = true;
    if (this.scrollEndTimeout) clearTimeout(this.scrollEndTimeout);
    this.scrollEndTimeout = setTimeout(() => {
      this.scrollEndTimeout = undefined;
      this.isScrollingProgrammatically = false;
    }, 100);
  }

  /**
   * Scroll each scroller between the match and the overlay host, innermost
   * first, until the match's first rect is inside it. Returns whether any of
   * them moved.
   */
  private revealInInnerScrollers(match: MatchRecord | undefined): boolean {
    if (!match?.scrollers.length || !match.node.isConnected) return false;
    const range = document.createRange();
    range.setStart(match.node, match.startOffset);
    range.setEnd(match.node, match.endOffset);

    let moved = false;
    for (const { element: scroller, x, y } of match.scrollers) {
      const rect = range.getClientRects()[0] ?? range.getBoundingClientRect();
      const box = paddingBoxOf(scroller);

      const before = { x: scroller.scrollLeft, y: scroller.scrollTop };
      if (x && (rect.left < box.left || rect.right > box.right)) {
        scroller.scrollLeft += (rect.left + rect.right) / 2 - (box.left + box.right) / 2;
      }
      if (y && (rect.top < box.top || rect.bottom > box.bottom)) {
        scroller.scrollTop += (rect.top + rect.bottom) / 2 - (box.top + box.bottom) / 2;
      }
      if (scroller.scrollLeft !== before.x || scroller.scrollTop !== before.y) moved = true;
    }
    return moved;
  }

  private updateHighlightPositions() {
    if (this.currentTerm) {
      const currentIndex = this.currentMatchIndex;
      const oldHighlights = this.highlights.length; // Store the current count
      this.highlight(this.currentTerm, this.caseSensitive);

      // If we still have the same number of highlights, restore the current index
      if (this.highlights.length === oldHighlights && currentIndex >= 0) {
        this.setCurrentMatch(currentIndex, false);
      }
    }
  }

  clearHighlights() {
    this.highlights.forEach((h) => h.remove());
    this.highlights = [];
    this.matches = [];
    this.currentTerm = '';
    this.currentMatchIndex = -1;
    while (this.overlay.firstChild) {
      this.overlay.removeChild(this.overlay.firstChild);
    }
  }

  destroy() {
    if (this.highlightTimeout) {
      clearTimeout(this.highlightTimeout);
    }
    if (this.scrollEndTimeout) {
      clearTimeout(this.scrollEndTimeout);
    }
    this.overlayHost.removeEventListener('scroll', this.handleScroll, { capture: true });
    this.resizeObserver.disconnect();
    this.mutationObserver.disconnect();
    this.overlay.remove();
  }
}
