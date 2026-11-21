import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CopilotSetupPanel, CopilotSetupTrigger, useCopilotSetup } from './CopilotSetup';
import { copilotSetupCopy } from './copy';

const mocks = vi.hoisted(() => ({ setup: vi.fn(), request: vi.fn() }));
vi.mock('./copilotApi', () => ({
  copilotSetup: mocks.setup,
  copilotRequestPermissions: mocks.request,
}));
beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, { electron: { platform: 'darwin', getConfig: () => ({}) } });
});

/**
 * The Settings row puts the trigger beside a switch and the panel after the row; here they sit
 * together, which is all these tests need.
 */
function CopilotSetup() {
  const setup = useCopilotSetup();
  return (
    <>
      <CopilotSetupTrigger setup={setup} />
      <CopilotSetupPanel setup={setup} />
    </>
  );
}

const checkButton = () => screen.getByRole('button', { name: copilotSetupCopy.check });

describe('CopilotSetup', () => {
  it('distinguishes a present runtime from unverified OS permissions', async () => {
    mocks.setup.mockResolvedValue({
      status: 'ready',
      runtime_version: '1.3.1',
      permissions: 'unknown',
      host: 'server-host',
      target: 'darwin-arm64',
    });
    render(<CopilotSetup />);
    expect(mocks.setup).not.toHaveBeenCalled();
    fireEvent.click(checkButton());
    await screen.findByText('Runtime ready');
    expect(screen.getByText(/OS permissions: Not checked/)).toBeVisible();
    // Which runtime answered is the state line's InfoTip, not two lines of the panel.
    const about = screen.getByRole('button', { name: 'About this runtime' });
    expect(about).toHaveAccessibleDescription(
      'Backend computer: server-host. Runtime version: 1.3.1 · darwin-arm64.'
    );
    expect(screen.getByText('Runtime ready')).toContainElement(about);
  });

  // A real machine had three copies of the engine — an installed Biorouter, a
  // packaged build and a source build — with the SAME bundle id and the same
  // Developer ID, and macOS had granted Screen Recording to two of them. The
  // panel told the operator to allow Screen Recording when they already had,
  // for a different copy, and nothing on screen said which one was being probed.
  // The path is the only thing that disambiguates.
  // The copy named is the app System Settings lists and grants, Biorouter Copilot, never the
  // engine executable inside it.
  it('names the exact copy that needs the OS grant when one is missing', async () => {
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      runtime_version: '1.3.1',
      target: 'darwin-arm64',
      executable:
        '/Users/x/src/target/computer-use/darwin-arm64/Biorouter Copilot.app/Contents/MacOS/agent-desktop',
      app_bundle: '/Users/x/src/target/computer-use/darwin-arm64/Biorouter Copilot.app',
      permissions: { accessibility: true, screen_recording: false },
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText(/Screen Recording: not allowed/);
    const path = screen.getByText(
      '/Users/x/src/target/computer-use/darwin-arm64/Biorouter Copilot.app'
    );
    expect(path).toBeVisible();
    // Data read character by character, so it is set as code (design.md D-42).
    expect(path.tagName).toBe('CODE');
    expect(path).toHaveClass('font-mono', 'text-code');
    expect(screen.queryByText(/Contents\/MacOS/)).not.toBeInTheDocument();
  });

  // A development payload outside any app is granted by its own executable, so that is shown.
  it('names the executable on macOS only when no app holds it', async () => {
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      target: 'darwin-arm64',
      executable: '/Users/x/src/target/debug/engine',
      permissions: { accessibility: false, screen_recording: false },
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText(/Accessibility: not allowed/);
    expect(screen.getByText('/Users/x/src/target/debug/engine').tagName).toBe('CODE');
  });

  // Windows and Linux grant nothing per copy, so no path is named there.
  it('names no path where the system grants nothing per copy', async () => {
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      target: 'linux-x64',
      executable: '/usr/lib/biorouter/computer-use/linux-x64/engine',
      permissions: { accessibility: false, screen_recording: true },
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText(/Accessibility: not allowed/);
    expect(screen.queryByText(/Grant these to this exact copy/)).not.toBeInTheDocument();
  });

  // The negative control: a ready runtime must NOT nag about a path, or every
  // healthy panel grows a line of noise.
  it('does not name a path once permissions are allowed', async () => {
    mocks.setup.mockResolvedValue({
      status: 'ready',
      runtime_version: '1.3.1',
      target: 'darwin-arm64',
      executable:
        '/Applications/Biorouter.app/Contents/Resources/computer-use/x/Contents/MacOS/agent-desktop',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText(/Screen Recording: allowed/);
    expect(screen.queryByText(/Grant these to this exact copy/)).not.toBeInTheDocument();
  });

  it('shows an actionable missing runtime result from the backend', async () => {
    mocks.setup.mockResolvedValue({
      status: 'missing_runtime',
      permissions: 'unknown',
      error: 'Payload not found on backend',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText('Bundled runtime missing');
    expect(screen.getByText('Payload not found on backend')).toBeVisible();
    expect(screen.getByText(/Install or repair the matching Biorouter package/)).toBeVisible();
  });
  it.each([
    ['probe_pending', copilotSetupCopy.runtimeStates.probe_pending],
    ['os_permission_required', 'OS permission required'],
  ])('renders %s with an actionable permission result', async (status, label) => {
    mocks.setup.mockResolvedValue({
      status,
      permissions: { accessibility: true, screen_recording: false },
      message: 'Screen Recording permission is required.',
      target: 'aarch64-apple-darwin',
      desktop_available: true,
      capture_available: false,
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText(label);
    expect(
      screen.getByText(/Accessibility: allowed · Screen Recording: not allowed/)
    ).toBeVisible();
    expect(screen.getByText('Screen Recording permission is required.')).toBeVisible();
    expect(screen.getByText(/System Settings/)).toBeVisible();
  });

  it.each([
    'Allow Biorouter Copilot in System Settings (Accessibility, and Screen Recording for screenshots), then check again.',
    'Allow Biorouter Copilot in System Settings, then check again.',
  ])(
    'uses the native permission owners without contradictory static guidance: %s',
    async (message) => {
      mocks.setup.mockResolvedValue({
        status: 'os_permission_required',
        permissions: { accessibility: false, screen_recording: false },
        message,
        target: 'aarch64-apple-darwin',
      });
      render(<CopilotSetup />);
      fireEvent.click(checkButton());

      expect(await screen.findByText(message)).toBeVisible();
      // The backend's own sentence names the owner, so the static one stands down.
      expect(screen.getAllByText(/Allow Biorouter Copilot/)).toHaveLength(1);
      expect(screen.getByText('Use the app names in the permission details above.')).toBeVisible();
      // What to do if macOS asks to reopen the app is the InfoTip beside the macOS line.
      expect(
        screen.getByRole('button', { name: 'About allowing permissions on macOS' })
      ).toHaveAccessibleDescription(/Checking permissions does not grant them/);
      expect(
        screen.getByText(/Review Accessibility and Screen Recording in System Settings/)
      ).toHaveTextContent('on the backend computer, then check again.');
    }
  );

  it('shows interactive desktop guidance for the native win32-x64 target', async () => {
    mocks.setup.mockResolvedValue({
      status: 'desktop_unavailable',
      permissions: 'unknown',
      target: 'win32-x64',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());

    expect(await screen.findByText(/Use a signed-in interactive desktop/)).toHaveTextContent(
      'Secure desktops and elevation prompts cannot be controlled.'
    );
    expect(screen.queryByText(/Grant the operating-system permissions requested/)).toBeNull();
  });

  it('rechecks setup after permissions change instead of reusing the previous displayed result', async () => {
    mocks.setup
      .mockResolvedValueOnce({
        status: 'os_permission_required',
        permissions: { accessibility: false, screen_recording: false },
      })
      .mockResolvedValueOnce({
        status: 'ready',
        permissions: { accessibility: true, screen_recording: true },
      });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText('OS permission required');
    fireEvent.click(screen.getByRole('button', { name: copilotSetupCopy.checkAgain }));
    await screen.findByText('Runtime ready');
    expect(screen.getByText(/Accessibility: allowed · Screen Recording: allowed/)).toBeVisible();
    expect(mocks.setup).toHaveBeenCalledTimes(2);
  });
  it('can hide the checks again and re-show them without asking the backend', async () => {
    mocks.setup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByText('Runtime ready');
    // Hiding is the control the user could not find at all before this.
    fireEvent.click(screen.getByRole('button', { name: copilotSetupCopy.hide }));
    expect(screen.queryByText('Runtime ready')).toBeNull();
    // Re-showing a result already fetched must not re-ask the backend; that is
    // what "Check again" inside the panel is for.
    fireEvent.click(screen.getByRole('button', { name: copilotSetupCopy.show }));
    expect(await screen.findByText('Runtime ready')).toBeVisible();
    expect(mocks.setup).toHaveBeenCalledTimes(1);
  });

  it('reports the outcome of a re-check that changes nothing on screen', async () => {
    mocks.setup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    expect(await screen.findByText(copilotSetupCopy.verdicts.ready)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: copilotSetupCopy.checkAgain }));
    await waitFor(() => expect(mocks.setup).toHaveBeenCalledTimes(2));
    expect(screen.getByText(copilotSetupCopy.verdicts.ready)).toBeVisible();
  });
  it('shows the checks after a failed first attempt without a third click', async () => {
    mocks.setup.mockRejectedValueOnce(new Error('backend down')).mockResolvedValueOnce({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    await screen.findByRole('alert');
    // Second press: the check succeeds, so its results must be VISIBLE. Flipping
    // `open` in the click handler left the toggle a click out of phase here, so
    // the panel stayed hidden and the button just silently changed its label.
    fireEvent.click(checkButton());
    expect(await screen.findByText('Runtime ready')).toBeVisible();
  });
});

// A `disabled` button that held keyboard focus hands it to <body>: a person who pressed Enter on
// "Check setup" was thrown back to the top of the page (measured in Chromium). The buttons stay
// focusable and say they are busy instead, and a second press is ignored.
describe('a check in flight', () => {
  const ready = {
    status: 'ready',
    permissions: { accessibility: true, screen_recording: true },
  };

  it('keeps the row button focused and ignores a second press while it runs', async () => {
    let finish!: (value: unknown) => void;
    mocks.setup.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      })
    );
    render(<CopilotSetup />);
    const trigger = checkButton();
    trigger.focus();
    fireEvent.click(trigger);

    const busy = await screen.findByRole('button', { name: copilotSetupCopy.checking });
    expect(busy).toBe(trigger);
    expect(busy).toHaveAttribute('aria-busy', 'true');
    expect(busy).toHaveAttribute('aria-disabled', 'true');
    expect(busy).not.toBeDisabled();
    expect(document.activeElement).toBe(busy);
    fireEvent.click(busy);
    expect(mocks.setup).toHaveBeenCalledTimes(1);

    finish(ready);
    await screen.findByText('Runtime ready');
    expect(trigger).not.toHaveAttribute('aria-disabled');
    expect(document.activeElement).toBe(trigger);
  });

  it('keeps "Check again" focused and ignores a second press while it runs', async () => {
    mocks.setup.mockResolvedValueOnce(ready);
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    const again = await screen.findByRole('button', { name: copilotSetupCopy.checkAgain });

    let finish!: (value: unknown) => void;
    mocks.setup.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      })
    );
    again.focus();
    fireEvent.click(again);
    await waitFor(() => expect(again).toHaveAttribute('aria-busy', 'true'));
    expect(again).toHaveAttribute('aria-disabled', 'true');
    expect(again).not.toBeDisabled();
    expect(document.activeElement).toBe(again);
    fireEvent.click(again);
    expect(mocks.setup).toHaveBeenCalledTimes(2);

    finish(ready);
    await waitFor(() => expect(again).not.toHaveAttribute('aria-busy', 'true'));
    expect(document.activeElement).toBe(again);
  });
});

describe('permission repair controls', () => {
  // AD-04: the grants belong to Biorouter Copilot, never to Biorouter itself.
  it('names Biorouter Copilot, not Biorouter, as the app that holds the macOS grants', async () => {
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      permissions: { accessibility: false, screen_recording: false },
      target: 'darwin-arm64',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    expect(await screen.findByText(copilotSetupCopy.macGrantOwner)).toBeVisible();
    expect(copilotSetupCopy.macGrantOwner).toMatch(/Biorouter itself needs neither/);
  });

  it('asks macOS for access in one press on a local backend and shows the answer', async () => {
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        getConfig: () => ({ BIOROUTER_LOCAL_BACKEND: true }),
        openCopilotPermissionSettings: vi.fn(),
      },
    });
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      permissions: { accessibility: false, screen_recording: false },
      target: 'darwin-arm64',
    });
    mocks.request.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
      target: 'darwin-arm64',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    const request = await screen.findByRole('button', { name: copilotSetupCopy.request });
    expect(mocks.request).not.toHaveBeenCalled();
    expect(
      screen.getByRole('button', { name: 'About requesting access' })
    ).toHaveAccessibleDescription(/System Events, which Biorouter Copilot does not use/);
    fireEvent.click(request);
    expect(await screen.findByText(/Nothing further to set up/)).toBeVisible();
    expect(mocks.request).toHaveBeenCalledTimes(1);
    expect(mocks.setup).toHaveBeenCalledTimes(1);
  });

  it('reports a request the system did not finish and keeps the settings shortcuts', async () => {
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        getConfig: () => ({ BIOROUTER_LOCAL_BACKEND: true }),
        openCopilotPermissionSettings: vi.fn(),
      },
    });
    const blocked = {
      status: 'os_permission_required',
      permissions: { accessibility: false, screen_recording: false },
      target: 'darwin-arm64',
    };
    mocks.setup.mockResolvedValue(blocked);
    mocks.request.mockResolvedValue({
      ...blocked,
      request_error: 'The system prompts did not finish in time.',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    fireEvent.click(await screen.findByRole('button', { name: copilotSetupCopy.request }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The system prompts did not finish in time.'
    );
    expect(screen.getByRole('button', { name: 'Open Accessibility settings' })).toBeVisible();
  });

  it('keeps probe failure separate from permission denial and shows recovery', async () => {
    mocks.setup.mockResolvedValue({
      status: 'probe_failed',
      permissions: 'unknown',
      target: 'darwin-arm64',
      message: 'native helper timed out',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    expect(await screen.findByText(/this does not mean access was denied/)).toBeVisible();
    expect(screen.getByText('native helper timed out')).toBeVisible();
    // The failure's first line is in view; what to do about it is the InfoTip beside it.
    expect(
      screen.getByRole('button', { name: 'About recovering from a failed check' })
    ).toHaveAccessibleDescription(/Restart Biorouter on the backend computer and check again/);
    expect(screen.queryByText(/A required OS permission is missing/)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Open Accessibility settings' })).toBeNull();
  });

  it('opens only local macOS settings on an explicit click and reports failures', async () => {
    const openSettings = vi.fn().mockRejectedValue(new Error('Settings could not open'));
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        getConfig: () => ({ BIOROUTER_LOCAL_BACKEND: true }),
        openCopilotPermissionSettings: openSettings,
      },
    });
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      permissions: { accessibility: false, screen_recording: false },
      target: 'darwin-arm64',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    const button = await screen.findByRole('button', { name: 'Open Accessibility settings' });
    expect(openSettings).not.toHaveBeenCalled();
    fireEvent.click(button);
    expect(openSettings).toHaveBeenCalledWith('accessibility');
    expect(await screen.findByRole('alert')).toHaveTextContent('Settings could not open');
  });

  it('keeps local settings available when a probe fails without claiming they fix the runtime', async () => {
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        getConfig: () => ({ BIOROUTER_LOCAL_BACKEND: true }),
        openCopilotPermissionSettings: vi.fn(),
      },
    });
    mocks.setup.mockResolvedValue({
      status: 'probe_failed',
      permissions: 'unknown',
      target: 'darwin-arm64',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    expect(
      await screen.findByRole('button', { name: 'Open Screen Recording settings' })
    ).toBeVisible();
    expect(screen.getByText(/this does not mean access was denied/)).toBeVisible();
  });

  it('guides external backends without offering local settings', async () => {
    const openSettings = vi.fn();
    Object.assign(window, {
      electron: {
        platform: 'darwin',
        getConfig: () => ({ BIOROUTER_LOCAL_BACKEND: false }),
        openCopilotPermissionSettings: openSettings,
      },
    });
    mocks.setup.mockResolvedValue({
      status: 'os_permission_required',
      permissions: { accessibility: false, screen_recording: false },
      target: 'darwin-arm64',
    });
    render(<CopilotSetup />);
    fireEvent.click(checkButton());
    expect(await screen.findByText('Make these changes on the backend computer.')).toBeVisible();
    expect(
      screen.getByRole('button', { name: 'About a remote backend' })
    ).toHaveAccessibleDescription(/If it is remote, changing permissions/);
    expect(screen.queryByRole('button', { name: 'Open Accessibility settings' })).toBeNull();
    // A remote backend's prompts would appear on a computer nobody here is looking at.
    expect(screen.queryByRole('button', { name: copilotSetupCopy.request })).toBeNull();
    expect(openSettings).not.toHaveBeenCalled();
  });
});
