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
