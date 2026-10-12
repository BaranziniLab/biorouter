import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import MessageDivergeLink from './MessageDivergeLink';
import { transcriptCopy } from './transcript/copy';

/** Open the message's ⋯ menu and choose "Diverge from here". */
async function divergeFromHere() {
  fireEvent.pointerDown(screen.getByRole('button', { name: transcriptCopy.message.more }), {
    button: 0,
    ctrlKey: false,
  });
  fireEvent.click(
    await screen.findByRole('menuitem', { name: transcriptCopy.message.divergeFromHere })
  );
}

const mockDivergeSession = vi.fn();
vi.mock('../api', () => ({
  divergeSession: (...args: unknown[]) => mockDivergeSession(...args),
}));

const mockToastError = vi.fn();
vi.mock('../toasts', () => ({
  toastError: (...args: unknown[]) => mockToastError(...args),
}));
vi.mock('../utils/sessionListCache', () => ({ notifySessionListChanged: vi.fn() }));

// Stub the Electron bridge used to open a new window.
const mockCreateChatWindow = vi.fn();
const mockCreateDivergedChatWindow = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  // @ts-expect-error test shim
  window.electron = {
    createChatWindow: mockCreateChatWindow,
    createDivergedChatWindow: mockCreateDivergedChatWindow,
  };
});

describe('MessageDivergeLink', () => {
  it('opens a new desktop window for the branch', async () => {
    mockDivergeSession.mockResolvedValue({
      data: { sessionId: '20260622_9', workingDir: '/home/u/proj', name: 'Thread (branch 1)' },
    });

    render(<MessageDivergeLink sessionId="20260622_1" />);
    await divergeFromHere();

    await waitFor(() => {
      expect(mockDivergeSession).toHaveBeenCalledWith({
        path: { session_id: '20260622_1' },
        body: {},
        // Issue #56 DR-19: a branch inherits the source chat's provider, so the
        // renderer proves the request came from the user. Empty here because
        // this harness has no bridge to mint a key from.
        headers: {},
        throwOnError: true,
      });
    });
    // New window opened with the diverged session id and its canonical branch
    // name, in pair view.
    expect(mockCreateDivergedChatWindow).toHaveBeenCalledWith(
      '/home/u/proj',
      '20260622_9',
      'Thread (branch 1)'
    );
    expect(mockCreateChatWindow).not.toHaveBeenCalled();
  });

  it('passes the durable message id and timestamp so the branch ends at this answer', async () => {
    mockDivergeSession.mockResolvedValue({
      data: { sessionId: '20260622_9', workingDir: '/home/u/proj' },
    });

    render(
      <MessageDivergeLink
        sessionId="20260622_1"
        truncateAfterMs={1717171717000}
        truncateAfterId="assistant-message-1"
      />
    );
    await divergeFromHere();

    await waitFor(() => {
      expect(mockDivergeSession).toHaveBeenCalledWith({
        path: { session_id: '20260622_1' },
        body: {
          truncateAfter: 1717171717000,
          truncateAfterId: 'assistant-message-1',
        },
        headers: {},
        throwOnError: true,
      });
    });
  });

  it('shows an error toast and opens nothing when the backend fails', async () => {
    mockDivergeSession.mockRejectedValue(new Error('boom'));

    render(<MessageDivergeLink sessionId="20260622_1" />);
    await divergeFromHere();

    await waitFor(() => expect(mockToastError).toHaveBeenCalled());
    expect(mockCreateChatWindow).not.toHaveBeenCalled();
    expect(mockCreateDivergedChatWindow).not.toHaveBeenCalled();
  });

  it('errors (and opens nothing) if the response lacks a session id', async () => {
    mockDivergeSession.mockResolvedValue({ data: { workingDir: '/x' } });

    render(<MessageDivergeLink sessionId="20260622_1" />);
    await divergeFromHere();

    await waitFor(() => expect(mockToastError).toHaveBeenCalled());
    expect(mockCreateChatWindow).not.toHaveBeenCalled();
    expect(mockCreateDivergedChatWindow).not.toHaveBeenCalled();
  });

  it('ignores rapid double-clicks (only one diverge in flight)', async () => {
    let resolve!: (v: unknown) => void;
    mockDivergeSession.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      })
    );

    render(<MessageDivergeLink sessionId="20260622_1" />);
    // The menu closes on the first choice; reopening it while the first branch
    // is in flight finds the item disabled and saying so, so a second choice
    // does nothing.
    const user = userEvent.setup();
    await divergeFromHere();
    await user.click(screen.getByRole('button', { name: transcriptCopy.message.more }));
    const item = await screen.findByRole('menuitem', { name: transcriptCopy.message.diverging });
    expect(item).toHaveAttribute('aria-disabled', 'true');
    await user.click(item);
    await user.keyboard('{Escape}');

    await waitFor(() => expect(mockDivergeSession).toHaveBeenCalledTimes(1));
    resolve({ data: { sessionId: '20260622_9', workingDir: '/x' } });
    await waitFor(() => expect(mockCreateDivergedChatWindow).toHaveBeenCalledTimes(1));
    expect(mockDivergeSession).toHaveBeenCalledTimes(1);
    expect(mockCreateChatWindow).not.toHaveBeenCalled();
  });
});
