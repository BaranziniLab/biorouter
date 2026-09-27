import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { ActivityWindow } from '../../api';
import { UsageHeatmap, UsageHeatmapLoading } from './UsageHeatmap';

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

  it('marks activity token counts as lower bounds when the server reports incomplete history', () => {
    render(
      <UsageHeatmap
        window={windowOf({
          tokensComplete: false,
          days: [day('2026-03-02', 3, 3, 128402, false)],
        })}
      />
    );

    expect(screen.getByRole('status')).toHaveTextContent('conservative estimates');
    expect(screen.getByText(/Highest recorded estimate/)).toHaveTextContent(
      'Highest recorded estimate · 128.4K tokens'
    );
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

    expect(screen.getByText('Token totals unavailable for older activity')).toBeInTheDocument();
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
      .filter((b) => b.className.includes('shadow-[inset_0_0_0_2px_var(--text-default)]'));
    expect(outlined).toHaveLength(3);
    for (const cell of outlined) {
      expect(cell).toHaveAccessibleName(/part of current streak/);
    }
    expect(screen.getByText('Current streak')).toBeInTheDocument();
    expect(screen.getByText('3 day streak')).toBeInTheDocument();
    expect(screen.getByText(/Longest streak · 5 days/)).toBeInTheDocument();

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
      .filter((b) => b.className.includes('inset_0_0_0_2px'));
    expect(outlined.map((b) => b.getAttribute('aria-label'))).toEqual([
      expect.stringContaining('2026-03-12'),
      expect.stringContaining('2026-03-13'),
    ]);
  });

  it('singularises the streak header', () => {
    render(<UsageHeatmap window={windowOf({ currentStreak: 1, longestStreak: 1 })} />);
    expect(screen.getByText('1 day streak')).toBeInTheDocument();
    expect(screen.getByText(/Longest streak · 1 day$/)).toBeInTheDocument();
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
  it('fills the loading state with an animated heatmap-shaped placeholder', () => {
    render(<UsageHeatmapLoading />);

    expect(screen.getByRole('status', { name: 'Loading usage activity' })).toBeInTheDocument();
    expect(screen.getAllByTestId('heatmap-loading-cell')).toHaveLength(154);
    expect(screen.getByText('Loading activity')).toBeInTheDocument();
  });
});
