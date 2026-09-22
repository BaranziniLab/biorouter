import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CopilotSetup } from './CopilotSetup';

const mocks = vi.hoisted(() => ({ setup: vi.fn() }));
vi.mock('./copilotApi', () => ({ copilotSetup: mocks.setup }));
beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, { electron: { platform: 'darwin', getConfig: () => ({}) } });
});

describe('CopilotSetup', () => {
  it('distinguishes a present runtime from unverified OS permissions', async () => {
    mocks.setup.mockResolvedValue({
      status: 'ready',
      runtime_version: '0.3.5',
      permissions: 'unknown',
      host: 'server-host',
      target: 'darwin-arm64',
    });
    render(<CopilotSetup />);
    expect(mocks.setup).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    await screen.findByText('Runtime ready');
    expect(screen.getByText(/OS permissions: Not checked/)).toBeVisible();
    expect(screen.getByText(/server-host/)).toBeVisible();
  });

  it('shows an actionable missing runtime result from the backend', async () => {
    mocks.setup.mockResolvedValue({
      status: 'missing_runtime',
      permissions: 'unknown',
      error: 'Payload not found on backend',
    });
    render(<CopilotSetup />);
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    await screen.findByText('Bundled runtime missing');
    expect(screen.getByText('Payload not found on backend')).toBeVisible();
    expect(screen.getByText(/Install or repair the matching Biorouter package/)).toBeVisible();
  });
  it.each([
    ['probe_pending', 'Runtime found — setup not checked'],
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
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    await screen.findByText(label);
    expect(
      screen.getByText(/Accessibility: allowed · Screen Recording: not allowed/)
    ).toBeVisible();
    expect(screen.getByText('Screen Recording permission is required.')).toBeVisible();
    expect(screen.getByText(/System Settings/)).toBeVisible();
  });

  it.each([
    'Enable Accessibility for BioRouter Computer Use and Screen Recording for Biorouter in System Settings.',
    'Enable Accessibility and Screen Recording for BioRouter Computer Use in System Settings.',
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
      fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));

      expect(await screen.findByText(message)).toBeVisible();
      expect(screen.getAllByText(/Biorouter Computer Use/i)).toHaveLength(1);
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
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));

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
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    await screen.findByText('OS permission required');
    fireEvent.click(screen.getByRole('button', { name: 'Check again' }));
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
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    await screen.findByText('Runtime ready');
    // Hiding is the control the user could not find at all before this.
    fireEvent.click(screen.getByRole('button', { name: /Hide Biorouter Copilot setup/ }));
    expect(screen.queryByText('Runtime ready')).toBeNull();
    // Re-showing a result already fetched must not re-ask the backend; that is
    // what "Check again" inside the panel is for.
    fireEvent.click(screen.getByRole('button', { name: /Show Biorouter Copilot setup/ }));
    expect(await screen.findByText('Runtime ready')).toBeVisible();
    expect(mocks.setup).toHaveBeenCalledTimes(1);
  });

  it('reports the outcome of a re-check that changes nothing on screen', async () => {
    mocks.setup.mockResolvedValue({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CopilotSetup />);
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    expect(await screen.findByText('All OS permissions are allowed.')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Check again' }));
    await waitFor(() => expect(mocks.setup).toHaveBeenCalledTimes(2));
    expect(screen.getByText('All OS permissions are allowed.')).toBeVisible();
  });
  it('shows the checks after a failed first attempt without a third click', async () => {
    mocks.setup.mockRejectedValueOnce(new Error('backend down')).mockResolvedValueOnce({
      status: 'ready',
      permissions: { accessibility: true, screen_recording: true },
    });
    render(<CopilotSetup />);
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    await screen.findByRole('alert');
    // Second press: the check succeeds, so its results must be VISIBLE. Flipping
    // `open` in the click handler left the toggle a click out of phase here, so
    // the panel stayed hidden and the button just silently changed its label.
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    expect(await screen.findByText('Runtime ready')).toBeVisible();
  });
});

describe('permission repair controls', () => {
  it('keeps probe failure separate from permission denial and shows recovery', async () => {
    mocks.setup.mockResolvedValue({
      status: 'probe_failed',
      permissions: 'unknown',
      target: 'darwin-arm64',
      message: 'native helper timed out',
    });
    render(<CopilotSetup />);
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    expect(await screen.findByText(/this does not mean access was denied/)).toBeVisible();
    expect(screen.getByText('native helper timed out')).toBeVisible();
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
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
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
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
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
    fireEvent.click(screen.getByRole('button', { name: 'Check Biorouter Copilot setup' }));
    expect(await screen.findByText(/If it is remote, changing permissions/)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Open Accessibility settings' })).toBeNull();
    expect(openSettings).not.toHaveBeenCalled();
  });
});
