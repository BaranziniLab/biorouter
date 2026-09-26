import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { ActivityWindow, DailyActivity } from '../../api';

/**
 * A GitHub-style contribution graph for Biorouter usage.
 *
 * Shading is computed server-side (see `build_activity_window`) from the
 * quartiles of the active days in the window, so a single 1.8M-token day cannot
 * flatten every ordinary day into the faintest shade. Absolute numbers live in
 * the tooltip, where they belong.
 */

const DAY_MS = 86_400_000;
const LOADING_WEEKS = 22;
const LOADING_CELLS = LOADING_WEEKS * 7;

/**
 * Cell sizing. The heatmap must fit whatever box the Home view can spare —
 * down to the smallest window — so instead of fixed CSS breakpoints the grid
 * picks the largest cell size whose full footprint (day-label gutter + one
 * column per week, 7 rows plus the chrome around the grid) fits the measured
 * box. Gap, gutter and label font sizes step down with the cells so nothing
 * overlaps at any size.
 */
const MAX_CELL = 24;
const MIN_CELL = 8;

type HeatMetrics = { cell: number; gap: number; labels: number; width: number };

function gapFor(cell: number): number {
  return cell >= 20 ? 6 : cell >= 15 ? 4 : cell >= 11 ? 3 : 2;
}

function gutterFor(cell: number): number {
  return cell >= 15 ? 34 : cell >= 11 ? 28 : 24;
}

/** Fixed vertical chrome around the grid: header + month ruler + legend, with
 * their margins. The incomplete-token note adds two short lines. Estimates —
 * a few px of error only moves the fit by at most one cell step. */
function chromeFor(tokensComplete: boolean): number {
  return tokensComplete ? 104 : 150;
}

/** Everything a cell size implies, including the block's total footprint:
 * day-label gutter, the gap after it, then one column per week. */
function metricsFor(weeks: number, cell: number): HeatMetrics {
  const gap = gapFor(cell);
  return {
    cell,
    gap,
    labels: gutterFor(cell),
    width: gutterFor(cell) + gap + weeks * cell + (weeks - 1) * gap,
  };
}

function fitMetrics(weeks: number, width: number, height: number, chrome: number): HeatMetrics {
  for (let cell = MAX_CELL; cell > MIN_CELL; cell--) {
    const m = metricsFor(weeks, cell);
    const gridHeight = 7 * cell + 6 * m.gap;
    if (m.width <= width && gridHeight <= height - chrome) {
      return m;
    }
  }
  return metricsFor(weeks, MIN_CELL);
}

/** CSS vars driving the grid, derived from the fitted metrics. Fonts shrink
 * with the cells so a label row can never be taller than its cell row. */
function heatStyle(m: HeatMetrics): React.CSSProperties {
  return {
    /* Bind the WHOLE block — streak header, month ruler, grid and legend — to
       the grid's own footprint, so they can never come apart. The cells are
       integer squares picked off a ladder, so the grid is always a little
       narrower than the box it was fitted into (measured: 11px at a full-size
       window, up to 49px where the gap steps down from 6 to 4). The chrome
       rows are `justify-between`, so without this they stay pinned to the
       box's right edge while the grid pulls left — "Longest streak" and
       "…on your busiest day" float away from the columns they annotate. It is
       far worse when HEIGHT is the binding constraint: a short window shrinks
       the cells without touching the box's width, and the measured gap reached
       339px. No window minimum can fix that case; only this can.

       `min(…, 100%)` because below MIN_CELL the footprint stops shrinking, and
       a block wider than its column would be clipped by the Hub's
       `overflow-x-hidden` rather than merely looking narrow. */
    width: `min(${m.width}px, 100%)`,
    '--heat-cell': `${m.cell}px`,
    '--heat-gap': `${m.gap}px`,
    '--heat-labels': `${m.labels}px`,
    '--heat-day-font': `${m.cell >= 12 ? 10 : m.cell >= 9 ? 9 : 8}px`,
    '--heat-month-font': `${m.cell >= 12 ? 11 : 10}px`,
    // A fixed 5px radius turns small cells into circles; keep them squares.
    '--heat-radius': `${m.cell >= 15 ? 5 : m.cell >= 11 ? 4 : 2}px`,
  } as React.CSSProperties;
}

