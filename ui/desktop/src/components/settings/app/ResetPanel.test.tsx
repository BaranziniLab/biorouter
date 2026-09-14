import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ResetPanel from './ResetPanel';
import { RESET_NEEDS_HOST_REASON } from './resetOnBrowser';
import { BROWSER_SURFACE_MARKER } from '../../../utils/surface';

const mocks = vi.hoisted(() => ({
  previewReset: vi.fn(),
  resetAppData: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
  userActionHeaders: vi.fn(),
}));

vi.mock('../../../api', () => ({
  previewReset: mocks.previewReset,
  resetAppData: mocks.resetAppData,
}));

vi.mock('../../../toasts', () => ({
  toastService: {
    success: mocks.toastSuccess,
    error: mocks.toastError,
  },
}));

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: mocks.userActionHeaders,
}));

/** What the desktop's `userActionHeaders()` resolves to: the person's proof. */
const PROOF = { 'X-User-Action': 'desktop-user-action-key' };

describe('ResetPanel', () => {
  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mocks.userActionHeaders.mockResolvedValue(PROOF);
    mocks.previewReset.mockResolvedValue({
      data: {
        counts: {
          applications: 2,
          knowledgeBases: 3,
          skills: 4,
          extensions: 1,
          schedules: 5,
          workflows: 6,
          conversations: 12,
        },
      },
    });
    mocks.resetAppData.mockResolvedValue({ data: { reset: [], removed: {} } });
  });

  it('shows live counts and requires a deliberate selection for partial reset', async () => {
    render(<ResetPanel />);

    expect(await screen.findByText('2 built')).toBeInTheDocument();
    const resetSelected = screen.getByRole('button', { name: 'Reset selected' });
    expect(resetSelected).toBeDisabled();
    expect(
      screen.queryByText('Delete every app created with Agent Drafter.')
    ).not.toBeInTheDocument();

    const applicationDetails = screen.getByRole('button', {
      name: 'Show details for Built apps',
    });
    expect(applicationDetails).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(applicationDetails);
    expect(applicationDetails).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Delete every app created with Agent Drafter.')).toBeVisible();

    const workflowDetails = screen.getByRole('button', { name: 'Show details for Workflows' });
    fireEvent.click(workflowDetails);
    expect(applicationDetails).toHaveAttribute('aria-expanded', 'false');
    expect(
      screen.queryByText('Delete every app created with Agent Drafter.')
    ).not.toBeInTheDocument();
    expect(
      screen.getByText('Remove managed workflows and restore the Meditation workflow.')
    ).toBeVisible();

    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Built apps for reset' }));
    expect(resetSelected).toBeEnabled();
    fireEvent.click(resetSelected);

    expect(screen.getByRole('dialog')).toHaveTextContent('Reset selected data?');
    expect(screen.getByRole('dialog')).toHaveTextContent('Built apps');
    expect(screen.getByRole('dialog')).toHaveTextContent(
      'models, provider credentials, theme, and app preferences'
    );
  });

  it('submits only the selected categories and refreshes the preview', async () => {
    const onReset = vi.fn();
    render(<ResetPanel onReset={onReset} />);
    await screen.findByText('12 chats');

    fireEvent.click(
      screen.getByRole('checkbox', { name: 'Select Chat & usage history for reset' })
    );
    fireEvent.click(screen.getByRole('button', { name: 'Reset selected' }));
    fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Reset selected' })
    );

    await waitFor(() => {
      expect(mocks.resetAppData).toHaveBeenCalledWith({
        body: { categories: ['history'] },
        headers: PROOF,
        throwOnError: true,
      });
    });
    await waitFor(() => expect(onReset).toHaveBeenCalledWith(['history']));
    expect(mocks.previewReset).toHaveBeenCalledTimes(2);
    expect(await screen.findByText(/Reset complete/)).toBeInTheDocument();
  });

  /**
   * The daemon answers `GET /reset/preview` and `POST /reset` only for a request
   * carrying the person's proof — a caller holding just the daemon secret had
   * been able to empty History, private chats included. Without the header the
   * desktop's own panel would show no counts and refuse every reset.
   */
  it('sends the user-action proof on the preview and on the reset', async () => {
    render(<ResetPanel />);
    await screen.findByText('12 chats');
    expect(mocks.previewReset).toHaveBeenCalledWith({ headers: PROOF, throwOnError: true });

    fireEvent.click(screen.getByRole('checkbox', { name: 'Select Workflows for reset' }));
    fireEvent.click(screen.getByRole('button', { name: 'Reset selected' }));
    fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Reset selected' })
    );

    await waitFor(() => expect(mocks.resetAppData).toHaveBeenCalledTimes(1));
    expect(mocks.resetAppData.mock.calls[0][0].headers).toEqual(PROOF);
    for (const call of mocks.previewReset.mock.calls) {
      expect(call[0].headers).toEqual(PROOF);
    }
  });

  /**
   * Under `throwOnError` the client throws the parsed JSON body, not an `Error`,
   * so the refusal's own sentence has to be read off `message` — it used to be
   * replaced with the generic fallback.
   */
  it("shows the daemon's own sentence when a reset is refused", async () => {
    const refusal =
      'This daemon was started without a user-action key, so it cannot verify that a request ' +
      'came from the person at the keyboard.';
    mocks.resetAppData.mockRejectedValueOnce({ message: refusal });
    render(<ResetPanel />);
    await screen.findByText('12 chats');

    fireEvent.click(screen.getByRole('button', { name: 'Reset everything' }));
    fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Reset everything' })
    );

    // The toast is what the person sees: a failed reset leaves the confirm dialog
    // open, which marks the panel's own status line behind it aria-hidden.
    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({ title: 'Reset failed', msg: refusal })
    );
    expect(screen.getByRole('status', { hidden: true })).toHaveTextContent(refusal);
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  /**
   * SD-8 on `biorouter serve`: that daemon holds no key, so it refuses every
   * reset. The panel says so in place of its controls, before the person can
   * select, confirm, and only then read a refusal — and it never asks for the
   * preview it would be refused.
   */
  it('explains before the click on a browser-served page, and never asks', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(<ResetPanel />);

    expect(await screen.findByTestId('reset-needs-host-note')).toHaveTextContent(
      RESET_NEEDS_HOST_REASON
    );
    expect(screen.queryByRole('button', { name: 'Reset selected' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Reset everything' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Select all' })).not.toBeInTheDocument();
    expect(
      screen.getByRole('checkbox', { name: 'Select Chat & usage history for reset' })
    ).toBeDisabled();
    // The categories still say what a reset covers.
    fireEvent.click(screen.getByRole('button', { name: 'Show details for Chat & usage history' }));
    expect(
      screen.getByText('Clear every chat, token meter, cost total, and checkpoint.')
    ).toBeVisible();

    expect(mocks.previewReset).not.toHaveBeenCalled();
    expect(mocks.resetAppData).not.toHaveBeenCalled();
    expect(mocks.userActionHeaders).not.toHaveBeenCalled();
  });
});
