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
import type { DictationHistoryEntryView } from '../../../api';
import {
  DICTIONARY_OVERSCAN,
  FIRST_DRAW,
  linesInView,
  nextDrawRange,
  rowsLines,
  sliceRows,
  type DrawRange,
  type GroupBox,
  type TallRow,
} from '../rowWindow';
import HistoryRow, { type HistoryRowActions } from './HistoryRow';
import type { HistoryGroup, HistorySettledKind } from './historyRows';

/**
 * A collapsed row's height and the space below it before one has been measured: two lines of
 * body text, one muted line, 10px of padding above and below, and the row's 1px bottom edge.
 * The real pitch is read off a drawn collapsed row, because the Text size preference scales it.
 */
export const HISTORY_ROW_PITCH_GUESS = 77;

/** The next page is asked for when the drawn lines come this close to the last line held. */
export const HISTORY_LOAD_AHEAD_LINES = DICTIONARY_OVERSCAN * 2;

export interface HistoryListProps {
  groups: readonly HistoryGroup[];
  /** Rows the person acted on while the pointer held the list. */
  settled: Readonly<Record<string, HistorySettledKind>>;
  /** Rows expanded in place; each is always drawn. */
  expanded: ReadonlySet<string>;
  writeAllowed: boolean;
  tierChangeAllowed: boolean;
  tierChangeReason?: string | null;
  /** One object whose callbacks keep their identity, so a drawn row re-renders only when it changes. */
  actions: HistoryRowActions;
  /** The time every row's muted line is read against. */
  now: number;
  /** Present while the daemon holds more matches than the list: called as the list nears its end. */
  onNearEnd?: () => void;
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
  /** A drawn collapsed row's height and the space below it; null when none is drawn. */
  pitch: number | null;
  /** Drawn rows taller than a collapsed one (expanded), by id, with their height and gap. */
  tall: Array<{ id: string; span: number }>;
}

/** The groups' boxes, the viewport and the rows' heights, read from the DOM; null with no layout. */
function measure(root: HTMLElement, container: HTMLElement): Measured | null {
  const height = container.clientHeight;
  if (height === 0 || root.getBoundingClientRect().height === 0) return null;
  const origin = container.getBoundingClientRect().top - container.scrollTop;
  const boxes: GroupBox[] = [];
  for (const section of root.querySelectorAll<HTMLElement>('section[data-rows-line]')) {
    const box = section.getBoundingClientRect();
    const list = section.querySelector<HTMLElement>('[data-history-rows]');
    boxes.push({
      rowsLine: Number(section.dataset.rowsLine),
      rows: Number(section.dataset.rowCount),
      top: box.top - origin,
      rowsTop: (list ? list.getBoundingClientRect().top : box.bottom) - origin,
    });
  }
  const anyRow = root.querySelector<HTMLElement>('[data-history-row]');
  const gap = anyRow?.parentElement
    ? parseFloat(getComputedStyle(anyRow.parentElement).rowGap) || 0
    : 0;
  const collapsed = root.querySelector<HTMLElement>('[data-history-row]:not([data-expanded])');
  const pitch = collapsed ? collapsed.getBoundingClientRect().height + gap : null;
  const tall: Measured['tall'] = [];
  for (const row of root.querySelectorAll<HTMLElement>('[data-history-row][data-expanded]')) {
    tall.push({ id: row.dataset.historyRow ?? '', span: row.getBoundingClientRect().height + gap });
  }
  return {
    boxes,
    viewTop: container.scrollTop,
    viewBottom: container.scrollTop + height,
    pitch: pitch && pitch > 0 ? pitch : null,
    tall,
  };
}

/**
 * The **History** tab's date groups and rows, drawing only the rows near the viewport
 * (`rowWindow.ts`), as the Dictionary's own list does: every group's heading is drawn, and a run
 * of rows outside the viewport and its overscan is one placeholder exactly their height.
 *
 * - The pitch is read off a drawn collapsed row, never assumed.
 * - An expanded row is always drawn, never folded into a placeholder, and its extra height is
 *   measured and handed to the line arithmetic, so every line below it stays exact.
 * - The row that last held focus stays drawn wherever the list is scrolled.
 * - From a scroll, the new rows are committed before the frame paints (`flushSync`).
 * - While the daemon holds more matches, the next page is asked for as the drawn lines come
 *   within {@link HISTORY_LOAD_AHEAD_LINES} of the last line held.
 * - A list that cannot be measured (no layout, as in jsdom) keeps its first draw.
 */
