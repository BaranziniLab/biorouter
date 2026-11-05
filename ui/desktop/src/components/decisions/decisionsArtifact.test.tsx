import { render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ page: vi.fn(), detail: vi.fn(), summary: vi.fn() }));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  fetchDecisionPage: mocks.page,
  fetchDecisionDetail: mocks.detail,
  fetchDecisionSummary: mocks.summary,
}));

import { ThemeProvider } from '../../contexts/ThemeContext';
import { decisionHistoryArtifact } from '../../utils/sessionDecisions';
import { ENTITY_ICONS } from '../icons/entity-icons';
import ArtifactViewer, { iconForArtifact } from '../artifacts/ArtifactViewer';

/**
 * The decision history is a tab of the artifact side panel (kind `decisions`):
 * one tab per chat, its glyph the decision mark, no "open externally" (there is
 * nothing outside the app to open it in), and the count in the preview bar.
 */
afterEach(() => vi.clearAllMocks());

describe('the decisions kind in the artifact side panel', () => {
  it('draws the decision mark on its tab', () => {
    expect(iconForArtifact(decisionHistoryArtifact('s1'))).toBe(ENTITY_ICONS.decision);
  });

  it('renders the decision history for the chat it names, filtered', async () => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      value: { embeddedBrowser: { isManagedAppUrl: vi.fn(async () => false) } },
    });
    mocks.page.mockResolvedValue({ records: [], nextCursor: null, total: 0 });
    mocks.summary.mockResolvedValue({ data: null, missing: true });
    render(
      <ThemeProvider>
        <ArtifactViewer
          artifact={decisionHistoryArtifact('s1', { origin: 'monitor', originId: 'form' })}
          sessionId="s1"
          onClose={vi.fn()}
          onOpenArtifact={vi.fn()}
        />
      </ThemeProvider>
    );
    const panel = screen.getByTestId('artifact-viewer');
    expect(await within(panel).findByTestId('decision-history')).toBeInTheDocument();
    await waitFor(() => expect(mocks.page).toHaveBeenCalled());
    expect(mocks.page.mock.calls[0][0]).toBe('s1');
    expect(mocks.page.mock.calls[0][1]).toMatchObject({ origin: 'monitor', originId: 'form' });
    expect(within(panel).getByRole('tab', { name: 'Decisions' })).toBeInTheDocument();
    expect(within(panel).queryByRole('button', { name: /open externally|open in/i })).toBeNull();
    expect(
      await within(panel).findByText('No decision calls match these filters.')
    ).toBeInTheDocument();
    expect(within(panel).getByTestId('decision-history-count')).toHaveTextContent('0 calls');
  });
});
