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
 * - **A scroll container does not.** The transcript's own scroller is where the
 *   overlay lives, so a match below the fold counts and is painted in place. An
 *   inner scroller (a code block with `overflow-x: auto`) is the same promise one
 *   level down: its hidden text counts, is painted only inside the block, and
 *   navigating to it scrolls the block until it shows.
 *
 * So the counter agrees with the marks: every counted match is either painted
 * or one scroll away, and nothing is painted outside the box that shows it.
 * Every approximation below errs toward showing a match rather than hiding one.
 */

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

interface ElementLayout {
  position: string;
  rendered: boolean;
  clip: ClipBox | null;
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
      acceptNode: (node) => {
        // Skip search UI elements
        const parent = node.parentElement;
        if (parent?.closest('.search-bar, .search-results')) {
          return NodeFilter.FILTER_REJECT;
        }
        return NodeFilter.FILTER_ACCEPT;
      },
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

    const measured: { record: MatchRecord; paint: ViewportRect[] }[] = [];
    for (const hit of hits) {
      const parent = hit.node.parentElement;
      if (!parent || !layoutOf(parent, layouts).rendered) continue;

      range.setStart(hit.node, hit.startOffset);
      range.setEnd(hit.node, hit.endOffset);
      const { clips, scrollers } = clipChainOf(parent, host, layouts);

      let counts = false;
      const paint: ViewportRect[] = [];
      for (const rect of Array.from(range.getClientRects())) {
        const verdict = clipMatchRect(rect, clips);
        if (!verdict.counts) continue;
        counts = true;
        if (verdict.paint) paint.push(verdict.paint);
      }
      if (counts) measured.push({ record: { ...hit, scrollers }, paint });
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
