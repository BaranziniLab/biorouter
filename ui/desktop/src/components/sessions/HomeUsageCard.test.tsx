import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ActivityWindow, DailyActivity, UsageReportRow } from '../../api';
import { homeUsageCopy as copy } from '../usage/copy';
import { HOME_USAGE_RANGE_STORAGE_KEY, HomeUsageCard, HomeUsageCardLoading } from './HomeUsageCard';
import { resetHeatmapRevealForTests } from './UsageHeatmap';

const mocks = vi.hoisted(() => ({
  getUsageReport: vi.fn(),
  isBrowserSurface: vi.fn(() => false),
}));

vi.mock('../../api', () => ({ getUsageReport: mocks.getUsageReport }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: async () => ({}) }));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: mocks.isBrowserSurface }));
vi.mock('../settings/models/predefinedModelsUtils', () => ({
  getModelDisplayName: (id: string) => (id === 'gpt-5.6-sol' ? 'GPT-5.6 Sol' : id),
}));

const day = (
  date: string,
  { sessions = 1, tokens = 1000, messages = 4, tokensComplete = true } = {}
): DailyActivity => ({
  date,
  sessions,
  tokens,
  tokensComplete,
  inputTokens: 0,
  outputTokens: 0,
  messages,
  level: 2,
});

function windowOf(overrides: Partial<ActivityWindow> = {}): ActivityWindow {
  return {
    start: '2025-10-04',
    end: '2026-10-09',
    maxSessions: 3,
    maxTokens: 1000,
    tokensComplete: true,
    currentStreak: 3,
    longestStreak: 5,
    days: [
      day('2026-01-15', { sessions: 10, tokens: 2_000_000, messages: 100 }),
      day('2026-09-20', { sessions: 2, tokens: 30_000, messages: 20 }),
      day('2026-10-07', { sessions: 1, tokens: 1_000, messages: 5 }),
      day('2026-10-08', { sessions: 1, tokens: 1_000, messages: 5 }),
      day('2026-10-09', { sessions: 1, tokens: 1_000, messages: 5 }),
    ],
    ...overrides,
  };
}

const report = (modelId: string | null, totalTokens: number, turns = 1): UsageReportRow => ({
  date: null,
  modelId,
  provider: modelId ? 'versa_azure' : null,
  inputTokens: totalTokens,
  outputTokens: 0,
  totalTokens,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  turns,
  cost: null,
  hasUnpriced: false,
  costExcludesCache: false,
});

