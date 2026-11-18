import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TunnelSection from './TunnelSection';

const mocks = vi.hoisted(() => ({
  getTunnelStatus: vi.fn(),
  refreshConfig: vi.fn(),
  startTunnel: vi.fn(),
  stopTunnel: vi.fn(),
}));

vi.mock('../../../api/sdk.gen', () => ({
  getTunnelStatus: mocks.getTunnelStatus,
  startTunnel: mocks.startTunnel,
  stopTunnel: mocks.stopTunnel,
}));

vi.mock('../../ConfigContext', () => ({
  useConfig: () => ({ refreshConfig: mocks.refreshConfig }),
}));

const PROOF = { 'X-User-Action': 'desktop-user-action-key' };

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => PROOF,
}));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.refreshConfig.mockResolvedValue(undefined);
  mocks.startTunnel.mockResolvedValue({
    data: {
      state: 'running',
      url: 'https://example.test',
      hostname: 'example.test',
      secret: 'secret',
    },
  });
  mocks.stopTunnel.mockResolvedValue({ data: undefined });
});

describe('TunnelSection config cache', () => {
  it('refreshes config after starting the tunnel', async () => {
    mocks.getTunnelStatus.mockResolvedValue({
      data: { state: 'idle', url: '', hostname: '', secret: '' },
    });

    render(<TunnelSection />);
    fireEvent.click(await screen.findByRole('button', { name: 'Start Tunnel' }));

    await waitFor(() => expect(mocks.startTunnel).toHaveBeenCalledOnce());
    expect(mocks.refreshConfig).toHaveBeenCalledOnce();
  });

  it('refreshes config after stopping the tunnel', async () => {
    mocks.getTunnelStatus.mockResolvedValue({
      data: {
        state: 'running',
        url: 'https://example.test',
        hostname: 'example.test',
        secret: 'secret',
      },
    });

    render(<TunnelSection />);
    fireEvent.click(await screen.findByRole('button', { name: 'Stop tunnel' }));

    await waitFor(() => expect(mocks.stopTunnel).toHaveBeenCalledOnce());
    expect(mocks.refreshConfig).toHaveBeenCalledOnce();
  });
});

/**
 * GAP-SV-01: the daemon opens and closes the tunnel, and shows its URL and secret, only for a
 * request that proves the person sent it. Without the proof each of these calls is refused or
 * answered with the state alone.
 */
describe('TunnelSection user-action proof', () => {
  it('sends the proof on the status read, the start and the stop', async () => {
    mocks.getTunnelStatus.mockResolvedValue({
      data: { state: 'idle', url: '', hostname: '', secret: '' },
    });

    render(<TunnelSection />);
    await waitFor(() => expect(mocks.getTunnelStatus).toHaveBeenCalled());
    expect(mocks.getTunnelStatus).toHaveBeenCalledWith({ headers: PROOF });

    fireEvent.click(await screen.findByRole('button', { name: 'Start Tunnel' }));
    await waitFor(() => expect(mocks.startTunnel).toHaveBeenCalledWith({ headers: PROOF }));

    fireEvent.click(await screen.findByRole('button', { name: 'Stop tunnel' }));
    await waitFor(() => expect(mocks.stopTunnel).toHaveBeenCalledWith({ headers: PROOF }));
  });

  it('waits for the relay to connect before it shows the tunnel as active', async () => {
    mocks.getTunnelStatus
      .mockResolvedValueOnce({ data: { state: 'idle', url: '', hostname: '', secret: '' } })
      .mockResolvedValueOnce({
        data: {
          state: 'starting',
          url: 'https://example.test',
          hostname: 'example.test',
          secret: 'secret',
        },
      })
      .mockResolvedValue({
        data: {
          state: 'running',
          url: 'https://example.test',
          hostname: 'example.test',
          secret: 'secret',
        },
      });
    // The daemon answers start before any websocket exists, so it says `starting`.
    mocks.startTunnel.mockResolvedValue({
      data: {
        state: 'starting',
        url: 'https://example.test',
        hostname: 'example.test',
        secret: 'secret',
      },
    });

    render(<TunnelSection />);
    fireEvent.click(await screen.findByRole('button', { name: 'Start Tunnel' }));

    expect(await screen.findByText('Tunnel is active', {}, { timeout: 4000 })).toBeInTheDocument();
    expect(mocks.getTunnelStatus).toHaveBeenLastCalledWith({ headers: PROOF });
  });
});
