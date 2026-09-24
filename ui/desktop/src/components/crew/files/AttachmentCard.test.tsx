import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewTransfer } from '../crewTransfers';
import { AttachmentCard, type CrewBlob } from './AttachmentCard';
import { TRANSFER_POLL_MS } from './useCrewTransfers';

const mocks = vi.hoisted(() => ({
  crewRequest: vi.fn(),
  listTransfers: vi.fn(),
  beginTransfer: vi.fn(),
  pauseTransfer: vi.fn(),
  resumeTransfer: vi.fn(),
  forgetTransfer: vi.fn(),
  previewAttachment: vi.fn(),
}));

vi.mock('../crewApi', () => ({ crewRequest: mocks.crewRequest }));
vi.mock('../crewTransfers', () => ({
  listTransfers: mocks.listTransfers,
  beginTransfer: mocks.beginTransfer,
  pauseTransfer: mocks.pauseTransfer,
  resumeTransfer: mocks.resumeTransfer,
  forgetTransfer: mocks.forgetTransfer,
  previewAttachment: mocks.previewAttachment,
}));

const blob = (id: string, overrides: Partial<CrewBlob> = {}): CrewBlob => ({
  id,
  channel_id: 'channel-1',
  name: `${id}.csv`,
  size: 56_320,
  sha256: 'f'.repeat(64),
  complete: true,
  media_type: 'text/csv',
  ...overrides,
});

const download = (blobId: string, overrides: Partial<CrewTransfer> = {}): CrewTransfer => ({
  id: `download-${blobId}`,
  request_id: `request-${blobId}`,
  connection_id: 'connection-1',
  channel_id: 'channel-1',
  direction: 'download',
  name: `${blobId}.csv`,
  size: 1000,
  sha256: 'f'.repeat(64),
  offset: 250,
  blob_id: blobId,
  state: 'downloading',
  error: null,
  ...overrides,
});

function installBlobs(overrides: Record<string, Partial<CrewBlob>> = {}) {
  mocks.crewRequest.mockImplementation(
    async (_connection: string, method: string, params: { blob_id: string }) => {
      if (method !== 'blob.status') throw new Error(`unexpected ${method}`);
      return blob(params.blob_id, overrides[params.blob_id]);
    }
  );
}

describe('AttachmentCard', () => {
  beforeEach(() => {
    for (const mock of Object.values(mocks)) mock.mockReset();
    mocks.listTransfers.mockResolvedValue([]);
    installBlobs();
  });

  it('shows the name and a human size, never a byte count or an ID', async () => {
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    expect(await screen.findByText('counts.csv')).toBeInTheDocument();
    expect(screen.getByText('55 KB')).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/56,?320/);
    expect(document.body.textContent).not.toContain('f'.repeat(64));
  });

  it('saves through the secure save dialog with the legacy download payload', async () => {
    mocks.beginTransfer.mockResolvedValue(null);
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    await screen.findByText('counts.csv');
    await userEvent.setup().click(screen.getByRole('button', { name: 'Save attachment' }));
    expect(mocks.beginTransfer).toHaveBeenCalledWith({
      connection_id: 'connection-1',
      channel_id: 'channel-1',
      direction: 'download',
      blob_id: 'counts',
      suggestedName: 'counts.csv',
    });
  });

  it('offers Preview image for the image types the daemon previews, and only those', async () => {
    installBlobs({ photo: { media_type: 'image/png', name: 'gel.png' } });
    const { unmount } = render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    await screen.findByText('counts.csv');
    expect(screen.queryByRole('button', { name: 'Preview image' })).toBeNull();
    unmount();

    render(<AttachmentCard connectionId="connection-1" blobId="photo" />);
    await screen.findByText('gel.png');
    expect(screen.getByRole('button', { name: 'Preview image' })).toBeInTheDocument();
  });

  it('copies the file ID and the SHA-256 from its menu and says so without a toast', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    await screen.findByText('counts.csv');
    await user.click(screen.getByRole('button', { name: 'More actions for counts.csv' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Copy file ID' }));
    expect(writeText).toHaveBeenLastCalledWith('counts');
    expect(await screen.findByText('Copied')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'More actions for counts.csv' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Copy SHA-256' }));
    expect(writeText).toHaveBeenLastCalledWith('f'.repeat(64));
  });

  it('draws download progress along the bottom edge and offers Pause while it moves', async () => {
    mocks.listTransfers.mockResolvedValue([download('counts')]);
    mocks.pauseTransfer.mockResolvedValue({});
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    const bar = await screen.findByRole('progressbar', { name: 'counts.csv: Downloading 25%' });
    expect(bar).toHaveAttribute('aria-valuenow', '25');
    expect(screen.getByRole('button', { name: 'Save attachment' })).toBeDisabled();

    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'More actions for counts.csv' }));
    const menu = await screen.findByRole('menu');
    expect(within(menu).queryByRole('menuitem', { name: /Remove from list/ })).toBeNull();
    await user.click(within(menu).getByRole('menuitem', { name: 'Pause' }));
    expect(mocks.pauseTransfer).toHaveBeenCalledWith('download-counts');
  });

  it('offers Resume… and Remove from list for a paused download', async () => {
    mocks.listTransfers.mockResolvedValue([download('counts', { state: 'needs_file_selection' })]);
    mocks.forgetTransfer.mockResolvedValue(null);
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    expect(await screen.findByText('55 KB · Paused')).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'More actions for counts.csv' }));
    const menu = await screen.findByRole('menu');
    expect(within(menu).getByRole('menuitem', { name: /Resume…/ })).toBeInTheDocument();
    const remove = within(menu).getByRole('menuitem', { name: /Remove from list/ });
    expect(remove).toHaveAccessibleDescription(
      'Removes the record on this computer. Shared files and saved downloads stay.'
    );
    await user.click(remove);
    expect(mocks.forgetTransfer).toHaveBeenCalledWith('download-counts');
  });

  it('shows an action failure once, in the card', async () => {
    mocks.beginTransfer.mockRejectedValue(new Error('The daemon refused this file selection.'));
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    await screen.findByText('counts.csv');
    await userEvent.setup().click(screen.getByRole('button', { name: 'Save attachment' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The daemon refused this file selection.'
    );
    expect(screen.getAllByRole('alert')).toHaveLength(1);
  });
});

