import {
  memo,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { flushSync } from 'react-dom';
import { Button } from '../ui/button';
import { DICTIONARY_LABELS } from './copy';
import DictionaryRow, { type DictionaryRowActions } from './DictionaryRow';
import type { EntryGroup, SettledKind } from './rows';
import {
  DICTIONARY_ROW_PITCH_GUESS,
  FIRST_DRAW,
  linesInView,
  nextDrawRange,
  rowsLines,
  sliceRows,
  type DrawRange,
  type GroupBox,
} from './rowWindow';

export interface DictionaryListProps {
  groups: readonly EntryGroup[];
  /** Rows the person acted on while the pointer held the list. */
  settled: Readonly<Record<string, SettledKind>>;
  readOnly: boolean;
  deleteAllowed?: boolean;
  tierChangeAllowed: boolean;
  tierChangeReason?: string | null;
  /** One object whose callbacks keep their identity, so a drawn row re-renders only when it changes. */
  actions: DictionaryRowActions;
  /** Present when **Review all…** is offered on **Suggested**. */
  onReviewAll?: () => void;
  /** The hover hold (`hoverHold.ts`): the list says when it holds, and its pointer handlers. */
  holding: boolean;
  handlers: { onPointerEnter: () => void; onPointerLeave: () => void };
}

/** The element the list scrolls in: the page's `ScrollArea` viewport, else the nearest scroller. */
function scrollContainerOf(element: HTMLElement): HTMLElement | null {
  const viewport = element.closest<HTMLElement>('[data-radix-scroll-area-viewport]');
  if (viewport) return viewport;
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    const overflow = getComputedStyle(parent).overflowY;
    if (overflow === 'auto' || overflow === 'scroll') return parent;
  }
  return null;
}

interface Measured {
  boxes: GroupBox[];
  viewTop: number;
  viewBottom: number;
  /** A drawn row's height and the space below it; null when no row is drawn. */
  pitch: number | null;
}

/** The groups' boxes and the viewport, read from the DOM; null for a list with no layout. */
function measure(root: HTMLElement, container: HTMLElement): Measured | null {
  const height = container.clientHeight;
  if (height === 0 || root.getBoundingClientRect().height === 0) return null;
  const origin = container.getBoundingClientRect().top - container.scrollTop;
  const boxes: GroupBox[] = [];
  for (const section of root.querySelectorAll<HTMLElement>('section[data-rows-line]')) {
    const box = section.getBoundingClientRect();
    const list = section.querySelector<HTMLElement>('[data-dictionary-rows]');
    boxes.push({
      rowsLine: Number(section.dataset.rowsLine),
      rows: Number(section.dataset.rowCount),
      top: box.top - origin,
      rowsTop: (list ? list.getBoundingClientRect().top : box.bottom) - origin,
    });
  }
  const row = root.querySelector<HTMLElement>('[data-dictionary-row]');
  const gap = row?.parentElement ? parseFloat(getComputedStyle(row.parentElement).rowGap) || 0 : 0;
  const pitch = row ? row.getBoundingClientRect().height + gap : null;
  return {
    boxes,
    viewTop: container.scrollTop,
    viewBottom: container.scrollTop + height,
    pitch: pitch && pitch > 0 ? pitch : null,
  };
}

/**
 * The Dictionary page's groups and rows ("Groups" and "Rows" in
 * `docs/dictation/dictionary.md`), drawing only the rows near the viewport
 * (`rowWindow.ts`). Every group's header is drawn; a run of rows outside the
 * viewport and its overscan is one placeholder exactly their height, so the
 * scroll height and every drawn row's place do not depend on what is drawn.
 *
 * - The pitch is read off a drawn row, never assumed, because the Text size
 *   preference scales it.
 * - From a scroll, the new rows are committed before the frame paints
 *   (`flushSync`), so a scroll never shows the empty placeholder where a row is
 *   about to be. A resize catches up in the next frame.
 * - The row that last held focus stays drawn wherever the list is scrolled, so
 *   focus is never unmounted under the person.
 * - A list that cannot be measured (no layout, as in jsdom, or hidden) keeps
 *   what it drew: the first {@link FIRST_DRAW} lines.
 */
