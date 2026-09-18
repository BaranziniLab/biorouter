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
    await screen.findByRole('button', { name: 'Show Computer Use details' });
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
    fireEvent.click(await screen.findByRole('button', { name: 'Show Computer Use details' }));
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
    fireEvent.click(await screen.findByRole('button', { name: 'Show Computer Use details' }));
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
  it('collapses again from the chevron and keeps the panel on its own surface', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false });
    render(<ComputerUseControl sessionId="task-a" />);
    const open = await screen.findByRole('button', { name: 'Show Computer Use details' });
    expect(open).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(open);
    expect(screen.getByText(/Ask Biorouter to use the computer/)).toBeVisible();
    const close = screen.getByRole('button', { name: 'Hide Computer Use details' });
    expect(close).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(close);
    // The container stays mounted and is hidden, so aria-controls always
    // resolves to a real element. Assert VISIBILITY, not presence.
    expect(screen.getByText(/Ask Biorouter to use the computer/)).not.toBeVisible();
    // jsdom applies no Tailwind and computes no layout, so this asserts the
    // TOKEN CHOICE that separates the panel from the chat canvas, not the
    // painted pixel. The tokens themselves are audited by check-contrast.mjs.
    const panel = screen.getByRole('region', { name: 'Computer Use' });
    expect(panel.className).toContain('bg-background-muted');
    expect(panel.className).toContain('border-border-subtle');
    expect(panel.className).toContain('rounded-container');
  });

  it('withholds the collapse control while an approval is pending', async () => {
    render(<ComputerUseControl sessionId="task-a" />);
    await screen.findByRole('button', { name: 'Allow control and sharing' });
    // The Allow button lives inside the details block; a chevron that could hide
    // it would be a control that hides the decision it is waiting for.
    expect(screen.queryByRole('button', { name: /Computer Use details/ })).toBeNull();
  });

  it('confirms a permission check that returns exactly what was already shown', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false });
    mocks.setup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Computer Use details' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check OS permissions' }));
    // The runtime detail is unchanged by the check, so the explicit result line
    // is the ONLY evidence the click did anything. That is the defect this pins.
    expect(await screen.findByText('All OS permissions are allowed.')).toBeVisible();
  });

  it('does not tell a fully granted machine to review its permissions', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false });
    mocks.setup.mockResolvedValue({
      status: 'ready',
      target: 'darwin-arm64',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Computer Use details' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check OS permissions' }));
    await screen.findByText('All OS permissions are allowed.');
    expect(
      screen.queryByText(/Review Accessibility and Screen Recording in System Settings/)
    ).toBeNull();
    expect(screen.getByText(/Nothing further to set up/)).toBeVisible();
  });
  it('keeps Stop working while a permission check is in flight', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false, state: 'active' });
    // A probe that never settles: the decision must not wait on it.
    mocks.setup.mockImplementation(() => new Promise(() => {}));
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Computer Use details' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check OS permissions' }));
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    // Stop is the safety control of a desktop-control feature. Sharing the probe
    // and the decision flag made this click a silent no-op.
    await waitFor(() => expect(mocks.decision).toHaveBeenCalledTimes(1));
  });

  it('never shows a permission verdict that contradicts the detail beside it', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false });
    mocks.setup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<ComputerUseControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Computer Use details' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check OS permissions' }));
    expect(await screen.findByText('All OS permissions are allowed.')).toBeVisible();
    // Someone revokes Accessibility; the 2s poll brings back a worse runtime.
    mocks.status.mockResolvedValue({
      ...status,
      requested: false,
      runtime: {
        status: 'os_permission_required',
        permissions: { accessibility: false, screen_recording: true },
      },
    });
    await waitFor(() => expect(screen.queryByText('All OS permissions are allowed.')).toBeNull(), {
      timeout: 4000,
    });
  });
});
