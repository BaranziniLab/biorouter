import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import MessageDivergeLink from './MessageDivergeLink';

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
    fireEvent.click(screen.getByRole('button', { name: /diverge/i }));

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
    fireEvent.click(screen.getByRole('button', { name: /diverge/i }));

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
    fireEvent.click(screen.getByRole('button', { name: /diverge/i }));

    await waitFor(() => expect(mockToastError).toHaveBeenCalled());
    expect(mockCreateChatWindow).not.toHaveBeenCalled();
    expect(mockCreateDivergedChatWindow).not.toHaveBeenCalled();
  });

  it('errors (and opens nothing) if the response lacks a session id', async () => {
    mockDivergeSession.mockResolvedValue({ data: { workingDir: '/x' } });

    render(<MessageDivergeLink sessionId="20260622_1" />);
    fireEvent.click(screen.getByRole('button', { name: /diverge/i }));

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
    const btn = screen.getByRole('button', { name: /diverge/i });
    fireEvent.click(btn);
    fireEvent.click(btn);
    fireEvent.click(btn);

    // Issue #56 DR-19: the renderer mints the user-action header before it
    // calls the API, so the request now lands a microtask after the click
    // instead of during it. The guard itself is unchanged — clicks 2 and 3 hit
    // a button React has already disabled — and the re-assertion after the
    // branch opens is what still holds that, rather than `waitFor` merely
    // catching the first of three calls.
    await waitFor(() => expect(mockDivergeSession).toHaveBeenCalledTimes(1));
    resolve({ data: { sessionId: '20260622_9', workingDir: '/x' } });
    await waitFor(() => expect(mockCreateDivergedChatWindow).toHaveBeenCalledTimes(1));
    expect(mockDivergeSession).toHaveBeenCalledTimes(1);
    expect(mockCreateChatWindow).not.toHaveBeenCalled();
  });
});
