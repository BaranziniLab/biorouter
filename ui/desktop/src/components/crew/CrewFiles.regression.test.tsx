import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewUpload } from './CrewFiles';

const mocks = vi.hoisted(() => ({
  beginTransfer: vi.fn(),
  listTransfers: vi.fn(),
}));

vi.mock('./crewTransfers', () => ({
  beginTransfer: mocks.beginTransfer,
  forgetTransfer: vi.fn(),
  listTransfers: mocks.listTransfers,
  pauseTransfer: vi.fn(),
  previewAttachment: vi.fn(),
  resumeTransfer: vi.fn(),
}));

describe('Crew upload privacy handoff', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.beginTransfer.mockReset();
    mocks.listTransfers.mockReset().mockResolvedValue([]);
  });

  const props = {
    connectionId: 'connection-1',
    channelId: 'channel-1',
    disabled: false,
    onReady: vi.fn(),
    onRemoteReference: vi.fn(),
  };

  it('refuses to open an upload when the authoritative privacy mode is unavailable', async () => {
    render(<CrewUpload {...props} expectedMode={undefined} />);

    fireEvent.click(screen.getByRole('button', { name: 'Choose file to upload' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Refresh the workspace to verify connection privacy before uploading.'
    );
    expect(mocks.beginTransfer).not.toHaveBeenCalled();
  });

  it('passes the observed mode into the transfer request before the picker resolves', async () => {
    mocks.beginTransfer.mockResolvedValue(null);
    render(<CrewUpload {...props} expectedMode="public" />);

    fireEvent.click(screen.getByRole('button', { name: 'Choose file to upload' }));

    await waitFor(() =>
      expect(mocks.beginTransfer).toHaveBeenCalledWith({
        expected_mode: 'public',
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'upload',
      })
    );
  });
});