/**
 * Measure the box the heatmap's parent gives it. Width drives the fit always;
 * height only when the parent has a definite height (the Home view's flex
 * column). jsdom and the first pre-layout paint report 0 — treat that as
 * "unknown, keep the CSS defaults", never as "no room".
 */
function useFittedMetrics(
  rootRef: React.RefObject<HTMLDivElement | null>,
  weeks: number,
  tokensComplete: boolean
): React.CSSProperties | undefined {
  const [box, setBox] = useState<{ w: number; h: number } | null>(null);

  useLayoutEffect(() => {
    const host = rootRef.current?.parentElement;
    if (!host) return;
    const measure = () => {
      const w = host.clientWidth;
      const h = host.clientHeight;
      if (w <= 0) return;
      setBox((prev) => (prev && prev.w === w && prev.h === h ? prev : { w, h }));
    };
    measure();
    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', measure);
      return () => window.removeEventListener('resize', measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => observer.disconnect();
  }, [rootRef]);

  return useMemo(() => {
    if (!box) return undefined;
    const height = box.h > 0 ? box.h : Number.POSITIVE_INFINITY;
    return heatStyle(fitMetrics(weeks, box.w, height, chromeFor(tokensComplete)));
  }, [box, weeks, tokensComplete]);
}

/** Local calendar day, `YYYY-MM-DD`, matching the server's `date('now','localtime')`. */
function isoDay(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function parseIsoDay(s: string): Date {
  const [y, m, d] = s.split('-').map(Number);
  return new Date(y, m - 1, d);
}

type Cell = {
  key: string;
  date: Date;
  day: DailyActivity | null;
  inStreak: boolean;
};

/**
 * Lay the window out Sunday-first so the columns line up with the day labels
 * and the month ruler. The first column is padded back to Sunday, but the last
 * column stops at the window end so future days do not look like idle days.
 */
function buildGrid(window: ActivityWindow): { cells: Cell[]; weeks: number } {
  const byDate = new Map(window.days.map((d) => [d.date, d]));

  const end = parseIsoDay(window.end);
  const start = parseIsoDay(window.start);
  // pad back to Sunday
  start.setDate(start.getDate() - start.getDay());

  const total = Math.round((end.getTime() - start.getTime()) / DAY_MS) + 1;

  // The current streak is the run of active days ending at `window.end`. Mark
  // those cells so the user can see the thing the header is counting.
  const streakDays = new Set<string>();
  if (window.currentStreak > 0) {
    const cursor = parseIsoDay(window.end);
    if (!byDate.has(isoDay(cursor))) cursor.setDate(cursor.getDate() - 1);
    for (let i = 0; i < window.currentStreak; i++) {
      streakDays.add(isoDay(cursor));
      cursor.setDate(cursor.getDate() - 1);
    }
  }

  const cells: Cell[] = [];
  for (let i = 0; i < total; i++) {
    const date = new Date(start.getTime() + i * DAY_MS);
    const key = isoDay(date);
    cells.push({ key, date, day: byDate.get(key) ?? null, inStreak: streakDays.has(key) });
  }
  return { cells, weeks: Math.ceil(total / 7) };
}

/**
 * Where an arrow key takes focus from cell `from`, or null when it stays put.
 *
 * The cells are stored column-major — a column is a week, Sunday first — so a
 * step of 1 is a day and a step of 7 is a week:
 *   · ↑ / ↓ move one DAY, and run on across the week boundary (Saturday → the
 *     next Sunday), because the grid is a calendar and a day's neighbour in time
 *     is the next day, not nothing.
 *   · ← / → move one WEEK along the same weekday row, and stop at the edge.
 *   · Home / End go to the first / last cell of the ROW (the grid pattern);
 *     Ctrl or ⌘ with them go to the first day and to today.
 */
function nextHeatCell(
  key: string,
  from: number,
  count: number,
  toGridEdge: boolean
): number | null {
  const last = count - 1;
  const day = from % 7;
  let to: number;
  switch (key) {
    case 'ArrowDown':
      to = from + 1;
      break;
    case 'ArrowUp':
      to = from - 1;
      break;
    case 'ArrowRight':
      to = from + 7;
      break;
    case 'ArrowLeft':
      to = from - 7;
      break;
    case 'Home':
      to = toGridEdge ? 0 : day;
      break;
    case 'End':
      to = toGridEdge ? last : day + 7 * Math.floor((last - day) / 7);
      break;
    default:
      return null;
  }
  return to >= 0 && to <= last ? to : null;
}

const LEVEL_CLASS: Record<number, string> = {
  0: 'bg-heat-0',
  1: 'bg-heat-1',
  2: 'bg-heat-2',
  3: 'bg-heat-3',
  4: 'bg-heat-4',
};

const compact = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 2 });
const full = new Intl.NumberFormat('en');

