import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CopilotControl } from './CopilotControl';
import type { CopilotStatus } from './copilotApi';

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
  decision: vi.fn(),
  setup: vi.fn(),
  browser: false,
  // The component compares with `instanceof`, so the class it imports and the
  // class the test throws must be the SAME object. Declaring it here and
  // returning it from the factory is what guarantees that; importing the real
  // module would also drag in the generated SDK this suite deliberately avoids.
  NotApplicable: class CopilotNotApplicable extends Error {},
}));
vi.mock('./copilotApi', () => ({
  copilotStatus: mocks.status,
  copilotDecision: mocks.decision,
  copilotSetup: mocks.setup,
  CopilotNotApplicable: mocks.NotApplicable,
}));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));

const status: CopilotStatus = {
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
  activity_id: 'activity-a',
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.browser = false;
  mocks.status.mockResolvedValue(status);
  mocks.decision.mockResolvedValue({ ...status, requested: false, state: 'active' });
});
afterEach(() => vi.useRealTimers());

describe('CopilotControl', () => {
  it('renders nothing and stops polling when the chat mode forbids Biorouter Copilot', async () => {
    // `GET /agent/computer_use/status` answers 409 for a Chat-mode session
    // ("Chat mode does not run Biorouter Copilot tools"). That is a fact about the
    // chat, not a transient fault: re-asking can never change it.
    mocks.status.mockRejectedValue(
      new mocks.NotApplicable('Chat mode does not run Biorouter Copilot tools')
    );
    vi.useFakeTimers({ shouldAdvanceTime: true });

    const { container } = render(<CopilotControl sessionId="task-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalled());
    await waitFor(() => expect(container).toBeEmptyDOMElement());

    // Not an alert above the composer, and no Retry offering an action that
    // cannot work.
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();

    // And the 2 s poll is gone. Each surviving poll re-spawns the native
    // helper's PowerShell/UIA bridge once its 30 s cache expires, for a chat
    // that can never use it.
    const settled = mocks.status.mock.calls.length;
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(mocks.status.mock.calls.length).toBe(settled);
  });

  it('silently retries an initial failure without interrupting a chat with no known activity', async () => {
    // The negative control for the test above: if "not applicable" swallowed
    // every failure, a genuinely transient error would silently hide the panel
    // instead of offering Retry.
    mocks.status.mockRejectedValue(new Error('network down'));
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const { container } = render(<CopilotControl sessionId="task-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalledTimes(1));
    expect(container).toBeEmptyDOMElement();
    mocks.status.mockResolvedValue(status);
    await act(async () => vi.advanceTimersByTime(2000));
    expect(await screen.findByRole('button', { name: 'Allow control and sharing' })).toBeVisible();
  });

  it('discloses destination and host before a public grant and retains a working Stop', async () => {
    render(<CopilotControl sessionId="task-a" />);
    const allow = await screen.findByRole('button', { name: 'Allow control and sharing' });
    expect(screen.getByText(status.destination, { exact: false })).toBeVisible();
    expect(screen.getByText(status.target)).toBeVisible();
    expect(screen.getByText(/including sensitive information/)).toBeVisible();
    expect(mocks.decision).not.toHaveBeenCalled();
    fireEvent.click(allow);
    await screen.findByText('Biorouter Copilot active');
    expect(mocks.decision).toHaveBeenCalledWith(status, 'consent', '');
    expect(screen.queryByRole('button', { name: 'Allow control and sharing' })).toBeNull();
    mocks.decision.mockResolvedValueOnce({ ...status, state: 'stopped', requested: false });
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await screen.findByRole('button', { name: 'Show Biorouter Copilot details' });
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
    render(<CopilotControl sessionId="task-a" />);
    await screen.findByRole('button', { name: 'Allow for this task' });
    expect(screen.getAllByText(disclosure)).toHaveLength(1);
    expect(mocks.decision).not.toHaveBeenCalled();
  });

  it('requires the terminal passphrase in a browser and names the backend target', async () => {
    mocks.browser = true;
    render(<CopilotControl sessionId="task-a" />);
    const allow = await screen.findByRole('button', { name: 'Allow control and sharing' });
    expect(allow).toBeDisabled();
    expect(screen.getByText(/computer running Biorouter/)).toBeVisible();
    fireEvent.change(screen.getByLabelText('Biorouter Copilot approval key', { exact: false }), {
      target: { value: 'human-passphrase' },
    });
    fireEvent.click(allow);
    await waitFor(() =>
      expect(mocks.decision).toHaveBeenCalledWith(status, 'consent', 'human-passphrase')
    );
  });

  it('clears the passphrase after approval and never carries it into another chat', async () => {
    mocks.browser = true;
    const { rerender } = render(<CopilotControl sessionId="task-a" />);
    fireEvent.change(
      await screen.findByLabelText('Biorouter Copilot approval key', { exact: false }),
      {
        target: { value: 'human-passphrase' },
      }
    );
    fireEvent.click(screen.getByRole('button', { name: 'Allow control and sharing' }));
    await screen.findByText('Biorouter Copilot active');
    mocks.status.mockResolvedValue({ ...status, session_id: 'task-b' });
    rerender(<CopilotControl sessionId="task-b" />);
    expect(
      await screen.findByLabelText('Biorouter Copilot approval key', { exact: false })
    ).toHaveValue('');
    expect(screen.getByRole('button', { name: 'Allow control and sharing' })).toBeDisabled();
  });

  it('keeps acknowledgement available after a refused or stale challenge', async () => {
    mocks.decision.mockRejectedValue(
      new Error('The model destination changed. Review the new destination.')
    );
    render(<CopilotControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Allow control and sharing' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('destination changed');
    expect(screen.queryByText('Biorouter Copilot active')).toBeNull();
    expect(screen.getByRole('button', { name: 'Allow control and sharing' })).toBeEnabled();
  });

  it('does not let a pre-consent poll overwrite an acknowledged grant', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    let resolvePoll!: (value: CopilotStatus) => void;
    mocks.status.mockResolvedValueOnce(status).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolvePoll = resolve;
        })
    );
    render(<CopilotControl sessionId="task-a" />);
    const allow = await screen.findByRole('button', { name: 'Allow control and sharing' });
    await act(async () => vi.advanceTimersByTime(2000));
    fireEvent.click(allow);
    await screen.findByText('Biorouter Copilot active');
    await act(async () => resolvePoll(status));
    expect(screen.getByRole('button', { name: 'Stop' })).toBeVisible();
  });

  it('keeps an approved task active across polls without prompting or approving again', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    render(<CopilotControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Allow control and sharing' }));
    await screen.findByText('Biorouter Copilot active');
    mocks.status.mockResolvedValue({ ...status, state: 'active', requested: false });
    await act(async () => vi.advanceTimersByTime(2000));
    await act(async () => vi.advanceTimersByTime(2000));
    expect(screen.getByRole('button', { name: 'Stop' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Allow control and sharing' })).toBeNull();
    expect(mocks.decision).toHaveBeenCalledTimes(1);
  });

  it('ignores late status responses from a previously displayed chat', async () => {
    let resolveOld!: (value: CopilotStatus) => void;
    mocks.status.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveOld = resolve;
        })
    );
    const { rerender } = render(<CopilotControl sessionId="task-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalledWith('task-a'));
    mocks.status.mockResolvedValue({ ...status, session_id: 'task-b', model: 'model-b' });
    rerender(<CopilotControl sessionId="task-b" />);
    await screen.findByText(/model-b/);
    await act(async () => resolveOld({ ...status, state: 'active' }));
    expect(screen.getByText(/model-b/)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Stop' })).toBeNull();
    expect(screen.queryByText(/model-a/)).toBeNull();
  });

  it.each(['idle', 'stopped', 'approval_required', 'busy'])(
    'hides %s with no evidence of activity in this conversation',
    async (state) => {
      mocks.status.mockResolvedValue({
        ...status,
        activity_id: undefined,
        requested: false,
        state,
      });
      const { container } = render(<CopilotControl sessionId="task-a" />);
      await waitFor(() => expect(mocks.status).toHaveBeenCalled());
      expect(container).toBeEmptyDOMElement();
      expect(mocks.decision).not.toHaveBeenCalled();
    }
  );

  it('dismisses a stopped banner across polls and remounts, then resurfaces a new request', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const stopped = { ...status, session_id: 'dismiss-chat', requested: false, state: 'stopped' };
    mocks.status.mockResolvedValue(stopped);
    const view = render(<CopilotControl sessionId="dismiss-chat" />);
    fireEvent.click(
      await screen.findByRole('button', { name: 'Dismiss Biorouter Copilot banner' })
    );
    expect(view.container).toBeEmptyDOMElement();
    mocks.status.mockResolvedValue({ ...stopped, challenge_id: 'ordinary-reply' });
    await act(async () => vi.advanceTimersByTime(2000));
    expect(view.container).toBeEmptyDOMElement();
    view.unmount();
    const reopened = render(<CopilotControl sessionId="dismiss-chat" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalledTimes(3));
    expect(reopened.container).toBeEmptyDOMElement();
    mocks.status.mockResolvedValue({
      ...status,
      session_id: 'dismiss-chat',
      activity_id: 'new-request',
    });
    await act(async () => vi.advanceTimersByTime(2000));
    expect(await screen.findByRole('button', { name: 'Allow control and sharing' })).toBeVisible();
    expect(mocks.decision).not.toHaveBeenCalled();
  });

  it('resurfaces renewed requests from an older backend without activity identifiers', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const legacy = { ...status, session_id: 'legacy-dismiss', activity_id: undefined };
    mocks.status.mockResolvedValue(legacy);
    render(<CopilotControl sessionId="legacy-dismiss" />);
    fireEvent.click(
      await screen.findByRole('button', { name: 'Dismiss Biorouter Copilot banner' })
    );
    expect(screen.getByRole('button', { name: 'Show Biorouter Copilot request' })).toBeVisible();
    mocks.status.mockResolvedValue({ ...legacy, challenge_id: 'next-legacy-request' });
    await act(async () => vi.advanceTimersByTime(2000));
    expect(await screen.findByRole('button', { name: 'Allow control and sharing' })).toBeVisible();
    expect(mocks.decision).not.toHaveBeenCalled();
  });

  it('dismisses active details without revoking and retains a visible Stop control', async () => {
    mocks.status.mockResolvedValue({
      ...status,
      session_id: 'active-dismiss',
      requested: false,
      state: 'active',
    });
    render(<CopilotControl sessionId="active-dismiss" />);
    fireEvent.click(
      await screen.findByRole('button', { name: 'Dismiss Biorouter Copilot banner' })
    );
    expect(screen.queryByRole('region', { name: 'Biorouter Copilot' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Show active Biorouter Copilot' })).toBeVisible();
    expect(mocks.decision).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(mocks.decision).toHaveBeenCalledTimes(1));
    expect(mocks.decision.mock.calls[0][1]).toBe('revoke');
  });

  it('never carries observed activity into an unused chat when switching sessions', async () => {
    mocks.status.mockResolvedValue({ ...status, activity_id: undefined, state: 'active' });
    const view = render(<CopilotControl sessionId="task-a" />);
    await screen.findByText('Biorouter Copilot active');
    mocks.status.mockResolvedValue({
      ...status,
      session_id: 'unused-chat',
      activity_id: undefined,
      requested: false,
      state: 'stopped',
    });
    view.rerender(<CopilotControl sessionId="unused-chat" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalledWith('unused-chat'));
    expect(view.container).toBeEmptyDOMElement();
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
    render(<CopilotControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Biorouter Copilot details' }));
    fireEvent.click(screen.getByRole('button', { name: 'Check OS permissions' }));
    await screen.findByText('OS permission required');
    expect(screen.getByText('Grant Accessibility permission.')).toBeVisible();
    expect(mocks.setup).toHaveBeenCalledTimes(1);
    expect(mocks.decision).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Allow control and sharing' })).toBeNull();
  });

  it('hides a disabled capability without granting or re-enabling it', async () => {
    mocks.status.mockResolvedValue({ ...status, enabled: false });
    const { container } = render(<CopilotControl sessionId="task-a" />);
    await waitFor(() => expect(mocks.status).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
    expect(mocks.decision).not.toHaveBeenCalled();
  });
  it('collapses again from the chevron and keeps the panel on its own surface', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false });
    render(<CopilotControl sessionId="task-a" />);
    const open = await screen.findByRole('button', { name: 'Show Biorouter Copilot details' });
    expect(open).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(open);
    expect(screen.getByText(/Ask Biorouter to use the computer/)).toBeVisible();
    const close = screen.getByRole('button', { name: 'Hide Biorouter Copilot details' });
    expect(close).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(close);
    // The container stays mounted and is hidden, so aria-controls always
    // resolves to a real element. Assert VISIBILITY, not presence.
    expect(screen.getByText(/Ask Biorouter to use the computer/)).not.toBeVisible();
    // jsdom applies no Tailwind and computes no layout, so this asserts the
    // TOKEN CHOICE that separates the panel from the chat canvas, not the
    // painted pixel. The tokens themselves are audited by check-contrast.mjs.
    const panel = screen.getByRole('region', { name: 'Biorouter Copilot' });
    expect(panel.className).toContain('bg-background-muted');
    expect(panel.className).toContain('border-border-subtle');
    expect(panel.className).toContain('rounded-container');
  });

  it('withholds the collapse control while an approval is pending', async () => {
    render(<CopilotControl sessionId="task-a" />);
    await screen.findByRole('button', { name: 'Allow control and sharing' });
    // The Allow button lives inside the details block; a chevron that could hide
    // it would be a control that hides the decision it is waiting for.
    expect(screen.queryByRole('button', { name: /Biorouter Copilot details/ })).toBeNull();
  });

  it('confirms a permission check that returns exactly what was already shown', async () => {
    mocks.status.mockResolvedValue({ ...status, requested: false });
    mocks.setup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CopilotControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Biorouter Copilot details' }));
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
    render(<CopilotControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Biorouter Copilot details' }));
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
    render(<CopilotControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Biorouter Copilot details' }));
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
    render(<CopilotControl sessionId="task-a" />);
    fireEvent.click(await screen.findByRole('button', { name: 'Show Biorouter Copilot details' }));
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
