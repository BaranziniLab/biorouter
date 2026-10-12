import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { ActivityWindow, DailyActivity } from '../../api';
import { prefersReducedMotion } from '../../styles/motion';
import { isoDay, parseIsoDay } from '../../utils/usageStats';

/**
 * A contribution graph for Biorouter usage: one column per week of the fetched
 * window, seven weekday rows, the coral heat ramp. It is the grid only; the
 * usage card around it (`HomeUsageCard`) owns the streak footnote and the
 * `Less … More` legend.
 *
 * Shading is computed server-side (see `build_activity_window`) from the
 * quartiles of the active days in the window, so a single 1.8M-token day cannot
 * flatten every ordinary day into the faintest shade. Absolute numbers live in
 * the tooltip, where they belong.
 *
 * The grid is FLUID (Codex): `--heat-weeks` equal columns that share the card's
 * width, and square cells. 371 days is 53 columns, about 10.8px each in the
 * 760px card. Nothing is measured, so there is no fitting code and no label
 * size that shrinks with the cells: the month row is 12px at every width.
 */

const DAY_MS = 86_400_000;

/**
 * The loading skeleton's width when its caller does not say: 54, what a 371-day
 * window fills on six days of the week (`expectedHeatmapWeeks` says it exactly).
 */
export const LOADING_WEEKS = 54;

type Cell = {
  key: string;
  date: Date;
  day: DailyActivity | null;
  inStreak: boolean;
};

/**
 * Lay the window out Sunday-first so the columns line up with the month row.
 * The first column is padded back to Sunday, but the last column stops at the
 * window end so future days do not look like idle days.
 */