/** The subset of a cell's box the tooltip needs, as plain numbers. */
type Anchor = { left: number; top: number; bottom: number; width: number };

function Tooltip({ cell, anchor }: { cell: Cell; anchor: Anchor }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  // Measure after paint: a tooltip positioned from a guessed width jumps.
  // The tooltip is portalled to <body> (below) so `fixed` is always
  // viewport-relative — a transformed ancestor would otherwise make `fixed`
  // resolve against that ancestor and let the tooltip escape the window. Clamp
  // both axes to the visual viewport (clientWidth/Height exclude the scrollbar).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    const left = Math.max(8, Math.min(anchor.left + anchor.width / 2 - width / 2, vw - width - 8));
    const above = anchor.top - height - 10;
    const below = anchor.bottom + 10;
    const top = above >= 8 ? Math.min(above, vh - height - 8) : Math.min(below, vh - height - 8);
    setPos({ left, top: Math.max(8, top) });
  }, [anchor]);

  const d = cell.day;
  const label = cell.date.toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });

  return (
    <div
      ref={ref}
      role="tooltip"
      // `--z-dropdown`, not `--z-toast`. This tooltip was the token's only real
      // consumer, and it borrowed the top rung of the ladder for a hover hint —
      // which put it above any actual toast once the toast layer started
      // honouring its own tier. It is portalled to `document.body`, so it only
      // has to clear the sticky page chrome (`--z-sticky`), and a transient
      // popover is exactly what `--z-dropdown` names.
      className="pointer-events-none fixed z-[var(--z-dropdown)] w-max min-w-[228px] max-w-[calc(100vw-16px)] rounded-surface border border-border-subtle bg-background-default p-3 shadow-popover"
      style={{
        left: pos?.left ?? -9999,
        top: pos?.top ?? -9999,
        visibility: pos ? 'visible' : 'hidden',
      }}
    >
      <p className="mb-2 text-label text-text-default">{label}</p>
      {cell.inStreak && (
        <p className="-mt-1 mb-2 text-supporting text-text-muted">Part of your current streak</p>
      )}
      {d ? (
        <>
          <Row label="Chats started" value={full.format(d.sessions)} />
          <Row label="Tokens processed" value={tokenDisplay(d)} />
          <Row label="Messages" value={full.format(d.messages)} />
          <p className="mt-2 border-t border-border-subtle pt-2 text-supporting leading-snug text-text-subtle">
            {d.tokensComplete
              ? 'Tokens are attributed to the turn that spent them.'
              : d.tokens > 0
                ? 'Conservative estimate; some token events that day could not be counted.'
                : 'Complete token accounting is unavailable for this day, so an exact total cannot be shown.'}
          </p>
        </>
      ) : (
        <Row label="No activity" value="0" />
      )}
    </div>
  );
}

