import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ComputerUseSetup } from './ComputerUseSetup';

const mocks = vi.hoisted(() => ({ setup: vi.fn() }));
vi.mock('./computerUseApi', () => ({ computerUseSetup: mocks.setup }));
beforeEach(() => vi.clearAllMocks());

describe('ComputerUseSetup', () => {
  it('distinguishes a present runtime from unverified OS permissions', async () => {
    mocks.setup.mockResolvedValue({
      status: 'ready',
      runtime_version: '0.3.5',
      permissions: 'unknown',
      host: 'server-host',
      target: 'darwin-arm64',
    });
    render(<ComputerUseSetup />);
    expect(mocks.setup).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Check Computer Use setup' }));
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
    render(<ComputerUseSetup />);
    fireEvent.click(screen.getByRole('button', { name: 'Check Computer Use setup' }));
    await screen.findByText('Bundled runtime missing');
    expect(screen.getByText('Payload not found on backend')).toBeVisible();
    expect(screen.getByText(/Install or repair the matching Biorouter package/)).toBeVisible();
  });
  it.each([
    ['probe_pending', 'Runtime found — setup not checked'],
    ['probe_failed', 'Could not check the native runtime'],
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
    render(<ComputerUseSetup />);
    fireEvent.click(screen.getByRole('button', { name: 'Check Computer Use setup' }));
    await screen.findByText(label);
    expect(
      screen.getByText(/Accessibility: allowed · Screen Recording: not allowed/)
    ).toBeVisible();
    expect(screen.getByText('Screen Recording permission is required.')).toBeVisible();
    expect(screen.getByText(/System Settings/)).toBeVisible();
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
    render(<ComputerUseSetup />);
    fireEvent.click(screen.getByRole('button', { name: 'Check Computer Use setup' }));
    await screen.findByText('OS permission required');
    fireEvent.click(screen.getByRole('button', { name: 'Check Computer Use setup' }));
    await screen.findByText('Runtime ready');
    expect(screen.getByText(/Accessibility: allowed · Screen Recording: allowed/)).toBeVisible();
    expect(mocks.setup).toHaveBeenCalledTimes(2);
  });
});