function buildGrid(window: ActivityWindow): { cells: Cell[]; weeks: number } {
  const byDate = new Map(window.days.map((d) => [d.date, d]));

  const end = parseIsoDay(window.end);
  const start = parseIsoDay(window.start);
  // pad back to Sunday
  start.setDate(start.getDate() - start.getDay());

  const total = Math.round((end.getTime() - start.getTime()) / DAY_MS) + 1;

  // The current streak is the run of active days ending at `window.end`. Mark
  // those cells so the person can see what the footnote is counting.
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
    // `setDate` rather than `+ i * DAY_MS`: a daylight-saving change makes one
    // local day 23 or 25 hours long, and a millisecond step drifts off midnight.
    const date = new Date(start);
    date.setDate(start.getDate() + i);
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

export const LEVEL_CLASS: Record<number, string> = {
  0: 'bg-heat-0',
  1: 'bg-heat-1',
  2: 'bg-heat-2',
  3: 'bg-heat-3',
  4: 'bg-heat-4',
};

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
    const above = anchor.top - height - 8;
    const below = anchor.bottom + 8;
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
      // `--z-dropdown`, not `--z-toast`: it is portalled to `document.body`, so
      // it only has to clear the sticky page chrome, and a transient popover is
      // exactly what `--z-dropdown` names.
      className="biorouter-heatmap-tooltip biorouter-popover-surface pointer-events-none fixed z-[var(--z-dropdown)] w-max max-w-[calc(100vw-16px)] rounded-container bg-background-default p-3"
      style={{
        left: pos?.left ?? -9999,
        top: pos?.top ?? -9999,
        visibility: pos ? 'visible' : 'hidden',
      }}
    >
      <p className="mb-1 text-label text-text-default">{label}</p>
      {cell.inStreak && (
        <p className="mb-1 text-supporting text-text-muted">Part of your current streak</p>
      )}
      {d ? (
        <>
          <Row label="Chats started" value={full.format(d.sessions)} />
          <Row label="Tokens processed" value={tokenDisplay(d)} />
          <Row label="Messages" value={full.format(d.messages)} />
          <p className="biorouter-heatmap-tooltip-note mt-2 border-t border-border-subtle pt-2 text-supporting text-text-subtle">
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
    <div className="flex items-baseline justify-between gap-5 py-0.5 text-supporting text-text-muted">
      <span>{label}</span>
      <b className="font-medium tabular-nums text-text-default">{value}</b>
    </div>
  );
}

/** `Less ■■■■■ More`: the heat ramp's legend, for the card's footnote row. */
export function HeatmapLegend() {
  return (
    <span className="biorouter-heatmap-legend">
      <span>Less</span>
      {[0, 1, 2, 3, 4].map((level) => (
        <i
          key={level}
          aria-hidden="true"
          className={`biorouter-heatmap-swatch ${LEVEL_CLASS[level]}`}
        />
      ))}
      <span>More</span>
    </span>
  );
}

/**
 * The 12px ringed cell that leads the footnote's streak words: a heat step with the
 * streak ring on it, which is what a streak cell looks like (an active day, ringed).
 * It is FILLED on purpose: a hollow ringed square beside a label reads as an unchecked
 * checkbox. While a streak is running its ring is the cells' own ink (the key to them);
 * with none running there are no ringed cells to point at, so the ring goes muted and
 * means "streak" only.
 */
export function StreakGlyph({ running = true }: { running?: boolean }) {
  return (
    <i
      aria-hidden="true"
      className={`biorouter-heatmap-swatch ${LEVEL_CLASS[3]}`}
      data-streak={running ? 'true' : 'past'}
    />
  );
}

/**
 * The grid's skeleton: the window's weeks, breathing, and nothing to read. Pass
 * `weeks` (`expectedHeatmapWeeks`) so it is as tall as the heatmap that replaces it.
 */
export function UsageHeatmapLoading({ weeks = LOADING_WEEKS }: { weeks?: number }) {
  return (
    <div
      className="biorouter-heatmap"
      aria-hidden="true"
      style={{ '--heat-weeks': weeks } as React.CSSProperties}
    >
      <div className="biorouter-heatmap-months">
        <span className="biorouter-heatmap-loading-line h-3 w-7 rounded-inner bg-heat-0" />
      </div>
      <div className="biorouter-heatmap-grid">
        {Array.from({ length: 7 }, (_, row) => (
          <div key={row} className="biorouter-heatmap-row">
            {Array.from({ length: weeks }, (_, column) => (
              <i
                key={column}
                data-testid="heatmap-loading-cell"
                className="biorouter-heatmap-cell biorouter-heatmap-loading-cell bg-heat-0"
                style={{ animationDelay: `${-((column * 70 + row * 25) % 2100)}ms` }}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

/**
 * The reveal plays on the first heatmap a renderer shows (one deliberate moment
 * of delight, spec 3.9), never on a remount after navigation, a range change or
 * a tab change: those mount at rest.
 */
let revealedThisSession = false;

/** Tests only: let the next mount reveal again. */
export function resetHeatmapRevealForTests() {
  revealedThisSession = false;
}

/** Codex's stagger: `col * 8 + row * 12` ms. */
const REVEAL_COLUMN_MS = 8;
const REVEAL_ROW_MS = 12;

export function UsageHeatmap({ window: activity }: { window: ActivityWindow }) {
  const { cells, weeks } = useMemo(() => buildGrid(activity), [activity]);
  const [hovered, setHovered] = useState<{ cell: Cell; rect: Anchor } | null>(null);
  // Decided once, without a side effect (StrictMode runs initializers twice);
  // the session flag is set after commit.
  const [reveal] = useState(() => !revealedThisSession && !prefersReducedMotion());
  useEffect(() => {
    revealedThisSession = true;
  }, []);

  /*
   * ONE TAB STOP, NOT ONE PER DAY (triage T-20). Every day used to be its own
   * `<button>`, so Tab crossed Home one DAY at a time. The grid is a single
   * stop with a roving `tabIndex`: the entry cell is today (the last day), or
   * the day the person last moved to, and the arrow keys walk the calendar from
   * there (`nextHeatCell`). The active day is held by its DATE, not its index,
   * so a refreshed window that shifts the columns keeps the same day focusable.
   */
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const activeIndex = useMemo(() => {
    const index = activeKey === null ? -1 : cells.findIndex((cell) => cell.key === activeKey);
    return index >= 0 ? index : cells.length - 1;
  }, [cells, activeKey]);
  const cellRefs = useRef<(HTMLDivElement | null)[]>([]);

  // Row-major for the DOM, because an ARIA grid is rows of cells: row `d` holds
  // weekday `d` of every week. Each row is its own CSS grid on the same
  // `--heat-weeks` template, so the columns line up, and the unfinished last
  // week simply ends early in the rows after today.
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

  // One label per month, placed on the first column whose Sunday falls in it. A label that
  // starts in the last two columns has no room for its three letters under them (a column is
  // about 11px, a label about 21px), so it would hang past the grid's right edge, the one edge
  // the range control, the cells and the legend share: `end` ends it flush with that edge.
  const months = useMemo(() => {
    const out: { column: number; label: string; end: boolean }[] = [];
    let last = '';
    for (let w = 0; w < weeks; w++) {
      const d = cells[w * 7].date;
      const key = `${d.getFullYear()}-${d.getMonth()}`;
      if (key !== last && d.getDate() <= 7) {
        last = key;
        out.push({
          column: w,
          label: d.toLocaleDateString('en-US', { month: 'short' }),
          end: weeks - w <= 2,
        });
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
    <div
      className="biorouter-heatmap"
      data-reveal={reveal ? 'true' : undefined}
      style={{ '--heat-weeks': weeks } as React.CSSProperties}
    >
      <div className="biorouter-heatmap-months" aria-hidden="true">
        {months.map(({ column, label, end }) => (
          <span
            key={column}
            data-end={end ? '' : undefined}
            style={end ? undefined : { gridColumnStart: column + 1 }}
          >
            {label}
          </span>
        ))}
      </div>

      <div
        className="biorouter-heatmap-grid"
        role="grid"
        aria-label="Daily usage heatmap"
        onKeyDown={handleGridKeyDown}
      >
        {rows.map((row, day) => (
          <div key={day} role="row" className="biorouter-heatmap-row">
            {row.map(({ cell, index }) => (
              <div
                key={cell.key}
                ref={(element) => {
                  cellRefs.current[index] = element;
                }}
                role="gridcell"
                data-index={index}
                data-streak={cell.inStreak ? 'true' : undefined}
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
                className={`biorouter-heatmap-cell ${LEVEL_CLASS[cell.day?.level ?? 0]}`}
                style={
                  reveal
                    ? ({
                        '--heat-delay': `${Math.floor(index / 7) * REVEAL_COLUMN_MS + day * REVEAL_ROW_MS}ms`,
                      } as React.CSSProperties)
                    : undefined
                }
              />
            ))}
          </div>
        ))}
      </div>

      {hovered &&
        createPortal(<Tooltip cell={hovered.cell} anchor={hovered.rect} />, document.body)}
    </div>
  );
}