function tokenDisplay(day: DailyActivity): string {
  if (day.tokensComplete) return full.format(day.tokens);
  if (day.tokens === 0) return 'Unavailable';
  return full.format(day.tokens);
}

function tokenAria(day: DailyActivity): string {
  if (day.tokensComplete) return `${day.tokens} tokens`;
  if (day.tokens === 0) return 'token total unavailable';
  return `${day.tokens} estimated tokens`;
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-5 text-secondary leading-[1.9] text-text-muted">
      <span>{label}</span>
      <b className="font-mono font-medium tabular-nums text-text-default">{value}</b>
    </div>
  );
}

export function UsageHeatmapLoading() {
  return (
    <div className="biorouter-heatmap" role="status" aria-label="Loading usage activity">
      <div aria-hidden="true">
        <div className="mb-4 flex items-center justify-between gap-4">
          <span className="biorouter-heatmap-loading-line h-6 w-24 rounded-element bg-heat-0" />
          <span className="biorouter-heatmap-loading-line h-2.5 w-28 rounded bg-heat-0" />
        </div>

        <div className="grid grid-cols-[var(--heat-labels)_1fr] gap-[var(--heat-gap)]">
          <span />
          <div className="flex justify-between pr-8">
            {[0, 1, 2, 3].map((month) => (
              <span
                key={month}
                className="biorouter-heatmap-loading-line h-2.5 w-7 rounded bg-heat-0"
              />
            ))}
          </div>
        </div>

        <div className="mt-1.5 grid grid-cols-[var(--heat-labels)_1fr] gap-[var(--heat-gap)]">
          <div className="biorouter-heatmap-days grid grid-rows-7 gap-[var(--heat-gap)] leading-none text-text-muted/60">
            {['Sun', '', 'Tue', '', 'Thu', '', 'Sat'].map((label, index) => (
              <span key={index} className="flex h-[var(--heat-cell)] items-center">
                {label}
              </span>
            ))}
          </div>
          <div
            className="grid grid-flow-col justify-start gap-[var(--heat-gap)]"
            style={{
              gridTemplateRows: 'repeat(7, var(--heat-cell))',
              gridAutoColumns: 'var(--heat-cell)',
            }}
          >
            {Array.from({ length: LOADING_CELLS }, (_, index) => {
              const column = Math.floor(index / 7);
              const row = index % 7;
              return (
                <i
                  key={index}
                  data-testid="heatmap-loading-cell"
                  className="biorouter-heatmap-loading-cell block h-[var(--heat-cell)] w-[var(--heat-cell)] rounded-[var(--heat-radius,5px)] bg-heat-0"
                  style={{ animationDelay: `${-((column * 70 + row * 25) % 1400)}ms` }}
                />
              );
            })}
          </div>
        </div>

        <div className="mt-4 flex items-center justify-between text-supporting text-text-muted/70">
          <div className="flex items-center gap-1">
            <span className="mr-1">Less</span>
            {[0, 1, 2, 3, 4].map((level) => (
              <i key={level} className={`block h-3 w-3 rounded-inner ${LEVEL_CLASS[level]}`} />
            ))}
            <span className="ml-1">More</span>
          </div>
          <span>Loading activity</span>
        </div>
      </div>
    </div>
  );
}

