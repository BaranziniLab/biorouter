import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewTransfer } from '../crewTransfers';
import { filesCopy } from './copy';
import { UploadChip } from './UploadChip';

const mocks = vi.hoisted(() => ({
  cancelUpload: vi.fn(),
  refreshCrewTransfers: vi.fn(),
}));

vi.mock('../crewTransfers', () => ({ cancelUpload: mocks.cancelUpload }));
vi.mock('./useCrewTransfers', () => ({ refreshCrewTransfers: mocks.refreshCrewTransfers }));

const upload = (overrides: Partial<CrewTransfer> = {}): CrewTransfer => ({
  id: 'upload-1',
  request_id: 'request-1',
  connection_id: 'connection-1',
  channel_id: 'channel-1',
  direction: 'upload',
  name: 'counts.csv',
  size: 1000,
  sha256: 'f'.repeat(64),
  offset: 0,
  blob_id: null,
  state: 'needs_file_selection',
  error: 'Transfer paused. Reselect the original local file or destination to resume.',
  ...overrides,
});

const PART_STAYS =
  'The unfinished part stays on the server for up to a day and counts toward the workspace’s file space until then.';

describe('UploadChip: Cancel upload (FILES-F7)', () => {
  beforeEach(() => {
    mocks.cancelUpload.mockReset();
    mocks.refreshCrewTransfers.mockReset().mockResolvedValue(undefined);
  });

  const renderChip = (transfer = upload()) =>
    render(<UploadChip transfer={transfer} onPause={vi.fn()} onResume={vi.fn()} />);

  it('says what cancelling leaves on the server before it cancels anything', async () => {
    renderChip();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Cancel uploading counts.csv' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Cancel uploading counts.csv?')).toBeInTheDocument();
    expect(within(dialog).getByText(PART_STAYS)).toBeInTheDocument();
    expect(filesCopy.unfinishedPartStays).toBe(PART_STAYS);
    await user.click(within(dialog).getByRole('button', { name: filesCopy.keepUpload }));
    expect(mocks.cancelUpload).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('cancels on confirmation and asks the poller for the list again', async () => {
    mocks.cancelUpload.mockResolvedValue('cancelled');
    renderChip(upload({ state: 'uploading', offset: 400, error: null }));
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Cancel uploading counts.csv' }));
    const dialog = await screen.findByRole('dialog');
    // A moving upload is kept by "Keep uploading".
    expect(within(dialog).getByRole('button', { name: filesCopy.keepUploading })).toBeVisible();
    await user.click(within(dialog).getByRole('button', { name: 'Cancel upload' }));
    expect(mocks.cancelUpload).toHaveBeenCalledWith('upload-1');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(mocks.refreshCrewTransfers).toHaveBeenCalledWith('connection-1');
  });

  it('shows a refusal in the confirmation that asked, in plain words', async () => {
    mocks.cancelUpload.mockRejectedValue(
      new Error('Crew couldn’t stop that upload yet. Try again in a moment.')
    );
    renderChip();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Cancel uploading counts.csv' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Cancel upload' }));
    expect(await within(dialog).findByRole('alert')).toHaveTextContent(
      'Crew couldn’t stop that upload yet. Try again in a moment.'
    );
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('reads a paused upload as Paused, with why on hover, and offers Resume', () => {
    renderChip();
    expect(screen.getByText('Paused')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Resume counts.csv' })).toBeInTheDocument();
    expect(screen.getByText('Paused').closest('[title]')).toHaveAttribute('title', 'You paused it');
  });

  it('offers no Resume for an upload the workspace refused', () => {
    renderChip(upload({ state: 'failed', error: 'You are no longer in #methods.' }));
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Resume counts.csv' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Cancel uploading counts.csv' })).toBeInTheDocument();
  });
});