function DictionaryList({
  groups,
  settled,
  readOnly,
  deleteAllowed = true,
  tierChangeAllowed,
  tierChangeReason,
  actions,
  onReviewAll,
  holding,
  handlers,
}: DictionaryListProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLElement | null>(null);
  const [range, setRange] = useState<DrawRange>(FIRST_DRAW);
  const [pitch, setPitch] = useState(DICTIONARY_ROW_PITCH_GUESS);
  const pitchRef = useRef(pitch);
  const [focusedId, setFocusedId] = useState<string | null>(null);

  const sync = useCallback((beforePaint: boolean) => {
    const root = rootRef.current;
    if (!root) return;
    const container = (containerRef.current ??= scrollContainerOf(root));
    if (!container) return;
    const measured = measure(root, container);
    if (!measured) return;
    const nextPitch =
      measured.pitch !== null && Math.abs(measured.pitch - pitchRef.current) > 0.25
        ? measured.pitch
        : pitchRef.current;
    pitchRef.current = nextPitch;
    const inView = linesInView(measured.boxes, measured.viewTop, measured.viewBottom, nextPitch);
    const update = () => {
      setPitch(nextPitch);
      setRange((current) => nextDrawRange(current, inView));
    };
    if (beforePaint) flushSync(update);
    else update();
  }, []);

  // After every change to what the list holds (and to the pitch, which moves every group below
  // the first), read what is in view again, before the frame paints.
  useLayoutEffect(() => {
    sync(false);
  }, [groups, pitch, sync]);

  useEffect(() => {
    const root = rootRef.current;
    const container = root ? (containerRef.current ??= scrollContainerOf(root)) : null;
    if (!root || !container) return;
    const onScroll = () => sync(true);
    container.addEventListener('scroll', onScroll, { passive: true });
    // The viewport's height (a window resize) and the rows' own (the Text size preference). Not
    // committed inside the observer's callback: a pitch that changes there resizes the list it
    // observes, which the browser reports as a resize loop.
    const observer =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => sync(false));
    observer?.observe(container);
    observer?.observe(root);
    return () => {
      container.removeEventListener('scroll', onScroll);
      observer?.disconnect();
    };
  }, [sync]);

  const onFocus = useCallback((event: React.FocusEvent) => {
    // A menu portalled out of a row is still its React child, and its items are no row's.
    const row =
      event.target instanceof Element ? event.target.closest('[data-dictionary-row]') : null;
    const id = row?.getAttribute('data-dictionary-row');
    if (id) setFocusedId(id);
  }, []);

  const sliced = useMemo(() => {
    const lines = rowsLines(groups);
    return groups.map((group, index) => ({
      group,
      rowsLine: lines[index],
      lines: sliceRows(group.entries, lines[index], range, (entry) => entry.id === focusedId),
    }));
  }, [groups, range, focusedId]);

  return (
    <div
      ref={rootRef}
      className="flex flex-col gap-6"
      data-testid="dictionary-groups"
      data-holding={holding ? 'true' : undefined}
      onFocus={onFocus}
      {...handlers}
    >
      {sliced.map(({ group, rowsLine, lines }) =>
        group.entries.length === 0 && group.key === 'matches' ? null : (
          <TermGroup
            key={group.key}
            groupKey={group.key}
            title={group.title}
            count={group.entries.length}
            rowsLine={rowsLine}
            empty={group.key === 'user' ? DICTIONARY_LABELS.userGroupEmpty : undefined}
            action={
              group.key === 'suggested' && onReviewAll ? (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={onReviewAll}
                  data-testid="dictionary-review-all"
                >
                  {DICTIONARY_LABELS.reviewAll}
                </Button>
              ) : null
            }
          >
            {lines.map((line) =>
              line.kind === 'gap' ? (
                <div
                  key={line.key}
                  className="br-dictionary-gap"
                  aria-hidden
                  data-dictionary-gap={line.rows}
                  style={{ height: line.rows * pitch }}
                />
              ) : (
                <DictionaryRow
                  key={line.entry.id}
                  entry={line.entry}
                  settled={settled[line.entry.id]}
                  readOnly={readOnly}
                  deleteAllowed={deleteAllowed}
                  tierChangeAllowed={tierChangeAllowed}
                  tierChangeReason={tierChangeReason}
                  {...actions}
                />
              )
            )}
          </TermGroup>
        )
      )}
    </div>
  );
}

export default memo(DictionaryList);

/**
 * One group: a caps label with its count, then flat rows told apart by air and the hover wash.
 * A group shown with no rows (**Added by you**, which shows whenever the dictionary holds any
 * row) says so in one muted line rather than a heading over nothing.
 */
function TermGroup({
  groupKey,
  title,
  count,
  rowsLine,
  empty,
  action,
  children,
}: {
  groupKey: string;
  title: string;
  count: number;
  rowsLine: number;
  empty?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  const headingId = useId();
  return (
    <section
      aria-labelledby={headingId}
      className="min-w-0"
      data-dictionary-section={groupKey}
      data-rows-line={rowsLine}
      data-row-count={count}
    >
      <div className="mb-1 flex items-center gap-3 px-3">
        <h2 id={headingId} className="min-w-0 flex-1 truncate text-caps text-text-muted">
          {title} <span className="tabular-nums">{count}</span>
        </h2>
        {action}
      </div>
      {count === 0 && empty ? (
        <p className="px-3 py-1 text-supporting text-text-muted" data-dictionary-group-empty="">
          {empty}
        </p>
      ) : (
        <div className="biorouter-list-shell br-rows-flat" data-dictionary-rows="">
          {children}
        </div>
      )}
    </section>
  );
}