describe('the one transfers poller (L13)', () => {
  beforeEach(() => {
    for (const mock of Object.values(mocks)) mock.mockReset();
    installBlobs();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const cards = (ids: string[]) => (
    <>
      {ids.map((id) => (
        <AttachmentCard key={id} connectionId="connection-1" blobId={id} />
      ))}
    </>
  );
  const flush = () => act(async () => undefined);
  const tick = () =>
    act(async () => {
      await vi.advanceTimersByTimeAsync(TRANSFER_POLL_MS);
    });

  it('lists once for any number of cards, and polls every 2s only while something moves', async () => {
    let transfers: CrewTransfer[] = [download('a')];
    mocks.listTransfers.mockImplementation(async () => transfers);
    const { unmount } = render(cards(['a', 'b', 'c', 'd', 'e']));
    await flush();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(1);
    expect(mocks.listTransfers).toHaveBeenCalledWith('connection-1');

    await tick();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(2);
    await tick();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(3);

    transfers = [download('a', { state: 'completed', offset: 1000 })];
    await tick();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(4);
    await tick();
    await tick();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(4);

    unmount();
  });

  it('does not poll at all while every transfer is at rest', async () => {
    mocks.listTransfers.mockResolvedValue([download('a', { state: 'completed' })]);
    render(cards(['a', 'b']));
    await flush();
    await tick();
    await tick();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(1);
  });

  it('stops polling when the last card leaves, and lists afresh for the next one', async () => {
    mocks.listTransfers.mockResolvedValue([download('a')]);
    const first = render(cards(['a']));
    await flush();
    first.unmount();
    await tick();
    await tick();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(1);

    render(cards(['a']));
    await flush();
    expect(mocks.listTransfers).toHaveBeenCalledTimes(2);
  });

  it('reads an answer that is not a list as a failed list, never as a crash', async () => {
    mocks.listTransfers.mockResolvedValue(undefined);
    render(cards(['a']));
    await flush();
    await flush();
    expect(screen.getByText('a.csv')).toBeInTheDocument();
    expect(screen.queryByRole('progressbar')).toBeNull();
  });

  it('keeps the last records and keeps trying after a failed list while a transfer moved', async () => {
    mocks.listTransfers.mockResolvedValueOnce([download('a')]);
    mocks.listTransfers.mockRejectedValueOnce(new Error('daemon restarting'));
    mocks.listTransfers.mockResolvedValue([download('a', { offset: 900 })]);
    render(cards(['a']));
    await flush();
    await flush();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '25');
    await tick();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '25');
    await tick();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '90');
  });
});