function HistoryList({
  groups,
  settled,
  expanded,
  writeAllowed,
  tierChangeAllowed,
  tierChangeReason,
  actions,
  now,
  onNearEnd,
  holding,
  handlers,
}: HistoryListProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLElement | null>(null);
  const [range, setRange] = useState<DrawRange>(FIRST_DRAW);
  const [pitch, setPitch] = useState(HISTORY_ROW_PITCH_GUESS);
  const pitchRef = useRef(pitch);
  const [focusedId, setFocusedId] = useState<string | null>(null);

  const lines = useMemo(() => rowsLines(groups), [groups]);
  // Each row's line, for the expanded rows the arithmetic needs to know about.
  const lineOf = useMemo(() => {
    const map = new Map<string, number>();
    groups.forEach((group, index) => {
      group.entries.forEach((entry, offset) => map.set(entry.id, lines[index] + offset));
    });
    return map;
  }, [groups, lines]);
  const lineOfRef = useRef(lineOf);
  useLayoutEffect(() => {
    lineOfRef.current = lineOf;
  }, [lineOf]);
  const totalLines = useMemo(
    () => groups.reduce((sum, group) => sum + 1 + group.entries.length, 0),
    [groups]
  );

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
    const tall: TallRow[] = [];
    for (const row of measured.tall) {
      const line = lineOfRef.current.get(row.id);
      const extra = row.span - nextPitch;
      if (line !== undefined && extra > 0.25) tall.push({ line, extra });
    }
    const inView = linesInView(
      measured.boxes,
      measured.viewTop,
      measured.viewBottom,
      nextPitch,
      tall
    );
    const update = () => {
      setPitch(nextPitch);
      setRange((current) => nextDrawRange(current, inView));
    };
    if (beforePaint) flushSync(update);
    else update();
  }, []);

  // After every change to what the list holds, which rows are expanded, and the pitch, read what
  // is in view again, before the frame paints.
  useLayoutEffect(() => {
    sync(false);
  }, [groups, expanded, pitch, sync]);

  useEffect(() => {
    const root = rootRef.current;
    const container = root ? (containerRef.current ??= scrollContainerOf(root)) : null;
    if (!root || !container) return;
    const onScroll = () => sync(true);
    container.addEventListener('scroll', onScroll, { passive: true });
    const observer =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => sync(false));
    observer?.observe(container);
    observer?.observe(root);
    return () => {
      container.removeEventListener('scroll', onScroll);
      observer?.disconnect();
    };
  }, [sync]);

  // The next page, once the drawn lines come near the last line held.
  useEffect(() => {
    if (!onNearEnd || totalLines === 0) return;
    if (range.to >= totalLines - HISTORY_LOAD_AHEAD_LINES) onNearEnd();
  }, [range, totalLines, onNearEnd]);

  const onFocus = useCallback((event: React.FocusEvent) => {
    // A menu portalled out of a row is still its React child, and its items are no row's.
    const row = event.target instanceof Element ? event.target.closest('[data-history-row]') : null;
    const id = row?.getAttribute('data-history-row');
    if (id) setFocusedId(id);
  }, []);

  const sliced = useMemo(
    () =>
      groups.map((group, index) => ({
        group,
        rowsLine: lines[index],
        drawn: sliceRows(
          group.entries,
          lines[index],
          range,
          (entry: DictationHistoryEntryView) => entry.id === focusedId || expanded.has(entry.id)
        ),
      })),
    [groups, lines, range, focusedId, expanded]
  );

  return (
    <div
      ref={rootRef}
      className="flex flex-col gap-6"
      data-testid="history-groups"
      data-holding={holding ? 'true' : undefined}
      onFocus={onFocus}
      {...handlers}
    >
      {sliced.map(({ group, rowsLine, drawn }) => (
        <HistoryGroupSection
          key={group.key}
          groupKey={group.key}
          title={group.title}
          count={group.entries.length}
          rowsLine={rowsLine}
        >
          {drawn.map((line) =>
            line.kind === 'gap' ? (
              <div
                key={line.key}
                className="br-dictionary-gap"
                aria-hidden
                data-history-gap={line.rows}
                style={{ height: line.rows * pitch }}
              />
            ) : (
              <HistoryRow
                key={line.entry.id}
                entry={line.entry}
                expanded={expanded.has(line.entry.id)}
                settled={settled[line.entry.id]}
                writeAllowed={writeAllowed}
                tierChangeAllowed={tierChangeAllowed}
                tierChangeReason={tierChangeReason}
                now={now}
                {...actions}
              />
            )
          )}
        </HistoryGroupSection>
      ))}
    </div>
  );
}

export default memo(HistoryList);

/** One date bucket: its heading in the chat lists' caps label, then flat rows told apart by air. */
function HistoryGroupSection({
  groupKey,
  title,
  count,
  rowsLine,
  children,
}: {
  groupKey: string;
  title: string;
  count: number;
  rowsLine: number;
  children: React.ReactNode;
}) {
  const headingId = useId();
  return (
    <section
      aria-labelledby={headingId}
      className="min-w-0"
      data-history-section={groupKey}
      data-rows-line={rowsLine}
      data-row-count={count}
    >
      <div className="mb-1 flex items-center gap-3 px-3">
        <h2 id={headingId} className="min-w-0 flex-1 truncate text-caps text-text-muted">
          {title}
        </h2>
      </div>
      <div className="biorouter-list-shell br-rows-flat" data-history-rows="">
        {children}
      </div>
    </section>
  );
}