export function UsageHeatmap({ window: activity }: { window: ActivityWindow }) {
  const { cells, weeks } = useMemo(() => buildGrid(activity), [activity]);
  const [hovered, setHovered] = useState<{ cell: Cell; rect: Anchor } | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const fittedStyle = useFittedMetrics(rootRef, weeks, activity.tokensComplete);

  /*
   * ONE TAB STOP, NOT ONE PER DAY (triage T-20). Every day used to be its own
   * `<button>`, so Tab crossed Home one DAY at a time — some 160 presses
   * through a chart to get past it. The grid is now a single stop with a roving
   * `tabIndex`: the entry cell is today (the last day), or the day the user
   * last moved to, and the arrow keys walk the calendar from there
   * (`nextHeatCell`). The active day is held by its DATE, not its index, so a
   * refreshed window that shifts the columns keeps the same day focusable.
   */
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const activeIndex = useMemo(() => {
    const index = activeKey === null ? -1 : cells.findIndex((cell) => cell.key === activeKey);
    return index >= 0 ? index : cells.length - 1;
  }, [cells, activeKey]);
  const cellRefs = useRef<(HTMLDivElement | null)[]>([]);

  // Row-major for the DOM, because an ARIA grid is rows of cells: row `d` holds
  // weekday `d` of every week. The flex rows reproduce the old column-flow
  // geometry exactly — same cell, same gap, and the unfinished last week still
  // simply ends early in the rows after today.
  const rows = useMemo(
    () =>
      Array.from({ length: 7 }, (_, day) =>
        cells.flatMap((cell, index) => (index % 7 === day ? [{ cell, index }] : []))
      ),
    [cells]
  );

  const handleGridKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      // The tooltip is content shown on focus, so it must be dismissible
      // without moving focus (WCAG 1.4.13). Only claim the key if it did so.
      if (hovered) {
        event.preventDefault();
        event.stopPropagation();
        setHovered(null);
      }
      return;
    }
    const from = Number((event.target as HTMLElement).dataset.index ?? activeIndex);
    const to = nextHeatCell(event.key, from, cells.length, event.ctrlKey || event.metaKey);
    if (to === null) {
      // An arrow at the edge is still ours: letting it through would scroll the
      // Home view under a keyboard user who only meant to stop at the last day.
      if (event.key.startsWith('Arrow')) event.preventDefault();
      return;
    }
    event.preventDefault();
    setActiveKey(cells[to].key);
    cellRefs.current[to]?.focus();
  };

  // One label per month, placed on the first column whose Sunday falls in it.
  const months = useMemo(() => {
    const out: (string | null)[] = [];
    let last = '';
    for (let w = 0; w < weeks; w++) {
      const d = cells[w * 7].date;
      const key = `${d.getFullYear()}-${d.getMonth()}`;
      if (key !== last && d.getDate() <= 7) {
        last = key;
        out.push(d.toLocaleDateString('en-US', { month: 'short' }));
      } else {
        out.push(null);
      }
    }
    return out;
  }, [cells, weeks]);

  const show = (cell: Cell) => (e: React.SyntheticEvent<HTMLElement>) => {
    const { left, top, bottom, width } = e.currentTarget.getBoundingClientRect();
    setHovered({ cell, rect: { left, top, bottom, width } });
  };
  const hide = () => setHovered(null);

  return (
    <div ref={rootRef} className="biorouter-heatmap" style={fittedStyle}>
      <div
        className={`${activity.tokensComplete ? 'mb-4' : 'mb-2'} flex flex-wrap items-baseline justify-between gap-x-4 gap-y-0.5`}
      >
        <h2 className="text-subheading text-text-default">
          {activity.currentStreak === 1 ? '1 day streak' : `${activity.currentStreak} day streak`}
        </h2>
        <span className="text-caps text-text-muted">
          Longest streak · {activity.longestStreak} {activity.longestStreak === 1 ? 'day' : 'days'}
        </span>
      </div>
      {!activity.tokensComplete && (
        <p className="mb-4 text-supporting text-text-subtle" role="status">
          Some days have incomplete token history, so their totals are conservative estimates.
          Unavailable means no trustworthy total was recorded.
        </p>
      )}

      <div className="grid grid-cols-[var(--heat-labels)_1fr] gap-[var(--heat-gap)]">
        <span aria-hidden="true" />
        <div className="biorouter-heatmap-months flex gap-[var(--heat-gap)] text-text-muted">
          {months.map((m, i) => (
            <span
              key={i}
              className="w-[var(--heat-cell)] flex-none overflow-visible whitespace-nowrap"
            >
              {m}
            </span>
          ))}
        </div>
      </div>

      <div className="mt-1.5 grid grid-cols-[var(--heat-labels)_1fr] gap-[var(--heat-gap)]">
        <div className="biorouter-heatmap-days grid grid-rows-7 gap-[var(--heat-gap)] leading-none text-text-muted">
          {['Sun', '', 'Tue', '', 'Thu', '', 'Sat'].map((d, i) => (
            <span key={i} className="flex h-[var(--heat-cell)] items-center">
              {d}
            </span>
          ))}
        </div>

        <div
          className="flex flex-col gap-[var(--heat-gap)]"
          role="grid"
          aria-label="Daily usage heatmap"
          onKeyDown={handleGridKeyDown}
        >
          {rows.map((row, day) => (
            <div key={day} role="row" className="flex h-[var(--heat-cell)] gap-[var(--heat-gap)]">
              {row.map(({ cell, index }) => (
                <div
                  key={cell.key}
                  ref={(element) => {
                    cellRefs.current[index] = element;
                  }}
                  role="gridcell"
                  data-index={index}
                  tabIndex={index === activeIndex ? 0 : -1}
                  // The tooltip is the only way to read the numbers, so it must open
                  // on keyboard focus, not hover alone.
                  onMouseEnter={show(cell)}
                  onFocus={(event) => {
                    setActiveKey(cell.key);
                    show(cell)(event);
                  }}
                  onMouseLeave={hide}
                  onBlur={hide}
                  aria-label={
                    cell.day
                      ? `${cell.key}: ${cell.day.sessions} ${cell.day.sessions === 1 ? 'chat' : 'chats'}, ${tokenAria(cell.day)}${cell.inStreak ? ', part of current streak' : ''}`
                      : `${cell.key}: no activity`
                  }
                  className={[
                    'relative block h-[var(--heat-cell)] w-[var(--heat-cell)] flex-none rounded-[var(--heat-radius,5px)]',
                    // hover:z-10 lifts the grown cell above its neighbors so scaling
                    // up doesn't get clipped by later-painted cells.
                    'transition-transform duration-[var(--motion-fast)] hover:z-10 hover:scale-110',
                    // A cell's fill IS its data, so focus cannot be shown by the
                    // app-wide focus fill; it is a ring outside the cell instead.
                    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus',
                    LEVEL_CLASS[cell.day?.level ?? 0],
                    cell.inStreak ? 'shadow-[inset_0_0_0_2px_var(--text-default)]' : '',
                  ].join(' ')}
                />
              ))}
            </div>
          ))}
        </div>
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-supporting text-text-muted">
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-1">
            <span className="mr-1">Less</span>
            {[0, 1, 2, 3, 4].map((l) => (
              <i key={l} className={`block h-3 w-3 rounded-inner ${LEVEL_CLASS[l]}`} />
            ))}
            <span className="ml-1">More</span>
          </div>
          {activity.currentStreak > 0 && (
            <div className="flex items-center gap-1.5">
              <i
                aria-hidden="true"
                className="block h-3 w-3 rounded-inner bg-heat-0 shadow-[inset_0_0_0_2px_var(--text-default)]"
              />
              <span>Current streak</span>
            </div>
          )}
        </div>
        <span className="tabular-nums">
          {activity.tokensComplete
            ? `${compact.format(activity.maxTokens)} tokens on your busiest day`
            : activity.maxTokens > 0
              ? `Highest recorded estimate · ${compact.format(activity.maxTokens)} tokens`
              : 'Token totals unavailable for older activity'}
        </span>
      </div>

      {hovered &&
        createPortal(<Tooltip cell={hovered.cell} anchor={hovered.rect} />, document.body)}
    </div>
  );
}
