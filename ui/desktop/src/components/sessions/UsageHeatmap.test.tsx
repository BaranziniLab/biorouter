import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ActivityWindow } from '../../api';
import { expectedHeatmapWeeks } from '../../utils/usageStats';
import {
  HeatmapLegend,
  LOADING_WEEKS,
  StreakGlyph,
  UsageHeatmap,
  UsageHeatmapLoading,
  resetHeatmapRevealForTests,
} from './UsageHeatmap';

beforeEach(() => {
  resetHeatmapRevealForTests();
});

function windowOf(overrides: Partial<ActivityWindow> = {}): ActivityWindow {
  return {
    start: '2026-03-01', // a Sunday
    end: '2026-03-14', // a Saturday
    maxSessions: 3,
    maxTokens: 128402,
    tokensComplete: true,
    currentStreak: 0,
    longestStreak: 0,
    days: [],
    ...overrides,
  };
}

const day = (date: string, level: number, sessions = 1, tokens = 1000, tokensComplete = true) => ({
  date,
  sessions,
  tokens,
  tokensComplete,
  inputTokens: 0,
  outputTokens: 0,
  messages: 4,
  level,
});

describe('UsageHeatmap', () => {
  it('does not repeat the hover explanation above the grid', () => {
    render(<UsageHeatmap window={windowOf()} />);
    expect(screen.queryByText(/Daily usage intensity/)).not.toBeInTheDocument();
  });

  it('renders complete weeks when the window ends on Saturday', () => {
    render(<UsageHeatmap window={windowOf()} />);
    const cells = screen.getAllByRole('gridcell');
    // Mar 1 2026 is a Sunday and Mar 14 a Saturday: exactly two full weeks.
    expect(cells).toHaveLength(14);
    expect(cells.length % 7).toBe(0);
  });

  it('pads the start back to Sunday but stops on the window end', () => {
    // Mar 4 is a Wednesday; Mar 11 is the following Wednesday. The leading
    // Sunday..Tuesday positions exist, but Thursday..Saturday in the last
    // column would represent future days and must not exist.
    render(<UsageHeatmap window={windowOf({ start: '2026-03-04', end: '2026-03-11' })} />);
    expect(screen.getAllByRole('gridcell')).toHaveLength(11);
    expect(screen.getByLabelText('2026-03-01: no activity')).toBeInTheDocument();
    expect(screen.getByLabelText('2026-03-11: no activity')).toBeInTheDocument();
    expect(screen.queryByLabelText('2026-03-12: no activity')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('2026-03-14: no activity')).not.toBeInTheDocument();
  });

  it('does not create cells after today in the current week', () => {
    render(<UsageHeatmap window={windowOf({ start: '2026-07-01', end: '2026-07-15' })} />);

    expect(screen.getByLabelText('2026-07-15: no activity')).toBeInTheDocument();
    expect(screen.queryByLabelText('2026-07-16: no activity')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('2026-07-18: no activity')).not.toBeInTheDocument();
  });

  it('omitted days render as level 0, present days keep their level', () => {
    render(
      <UsageHeatmap window={windowOf({ days: [day('2026-03-02', 4), day('2026-03-05', 1)] })} />
    );
    const busy = screen.getByLabelText(/2026-03-02: 1 chat, 1000 tokens/);
    expect(busy.className).toContain('bg-heat-4');
    const quiet = screen.getByLabelText(/2026-03-05: 1 chat, 1000 tokens/);
    expect(quiet.className).toContain('bg-heat-1');
    const idle = screen.getByLabelText('2026-03-03: no activity');
    expect(idle.className).toContain('bg-heat-0');
  });

  it('opens the tooltip on hover with the real numbers', () => {
    render(
      <UsageHeatmap
        window={windowOf({ days: [day('2026-03-02', 3, 3, 128402)], currentStreak: 0 })}
      />
    );
    expect(screen.queryByRole('tooltip')).toBeNull();

    fireEvent.mouseEnter(screen.getByLabelText(/2026-03-02/));
    const tip = screen.getByRole('tooltip');
    expect(tip).toHaveTextContent('Mar 2, 2026');
    expect(tip).toHaveTextContent('128,402');
    expect(tip).toHaveTextContent('Chats started');

    fireEvent.mouseLeave(screen.getByLabelText(/2026-03-02/));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('draws the tooltip on the one floating-surface recipe, so dark lifts it off the card', () => {
    // A floating surface keeps its hairline and shadow (R-27), and in dark it steps up one ground
    // and takes the stronger edge, which `.dark .biorouter-popover-surface` does. The tooltip used
    // to hand-roll `border` + `shadow-popover` on the card's own ground and so vanished on the
    // dark fill (vision round 1, V1-M5).
    render(<UsageHeatmap window={windowOf({ days: [day('2026-03-02', 3, 3, 128402)] })} />);
    fireEvent.mouseEnter(screen.getByLabelText(/2026-03-02/));
    const tip = screen.getByRole('tooltip');
    expect(tip).toHaveClass('biorouter-popover-surface', 'rounded-container');
    expect(tip.className).not.toMatch(/\bborder\b|shadow-popover/);
  });

  it('marks activity token counts as lower bounds when the server reports incomplete history', () => {
    render(
      <UsageHeatmap
        window={windowOf({
          tokensComplete: false,
          days: [day('2026-03-02', 3, 3, 128402, false)],
        })}
      />
    );

    // The window-level explanation is the usage card's Tokens InfoTip now; the
    // grid carries no paragraph and no live region of its own.
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByText(/conservative estimates/)).toBeNull();
    expect(screen.queryByText(/Highest recorded estimate/)).toBeNull();
    const cell = screen.getByLabelText(/2026-03-02: 3 chats, 128402 estimated tokens/);
    fireEvent.mouseEnter(cell);
    expect(screen.getByRole('tooltip')).toHaveTextContent('128,402');
    expect(screen.getByRole('tooltip')).toHaveTextContent('Conservative estimate');
    expect(screen.getByRole('tooltip')).not.toHaveTextContent('≥');
  });

  it('shows unavailable instead of a meaningless lower bound of zero', () => {
    render(
      <UsageHeatmap
        window={windowOf({
          tokensComplete: false,
          maxTokens: 0,
          days: [day('2026-03-02', 3, 3, 0, false)],
        })}
      />
    );

    expect(screen.queryByText('Token totals unavailable for older activity')).toBeNull();
    const cell = screen.getByLabelText(/2026-03-02: 3 chats, token total unavailable/);
    fireEvent.mouseEnter(cell);
    expect(screen.getByRole('tooltip')).toHaveTextContent('Tokens processedUnavailable');
    expect(screen.getByRole('tooltip')).toHaveTextContent('exact total cannot be shown');
  });

  it('keeps exact days exact when another day makes the window incomplete', () => {
    render(
      <UsageHeatmap
        window={windowOf({
          tokensComplete: false,
          days: [day('2026-03-02', 3, 3, 42000, true)],
        })}
      />
    );

    const cell = screen.getByLabelText(/2026-03-02: 3 chats, 42000 tokens/);
    fireEvent.mouseEnter(cell);
    expect(screen.getByRole('tooltip')).toHaveTextContent('42,000');
    expect(screen.getByRole('tooltip')).not.toHaveTextContent('Conservative estimate');
  });

  it('opens the tooltip on keyboard focus, not hover alone', () => {
    // The tooltip is the only way to read a cell's numbers, so a keyboard user
    // must be able to reach it.
    render(<UsageHeatmap window={windowOf({ days: [day('2026-03-02', 2)] })} />);
    act(() => screen.getByLabelText(/2026-03-02/).focus());
    expect(screen.getByRole('tooltip')).toHaveTextContent('Mar 2, 2026');
  });

  it('says "no activity" for an idle day', () => {
    render(<UsageHeatmap window={windowOf()} />);
    fireEvent.mouseEnter(screen.getByLabelText('2026-03-03: no activity'));
    expect(screen.getByRole('tooltip')).toHaveTextContent('No activity');
  });

  it('marks exactly the current streak', () => {
    render(
      <UsageHeatmap
        window={windowOf({
          days: [day('2026-03-12', 2), day('2026-03-13', 2), day('2026-03-14', 2)],
          currentStreak: 3,
          longestStreak: 5,
        })}
      />
    );
    const outlined = screen
      .getAllByRole('gridcell')
      .filter((cell) => cell.getAttribute('data-streak') === 'true');
    expect(outlined).toHaveLength(3);
    for (const cell of outlined) {
      expect(cell).toHaveAccessibleName(/part of current streak/);
    }
    // The streak's words are the usage card's footnote (`streakFootnote`).
    expect(screen.queryByText('3 day streak')).toBeNull();
    expect(screen.queryByText(/Longest streak/)).toBeNull();

    fireEvent.mouseEnter(outlined[0]);
    expect(screen.getByRole('tooltip')).toHaveTextContent('Part of your current streak');
  });

  it('a streak that ended yesterday still highlights the right cells', () => {
    // The server reports currentStreak counting back from `end`; if the user has
    // not opened the app today, the run ends on `end - 1`.
    render(
      <UsageHeatmap
        window={windowOf({ days: [day('2026-03-12', 2), day('2026-03-13', 2)], currentStreak: 2 })}
      />
    );
    const outlined = screen
      .getAllByRole('gridcell')
      .filter((cell) => cell.getAttribute('data-streak') === 'true');
    expect(outlined.map((b) => b.getAttribute('aria-label'))).toEqual([
      expect.stringContaining('2026-03-12'),
      expect.stringContaining('2026-03-13'),
    ]);
  });
});

describe('UsageHeatmap layout', () => {
  const YEAR = { start: '2025-10-04', end: '2026-10-09' }; // 371 days

  it('draws one column per week of the window, never a fixed count', () => {
    const { container } = render(<UsageHeatmap window={windowOf(YEAR)} />);
    const root = container.querySelector<HTMLElement>('.biorouter-heatmap')!;
    // 2025-10-04 is a Saturday, padded back to Sunday Sep 28: 377 days, so the
    // ragged last week makes 54 columns.
    expect(root.style.getPropertyValue('--heat-weeks')).toBe('54');
    expect(screen.getAllByRole('gridcell')).toHaveLength(377);

    const { container: short } = render(<UsageHeatmap window={windowOf()} />);
    expect(
      short.querySelector<HTMLElement>('.biorouter-heatmap')!.style.getPropertyValue('--heat-weeks')
    ).toBe('2');
  });

  it('labels months over their first full week and draws no weekday labels', () => {
    const { container } = render(<UsageHeatmap window={windowOf(YEAR)} />);
    const months = container.querySelector('.biorouter-heatmap-months')!;
    expect(months).toHaveAttribute('aria-hidden', 'true');
    const labels = [...months.querySelectorAll('span')].map((label) => label.textContent);
    expect(labels).toEqual([
      'Oct',
      'Nov',
      'Dec',
      'Jan',
      'Feb',
      'Mar',
      'Apr',
      'May',
      'Jun',
      'Jul',
      'Aug',
      'Sep',
      'Oct',
    ]);
    for (const weekday of ['Sun', 'Mon', 'Tue', 'Thu', 'Sat']) {
      expect(screen.queryByText(weekday)).toBeNull();
    }
  });

  it('flags only a label in the last two columns, to end flush with the grid edge', () => {
    // The window ends Friday 2026-10-09; its last column starts Sunday 2026-10-04, so "Oct" has
    // one column under it: about 11px for a label of about 21px, which would hang past the
    // grid's right edge. Only that label ends at the edge; the rest sit on their own column.
    const { container } = render(<UsageHeatmap window={windowOf(YEAR)} />);
    const labels = [...container.querySelectorAll<HTMLElement>('.biorouter-heatmap-months span')];
    const flagged = labels.filter((label) => label.hasAttribute('data-end'));
    expect(flagged.map((label) => label.textContent)).toEqual(['Oct']);
    expect(flagged[0]).toBe(labels[labels.length - 1]);
    // A flagged label spans the row (the stylesheet), so it carries no column of its own.
    expect(flagged[0].style.gridColumnStart).toBe('');
    for (const label of labels.filter((label) => !label.hasAttribute('data-end'))) {
      expect(label.style.gridColumnStart).not.toBe('');
    }

    // A window whose last week is mid-month has no such label: nothing is flagged.
    const mid = render(
      <UsageHeatmap window={windowOf({ start: '2026-03-01', end: '2026-03-21' })} />
    );
    expect(mid.container.querySelectorAll('.biorouter-heatmap-months [data-end]')).toHaveLength(0);
  });

  it('reveals the first heatmap of a session once, and later mounts at rest', () => {
    const first = render(<UsageHeatmap window={windowOf()} />);
    const root = first.container.querySelector('.biorouter-heatmap')!;
    expect(root).toHaveAttribute('data-reveal', 'true');
    // Codex's stagger: col * 8 + row * 12 ms. Mar 10 is week 1, a Tuesday (row 2).
    expect(
      screen.getByLabelText('2026-03-10: no activity').style.getPropertyValue('--heat-delay')
    ).toBe('32ms');
    first.unmount();

    const again = render(<UsageHeatmap window={windowOf()} />);
    expect(again.container.querySelector('.biorouter-heatmap')).not.toHaveAttribute('data-reveal');
    expect(
      screen.getByLabelText('2026-03-10: no activity').style.getPropertyValue('--heat-delay')
    ).toBe('');
  });

  describe('under reduced motion', () => {
    const original = window.matchMedia;
    beforeEach(() => {
      window.matchMedia = vi.fn((query: string) => ({
        matches: query.includes('reduce'),
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      })) as unknown as typeof window.matchMedia;
    });
    afterEach(() => {
      window.matchMedia = original;
    });

    it('never reveals', () => {
      const { container } = render(<UsageHeatmap window={windowOf()} />);
      expect(container.querySelector('.biorouter-heatmap')).not.toHaveAttribute('data-reveal');
    });
  });
});

describe('legend and streak glyph', () => {
  it('draws the five steps of the ramp between Less and More', () => {
    const { container } = render(<HeatmapLegend />);
    expect(container).toHaveTextContent('LessMore');
    const swatches = container.querySelectorAll('.biorouter-heatmap-swatch');
    expect([...swatches].map((swatch) => swatch.className)).toEqual([
      expect.stringContaining('bg-heat-0'),
      expect.stringContaining('bg-heat-1'),
      expect.stringContaining('bg-heat-2'),
      expect.stringContaining('bg-heat-3'),
      expect.stringContaining('bg-heat-4'),
    ]);
  });

  it('rings the streak glyph the way it rings a streak cell', () => {
    const { container } = render(<StreakGlyph />);
    expect(container.firstElementChild).toHaveAttribute('data-streak', 'true');
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true');
  });
});

/**
 * T-20: the heat map was one Tab stop PER DAY — some 160 presses to get past a
 * chart on Home. It is one stop now, entered on today, and walked with the
 * arrow keys: ↑/↓ a day, ←/→ a week, Home/End the row, Ctrl/⌘+Home/End the
 * whole range.
 *
 * The window below is Sun 2026-03-01 … Wed 2026-03-18: two full weeks plus a
 * third that stops on "today", so the grid has a ragged last column — the case
 * where a week step or a row's End has to know where the days run out.
 */
describe('UsageHeatmap keyboard model', () => {
  const RAGGED = { start: '2026-03-01', end: '2026-03-18' };

  const cell = (date: string) => screen.getByLabelText(new RegExp(`^${date}:`));
  const focused = () => document.activeElement?.getAttribute('aria-label') ?? '';
  const press = (key: string, init: { ctrlKey?: boolean; metaKey?: boolean } = {}) =>
    fireEvent.keyDown(document.activeElement ?? document.body, { key, ...init });
  const enter = () => {
    const [stop] = screen.getAllByRole('gridcell').filter((c) => c.tabIndex === 0);
    act(() => stop.focus());
  };

  it('is ONE tab stop, entered on today', () => {
    render(<UsageHeatmap window={windowOf(RAGGED)} />);

    const cells = screen.getAllByRole('gridcell');
    expect(cells).toHaveLength(18);
    const stops = cells.filter((c) => c.tabIndex === 0);
    expect(stops).toHaveLength(1);
    expect(stops[0]).toHaveAccessibleName('2026-03-18: no activity');
    expect(cells.filter((c) => c.tabIndex === -1)).toHaveLength(17);
    // Not a button per day any more: nothing in the chart is a Tab stop but
    // the one cell.
    expect(screen.queryAllByRole('button')).toHaveLength(0);
  });

  it('has grid semantics: seven weekday rows of day cells', () => {
    render(<UsageHeatmap window={windowOf(RAGGED)} />);

    const grid = screen.getByRole('grid', { name: 'Daily usage heatmap' });
    const rows = within(grid).getAllByRole('row');
    expect(rows).toHaveLength(7);
    // Row 0 is every Sunday, in week order; the Thursday row stops a week
    // early because the third Thursday has not happened yet.
    expect(
      within(rows[0])
        .getAllByRole('gridcell')
        .map((c) => c.getAttribute('aria-label')?.slice(0, 10))
    ).toEqual(['2026-03-01', '2026-03-08', '2026-03-15']);
    expect(within(rows[4]).getAllByRole('gridcell')).toHaveLength(2);
  });

  it('moves a day with the vertical arrows and runs on across the week', () => {
    render(<UsageHeatmap window={windowOf(RAGGED)} />);
    enter();
    expect(focused()).toMatch(/^2026-03-18/);

    press('ArrowUp');
    expect(focused()).toMatch(/^2026-03-17/);
    // The roving stop follows focus, so Tab out and back returns here.
    expect(cell('2026-03-17')).toHaveAttribute('tabindex', '0');
    expect(cell('2026-03-18')).toHaveAttribute('tabindex', '-1');

    // Sunday → the Saturday before it: the day before, in the previous column.
    act(() => cell('2026-03-15').focus());
    press('ArrowUp');
    expect(focused()).toMatch(/^2026-03-14/);
    press('ArrowDown');
    expect(focused()).toMatch(/^2026-03-15/);
  });

  it('moves a week with the horizontal arrows and stops at either edge', () => {
    render(<UsageHeatmap window={windowOf(RAGGED)} />);
    enter();

    press('ArrowLeft');
    expect(focused()).toMatch(/^2026-03-11/);
    press('ArrowLeft');
    expect(focused()).toMatch(/^2026-03-04/);
    // No earlier week: focus stays, and the key is still consumed so the page
    // does not scroll under the user.
    const edge = fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' });
    expect(edge).toBe(false);
    expect(focused()).toMatch(/^2026-03-04/);

    // Thursday 12th → there is no Thursday 19th yet; focus must not jump.
    act(() => cell('2026-03-12').focus());
    press('ArrowRight');
    expect(focused()).toMatch(/^2026-03-12/);
    // And nothing after today exists to move down into.
    act(() => cell('2026-03-18').focus());
    press('ArrowDown');
    expect(focused()).toMatch(/^2026-03-18/);
  });

  it('jumps along the row with Home/End, and to the range ends with Ctrl or ⌘', () => {
    render(<UsageHeatmap window={windowOf(RAGGED)} />);
    act(() => cell('2026-03-10').focus()); // a Tuesday

    press('End');
    expect(focused()).toMatch(/^2026-03-17/);
    press('Home');
    expect(focused()).toMatch(/^2026-03-03/);

    press('End', { ctrlKey: true });
    expect(focused()).toMatch(/^2026-03-18/);
    press('Home', { metaKey: true });
    expect(focused()).toMatch(/^2026-03-01/);

    // A row whose last week has not arrived ends a week earlier.
    act(() => cell('2026-03-06').focus()); // a Friday
    press('End');
    expect(focused()).toMatch(/^2026-03-13/);
  });

  it('shows each day as it is reached, and Escape dismisses it without moving focus', () => {
    render(
      <UsageHeatmap window={windowOf({ ...RAGGED, days: [day('2026-03-17', 2, 2, 5000)] })} />
    );
    enter();
    press('ArrowUp');
    expect(screen.getByRole('tooltip')).toHaveTextContent('Mar 17, 2026');
    expect(screen.getByRole('tooltip')).toHaveTextContent('5,000');

    press('Escape');
    expect(screen.queryByRole('tooltip')).toBeNull();
    expect(focused()).toMatch(/^2026-03-17/);
  });

  it('keeps the same DAY as the stop when a refreshed window moves the columns', () => {
    const { rerender } = render(<UsageHeatmap window={windowOf(RAGGED)} />);
    act(() => cell('2026-03-10').focus());

    // The window slides forward a week; the 10th is now a different index.
    rerender(<UsageHeatmap window={windowOf({ start: '2026-03-08', end: '2026-03-18' })} />);
    expect(cell('2026-03-10')).toHaveAttribute('tabindex', '0');
    expect(screen.getAllByRole('gridcell').filter((c) => c.tabIndex === 0)).toHaveLength(1);

    // A day that scrolls out of the window hands the stop back to today.
    rerender(<UsageHeatmap window={windowOf({ start: '2026-03-15', end: '2026-03-18' })} />);
    expect(cell('2026-03-18')).toHaveAttribute('tabindex', '0');
  });
});

describe('UsageHeatmapLoading', () => {
  it('draws the full window as a breathing placeholder with nothing to read', () => {
    const { container } = render(<UsageHeatmapLoading />);

    expect(screen.getAllByTestId('heatmap-loading-cell')).toHaveLength(LOADING_WEEKS * 7);
    // The card around it carries the status; the grid itself is decoration.
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true');
    expect(screen.queryByText('Loading activity')).toBeNull();
    expect(container).toHaveTextContent('');
  });

  it('is as wide as the heatmap that replaces it, so the card keeps its height', () => {
    // The 371 days ending Sat 2026-10-10 fill 53 columns; ending Fri 2026-10-09, 54.
    for (const [end, today] of [
      ['2026-10-10', new Date(2026, 9, 10)],
      ['2026-10-09', new Date(2026, 9, 9)],
    ] as const) {
      const start = new Date(today);
      start.setDate(start.getDate() - 370);
      const iso = `${start.getFullYear()}-${String(start.getMonth() + 1).padStart(2, '0')}-${String(start.getDate()).padStart(2, '0')}`;
      const real = render(<UsageHeatmap window={windowOf({ start: iso, end })} />);
      const realWeeks = real.container
        .querySelector<HTMLElement>('.biorouter-heatmap')!
        .style.getPropertyValue('--heat-weeks');
      real.unmount();

      const loading = render(<UsageHeatmapLoading weeks={expectedHeatmapWeeks(371, today)} />);
      expect(
        loading.container
          .querySelector<HTMLElement>('.biorouter-heatmap')!
          .style.getPropertyValue('--heat-weeks')
      ).toBe(realWeeks);
      loading.unmount();
    }
  });
});