const tiles = () => screen.getByTestId('home-usage-tiles');
const figure = (label: string) => {
  const term = within(tiles())
    .getAllByRole('term')
    .find((t) => t.querySelector('.br-stat-label-text')?.textContent === label);
  if (!term) throw new Error(`no tile labelled ${label}`);
  return term.nextElementSibling as HTMLElement;
};

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  resetHeatmapRevealForTests();
  mocks.isBrowserSurface.mockReturnValue(false);
  mocks.getUsageReport.mockResolvedValue({
    data: { rows: [report('gpt-5.6-sol', 900), report('claude-opus', 100)] },
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('HomeUsageCard', () => {
  it('is one labelled region: underline tabs, a range control, six tiles, the grid', async () => {
    render(<HomeUsageCard window={windowOf()} />);

    const card = screen.getByRole('region', { name: copy.region });
    const tablist = within(card).getByRole('tablist', { name: copy.tabsLabel });
    expect(
      within(tablist)
        .getAllByRole('tab')
        .map((tab) => tab.textContent)
    ).toEqual(['Overview', 'Models']);
    expect(within(tablist).getByRole('tab', { name: 'Overview' })).toHaveAttribute(
      'aria-selected',
      'true'
    );

    const range = within(card).getByRole('radiogroup', { name: copy.rangeLabel });
    const radios = within(range).getAllByRole('radio');
    expect(radios.map((radio) => radio.getAttribute('aria-label'))).toEqual([
      'Last year',
      'Last 30 days',
      'Last 7 days',
    ]);
    expect(radios.map((radio) => radio.textContent)).toEqual(['1y', '30d', '7d']);
    expect(within(range).getByRole('radio', { name: 'Last year' })).toHaveAttribute(
      'aria-checked',
      'true'
    );

    expect(
      within(tiles())
        .getAllByRole('term')
        .map((term) => term.querySelector('.br-stat-label-text')?.textContent)
    ).toEqual(['Chats', 'Messages', 'Tokens', 'Active days', 'Streak', 'Favorite model']);
    expect(within(card).getByRole('grid', { name: 'Daily usage heatmap' })).toBeInTheDocument();
    await waitFor(() => expect(figure('Favorite model')).toHaveTextContent('GPT-5.6 Sol'));
  });

  it('says nothing in paragraphs: no live region, no caps, no 17px heading', async () => {
    const { container } = render(<HomeUsageCard window={windowOf()} />);
    await waitFor(() => expect(figure('Favorite model')).toHaveTextContent('GPT-5.6 Sol'));
    expect(screen.queryByRole('status')).toBeNull();
    expect(container.querySelector('.text-caps, .text-subheading, h2, p')).toBeNull();
  });

  it('shows the year by default and moves the tiles, not the heatmap, with the range', async () => {
    const user = userEvent.setup();
    render(<HomeUsageCard window={windowOf()} />);

    expect(figure('Chats')).toHaveTextContent('15');
    expect(figure('Messages')).toHaveTextContent('135');
    expect(figure('Tokens')).toHaveTextContent('2M');
    expect(figure('Active days')).toHaveTextContent('5/365');
    const cellsBefore = screen.getAllByRole('gridcell').length;

    await user.click(screen.getByRole('radio', { name: 'Last 30 days' }));
    expect(figure('Chats')).toHaveTextContent('5');
    expect(figure('Tokens')).toHaveTextContent('33K');
    expect(figure('Active days')).toHaveTextContent('4/30');
    // A range switch is a change the person made: the figures fade in.
    expect(figure('Chats')).toHaveAttribute('data-animate');

    await user.click(screen.getByRole('radio', { name: 'Last 7 days' }));
    expect(figure('Chats')).toHaveTextContent('3');
    expect(figure('Active days')).toHaveTextContent('3/7');

    expect(screen.getAllByRole('gridcell')).toHaveLength(cellsBefore);
    // The report is asked for each range, from that range's first day.
    await waitFor(() => expect(mocks.getUsageReport).toHaveBeenCalledTimes(3));
    expect(mocks.getUsageReport.mock.calls[2][0]).toMatchObject({
      query: {
        from: Math.floor(new Date(2026, 9, 3).getTime() / 1000),
        group: 'model',
      },
    });
  });

  it('mounts at rest: the first figures do not fade in', () => {
    render(<HomeUsageCard window={windowOf()} />);
    expect(figure('Chats')).not.toHaveAttribute('data-animate');
  });

  it('remembers the range per viewer, and survives storage that throws', async () => {
    const user = userEvent.setup();
    const first = render(<HomeUsageCard window={windowOf()} />);
    await user.click(screen.getByRole('radio', { name: 'Last 7 days' }));
    expect(localStorage.getItem(HOME_USAGE_RANGE_STORAGE_KEY)).toBe('7d');
    first.unmount();

    render(<HomeUsageCard window={windowOf()} />);
    expect(screen.getByRole('radio', { name: 'Last 7 days' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  it('falls back to the year when storage throws or holds a stranger', () => {
    localStorage.setItem(HOME_USAGE_RANGE_STORAGE_KEY, 'all');
    const { unmount } = render(<HomeUsageCard window={windowOf()} />);
    expect(screen.getByRole('radio', { name: 'Last year' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
    unmount();

    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    render(<HomeUsageCard window={windowOf()} />);
    expect(screen.getByRole('radio', { name: 'Last year' })).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  it('explains a lower-bound token total only when the range has an incomplete day', async () => {
    const user = userEvent.setup();
    render(
      <HomeUsageCard
        window={windowOf({
          days: [
            day('2026-01-15', { tokens: 500, tokensComplete: false }),
            day('2026-10-09', { tokens: 1_000 }),
          ],
        })}
      />
    );
    const tip = screen.getByRole('button', { name: 'About Tokens' });
    expect(tip).toHaveAccessibleDescription(copy.tokensPartialHelp);

    await user.click(screen.getByRole('radio', { name: 'Last 7 days' }));
    expect(screen.queryByRole('button', { name: 'About Tokens' })).toBeNull();
  });

  it('says Unavailable, not 0, over a range with no trustworthy total', () => {
    render(
      <HomeUsageCard
        window={windowOf({ days: [day('2026-10-09', { tokens: 0, tokensComplete: false })] })}
      />
    );
    expect(figure('Tokens')).toHaveTextContent(copy.tokensUnavailable);
    expect(screen.getByRole('button', { name: 'About Tokens' })).toHaveAccessibleDescription(
      copy.tokensUnavailableHelp
    );
  });

  it('keeps the explanations of Messages and Streak behind InfoTips', () => {
    render(<HomeUsageCard window={windowOf()} />);
    expect(screen.getByRole('button', { name: 'About Messages' })).toHaveAccessibleDescription(
      copy.messagesHelp
    );
    expect(screen.getByRole('button', { name: 'About Streak' })).toHaveAccessibleDescription(
      'Longest in the last year: 5 days.'
    );
    expect(figure('Streak')).toHaveTextContent('3days');
  });

  it('ends on one footnote line: the streak and the legend', () => {
    render(<HomeUsageCard window={windowOf()} />);
    expect(screen.getByTestId('home-usage-streak')).toHaveTextContent(
      '3 day streak · longest 5 days'
    );
    expect(screen.getByText('Less')).toBeInTheDocument();
    expect(screen.getByText('More')).toBeInTheDocument();
  });

  it('drops the streak words when there has never been one', () => {
    render(<HomeUsageCard window={windowOf({ currentStreak: 0, longestStreak: 0 })} />);
    expect(screen.queryByTestId('home-usage-streak')).toBeNull();
  });

  it('lists the top three models and Other on the Models tab, with the scope in a tip', async () => {
    const user = userEvent.setup();
    mocks.getUsageReport.mockResolvedValue({
      data: {
        rows: [
          report('gpt-5.6-sol', 500),
          report('b', 200),
          report('c', 150),
          report('d', 100),
          report('e', 50),
        ],
      },
    });
    render(<HomeUsageCard window={windowOf()} />);
    expect(screen.getByRole('button', { name: 'About Models' })).toHaveAccessibleDescription(
      copy.modelsHelp
    );

    await user.click(screen.getByRole('tab', { name: 'Models' }));
    const rows = await screen.findAllByTestId('home-usage-model');
    expect(rows.map((row) => row.firstElementChild?.textContent)).toEqual([
      'GPT-5.6 Sol',
      'b',
      'c',
      'Other',
    ]);
    expect(rows[0]).toHaveTextContent('50%');
    expect(rows[0].firstElementChild).toHaveAttribute('title', 'versa_azure/gpt-5.6-sol');
    expect(rows[3]).toHaveTextContent('15%');
    // The heatmap stays on both tabs.
    expect(screen.getByRole('grid', { name: 'Daily usage heatmap' })).toBeInTheDocument();
  });

  it('says so in one line when no model was used in the range', async () => {
    const user = userEvent.setup();
    mocks.getUsageReport.mockResolvedValue({ data: { rows: [] } });
    render(<HomeUsageCard window={windowOf()} />);
    await waitFor(() => expect(figure('Favorite model')).toHaveTextContent(copy.none));
    await user.click(screen.getByRole('tab', { name: 'Models' }));
    expect(await screen.findByText(copy.modelsEmpty)).toBeInTheDocument();
  });

  it('on a browser surface asks for no report, hides Models and shows the longest streak', () => {
    mocks.isBrowserSurface.mockReturnValue(true);
    render(<HomeUsageCard window={windowOf()} />);

    expect(mocks.getUsageReport).not.toHaveBeenCalled();
    expect(screen.queryByRole('tab', { name: 'Models' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'About Models' })).toBeNull();
    expect(figure('Longest streak')).toHaveTextContent('5days');
  });

  it('falls back the same way when the report is refused', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    mocks.getUsageReport.mockRejectedValue(new Error('403'));
    render(<HomeUsageCard window={windowOf()} />);

    await waitFor(() => expect(screen.queryByRole('tab', { name: 'Models' })).toBeNull());
    expect(figure('Longest streak')).toHaveTextContent('5days');
  });
});

describe('HomeUsageCardLoading', () => {
  it('is the same card, disabled, with skeletons and a named status but no words', () => {
    const { container } = render(<HomeUsageCardLoading />);
    const status = screen.getByRole('status', { name: copy.loading });
    expect(status).toHaveTextContent('');
    expect(screen.getAllByTestId('home-usage-tile-skeleton')).toHaveLength(6);
    for (const tab of screen.getAllByRole('tab')) expect(tab).toBeDisabled();
    for (const radio of screen.getAllByRole('radio')) expect(radio).toBeDisabled();
    expect(container.querySelectorAll('[data-testid="heatmap-loading-cell"]').length).toBe(371);
  });
});
