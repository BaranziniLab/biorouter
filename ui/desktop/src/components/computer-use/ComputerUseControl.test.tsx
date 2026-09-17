import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ComputerUseControl } from './ComputerUseControl';
import type { ComputerUseStatus } from './computerUseApi';

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
  decision: vi.fn(),
  setup: vi.fn(),
  browser: false,
}));
vi.mock('./computerUseApi', () => ({
  computerUseStatus: mocks.status,
  computerUseDecision: mocks.decision,
  computerUseSetup: mocks.setup,
}));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));

const status: ComputerUseStatus = {
  session_id: 'task-a',
  provider: 'public-provider',
  model: 'model-a',
  destination: 'https://provider.example/api',
  target: 'backend-host',
  disclosure: '',
  state: 'approval_required',
  challenge_id: 'challenge-a',
  public_model: true,
  handoff_required: false,
  requested: true,
  enabled: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.browser = false;
  mocks.status.mockResolvedValue(status);
  mocks.decision.mockResolvedValue({ ...status, requested: false, state: 'active' });
});
afterEach(() => vi.useRealTimers());

describe('ComputerUseControl', () => {
  it('discloses destination and host before a public grant and retains a working Stop', async () => {
    render(<ComputerUseControl sessionId="task-a" />);
    const allow = await screen.findByRole('button', { name: 'Allow control and sharing' });
    expect(screen.getByText(status.destination, { exact: false })).toBeVisible();
    expect(screen.getByText(status.target)).toBeVisible();
    expect(screen.getByText(/including sensitive information/)).toBeVisible();
    expect(mocks.decision).not.toHaveBeenCalled();
    fireEvent.click(allow);
    await screen.findByText('Computer use active');
    expect(mocks.decision).toHaveBeenCalledWith(status, 'consent', '');
    expect(screen.queryByRole('button', { name: 'Allow control and sharing' })).toBeNull();
    mocks.decision.mockResolvedValueOnce({ ...status, state: 'stopped', requested: false });
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await screen.findByRole('button', { name: 'Set up' });
    expect(mocks.decision.mock.calls[1][1]).toBe('revoke');
  });

  it('uses the canonical private disclosure once and exposes an explicit task grant', async () => {
    const disclosure =
      'Allow this private deployment? Content left open by another task may be visible.';
    mocks.status.mockResolvedValue({
      ...status,
      disclosure,
      public_model: false,
      handoff_required: true,
    });
    render(<ComputerUseControl sessionId="task-a" />);
    await screen.findByRole('button', { name: 'Allow for this task' });
    expect(screen.getAllByText(disclosure)).toHaveLength(1);
    expect(mocks.decision).not.toHaveBeenCalled();
  });

  it('requires the terminal passphrase in a browser and names the backend target', async () => {
    mocks.browser = true;
    render(<ComputerUseControl sessionId="task-a" />);
    const allow = await screen.findByRole('button', { name: 'Allow control and sharing' });
    expect(allow).toBeDisabled();
    expect(screen.getByText(/computer running Biorouter/)).toBeVisible();
    fireEvent.change(screen.getByLabelText('Computer Use approval key', { exact: false }), {
      target: { value: 'human-passphrase' },
    });
    fireEvent.click(allow);
    await waitFor(() =>
      expect(mocks.decision).toHaveBeenCalledWith(status, 'consent', 'human-passphrase')
    );
  });

  it('clears the passphrase after approval and never carries it into another chat', async () => {
    mocks.browser = true;
    const { rerender } = render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.change(await screen.findByLabelText('Computer Use approval key', { exact: false }), {
      target: { value: 'human-passphrase' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Allow control and sharing' }));
    await screen.findByText('Computer use active');
    mocks.status.mockResolvedValue({ ...status, session_id: 'task-b' });
    rerender(<ComputerUseControl sessionId="task-b" />);
    expect(await screen.findByLabelText('Computer Use approval key', { exact: false })).toHaveValue(
      ''
    );
    expect(screen.getByRole('button', { name: 'Allow control and sharing' })).toBeDisabled();
  });

  it('keeps acknowledgement available after a refused or stale challenge', async () => {
    mocks.decision.mockRejectedValue(
      new Error('The model destination changed. Review the new destination.')
    );
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Allow control and sharing' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('destination changed');
    expect(screen.queryByText('Computer use active')).toBeNull();
    expect(screen.getByRole('button', { name: 'Allow control and sharing' })).toBeEnabled();
  });

  it('does not let a pre-consent poll overwrite an acknowledged grant', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    let resolvePoll!: (value: ComputerUseStatus) => void;
    mocks.status.mockResolvedValueOnce(status).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolvePoll = resolve;
        })
    );
    render(<ComputerUseControl sessionId="task-a" />);
    const allow = await screen.findByRole('button', { name: 'Allow control and sharing' });
    await act(async () => vi.advanceTimersByTime(2000));
    fireEvent.click(allow);
    await screen.findByText('Computer use active');
    await act(async () => resolvePoll(status));
    expect(screen.getByRole('button', { name: 'Stop' })).toBeVisible();
  });

  it('keeps an approved task active across polls without prompting or approving again', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Allow control and sharing' }));
    await screen.findByText('Computer use active');
    mocks.status.mockResolvedValue({ ...status, state: 'active', requested: false });
    await act(async () => vi.advanceTimersByTime(2000));
    await act(async () => vi.advanceTimersByTime(2000));
    expect(screen.getByRole('button', { name: 'Stop' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Allow control and sharing' })).toBeNull();
    expect(mocks.decision).toHaveBeenCalledTimes(1);
  });

  it('ignores late status responses from a previously displayed chat', async () => {
    let resolveOld!: (value: ComputerUseStatus) => void;
    mocks.status.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveOld = resolve;
        })
    );
    const { rerender } = render(<ComputerUseControl sessionId="task-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalledWith('task-a'));
    mocks.status.mockResolvedValue({ ...status, session_id: 'task-b', model: 'model-b' });
    rerender(<ComputerUseControl sessionId="task-b" />);
    await screen.findByText(/model-b/);
    await act(async () => resolveOld({ ...status, state: 'active' }));
    expect(screen.getByText(/model-b/)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Stop' })).toBeNull();
    expect(screen.queryByText(/model-a/)).toBeNull();
  });

  it('does not interrupt a fresh chat with unsolicited consent', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false });
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Set up' }));
    expect(screen.getByText(/Ask Biorouter to use the computer/)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Allow control and sharing' })).toBeNull();
    expect(mocks.decision).not.toHaveBeenCalled();
  });

  it('checks pending OS permissions from the chat without granting idle control', async () => {
    mocks.status.mockResolvedValue({
      ...status,
      requested: false,
      runtime: { status: 'probe_pending', permissions: 'unknown' },
    });
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      permissions: { accessibility: false, screen_recording: false },
      message: 'Grant Accessibility permission.',
    });
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Set up' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check OS permissions' }));
    await screen.findByText('OS permission required');
    expect(screen.getByText('Grant Accessibility permission.')).toBeVisible();
    expect(mocks.setup).toHaveBeenCalledTimes(1);
    expect(mocks.decision).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Allow control and sharing' })).toBeNull();
  });

  it('hides a disabled capability without granting or re-enabling it', async () => {
    mocks.status.mockResolvedValue({ ...status, enabled: false });
    const { container } = render(<ComputerUseControl sessionId="task-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
    expect(mocks.decision).not.toHaveBeenCalled();
  });
});
