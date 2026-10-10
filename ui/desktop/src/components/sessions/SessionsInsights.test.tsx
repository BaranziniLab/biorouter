import { render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SessionInsights } from './SessionsInsights';
import { cacheHomeActivity, clearHomeInsightsCache } from '../../utils/homeInsightsCache';

const mocks = vi.hoisted(() => ({
  getSessionActivity: vi.fn(),
}));

vi.mock('../../api', () => ({
  getSessionActivity: mocks.getSessionActivity,
}));

vi.mock('../common/Greeting', () => ({
  Greeting: ({ className }: { className?: string }) => <h1 className={className}>Greeting</h1>,
}));

vi.mock('./HomeUsageCard', () => ({
  HomeUsageCard: ({ window }: { window: { currentStreak: number } }) => (
    <div>Usage heatmap {window.currentStreak}</div>
  ),
  HomeUsageCardLoading: () => <div>Loading usage</div>,
}));

const activity = {
  start: '2026-02-11',
  end: '2026-07-15',
  maxSessions: 4,
  maxTokens: 1000,
  tokensComplete: true,
  currentStreak: 7,
  longestStreak: 12,
  days: [],
};

beforeEach(() => {
  vi.clearAllMocks();
  clearHomeInsightsCache();
  mocks.getSessionActivity.mockReturnValue(new Promise(() => {}));
});

describe('SessionInsights', () => {
  it('does not list recent chats — the sidebar Recents owns that job', async () => {
    render(
      <MemoryRouter>
        <SessionInsights />
      </MemoryRouter>
    );

    expect(screen.queryByText('Recent chats')).not.toBeInTheDocument();
    expect(screen.queryByText('See all')).not.toBeInTheDocument();
  });

  it('has no eyebrow above the greeting, and the greeting takes the title role', () => {
    render(
      <MemoryRouter>
        <SessionInsights />
      </MemoryRouter>
    );

    expect(screen.queryByText(/UCSF Biorouter/i)).not.toBeInTheDocument();
    const greeting = screen.getByRole('heading', { name: 'Greeting' });
    expect(greeting).toHaveClass('text-title');
    expect(greeting).not.toHaveClass('text-2xl', 'font-semibold');
  });

  it('shows the card loading state until activity arrives', () => {
    render(
      <MemoryRouter>
        <SessionInsights />
      </MemoryRouter>
    );

    expect(screen.getByText('Loading usage')).toBeInTheDocument();
  });

  it('renders persisted activity immediately while refreshing', async () => {
    cacheHomeActivity(activity);

    render(
      <MemoryRouter>
        <SessionInsights />
      </MemoryRouter>
    );

    expect(screen.getByText('Usage heatmap 7')).toBeInTheDocument();
    expect(screen.queryByText('Loading usage')).not.toBeInTheDocument();
    await waitFor(() => expect(mocks.getSessionActivity).toHaveBeenCalledTimes(1));
  });

  it('renders refreshed activity once the request resolves', async () => {
    mocks.getSessionActivity.mockResolvedValue({ data: activity });

    render(
      <MemoryRouter>
        <SessionInsights />
      </MemoryRouter>
    );

    await waitFor(() => expect(screen.getByText('Usage heatmap 7')).toBeInTheDocument());
  });

  it('collapses the usage card on a definitive failure instead of a void', async () => {
    mocks.getSessionActivity.mockRejectedValue(new Error('404'));

    render(
      <MemoryRouter>
        <SessionInsights />
      </MemoryRouter>
    );

    await waitFor(() => expect(screen.queryByText('Loading usage')).not.toBeInTheDocument());
    expect(screen.queryByText(/Usage heatmap/)).not.toBeInTheDocument();
  });
});
