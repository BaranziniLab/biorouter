import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewTransfer } from '../crewTransfers';
import { AttachmentCard, type CrewBlob } from './AttachmentCard';
import { AttachmentIndexProvider } from './attachmentIndex';
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
    await userEvent.setup().click(screen.getByRole('button', { name: 'Save counts.csv' }));
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
    expect(screen.queryByRole('button', { name: /^Preview / })).toBeNull();
    unmount();

    render(<AttachmentCard connectionId="connection-1" blobId="photo" />);
    await screen.findByText('gel.png');
    expect(screen.getByRole('button', { name: 'Preview gel.png' })).toBeInTheDocument();
  });

  /** Opens the card's ⋯, then its "Copy for support" submenu, by keyboard as a person would. */
  async function openSupportMenu(user: ReturnType<typeof userEvent.setup>, name = 'counts.csv') {
    await user.click(screen.getByRole('button', { name: `More actions for ${name}` }));
    const support = await screen.findByRole('menuitem', { name: 'Copy for support' });
    act(() => support.focus());
    await user.keyboard('{ArrowRight}');
    return support;
  }

  it('keeps the file ID and the SHA-256 in “Copy for support”, and answers in the menu (Q3-26)', async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    await screen.findByText('counts.csv');

    // The menu is never one of IDs only: Save comes first, the IDs last and one step away.
    await user.click(screen.getByRole('button', { name: 'More actions for counts.csv' }));
    const menu = await screen.findByRole('menu');
    const top = Array.from(
      menu.querySelectorAll(':scope > [role="menuitem"], :scope > [role="separator"]')
    ).map((node) => (node.getAttribute('role') === 'separator' ? '—' : node.textContent));
    expect(top).toEqual(['Save counts.csv…', '—', 'Copy for support']);
    expect(within(menu).queryByRole('menuitem', { name: 'Copy file ID' })).toBeNull();
    await user.keyboard('{Escape}');

    await openSupportMenu(user);
    await user.click(await screen.findByRole('menuitem', { name: 'Copy file ID' }));
    expect(writeText).toHaveBeenLastCalledWith('counts');
    // "Copied" on the item itself until the menu closes, and said aloud.
    expect(await screen.findByRole('menuitem', { name: 'Copied' })).toHaveAttribute(
      'data-crew-copy-state',
      'copied'
    );
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull(), { timeout: 2000 });
    expect(screen.getByText('Copied')).toBeInTheDocument();

    await openSupportMenu(user);
    await user.click(await screen.findByRole('menuitem', { name: 'Copy SHA-256' }));
    expect(writeText).toHaveBeenLastCalledWith('f'.repeat(64));
  });

  it('saves from ⋯ too, through the same secure dialog', async () => {
    mocks.beginTransfer.mockResolvedValue(null);
    const user = userEvent.setup();
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    await screen.findByText('counts.csv');
    await user.click(screen.getByRole('button', { name: 'More actions for counts.csv' }));
    await user.click(await screen.findByRole('menuitem', { name: 'Save counts.csv…' }));
    expect(mocks.beginTransfer).toHaveBeenCalledWith(
      expect.objectContaining({ direction: 'download', blob_id: 'counts' })
    );
  });

  it('puts its controls in the Tab order only when told to (Q3-05)', async () => {
    installBlobs({ photo: { media_type: 'image/png', name: 'gel.png' } });
    const { rerender } = render(
      <AttachmentCard connectionId="connection-1" blobId="photo" tabIndex={-1} />
    );
    await screen.findByText('gel.png');
    const controls = () => [
      screen.getByRole('button', { name: 'Save gel.png' }),
      screen.getByRole('button', { name: 'Preview gel.png' }),
      screen.getByRole('button', { name: 'More actions for gel.png' }),
    ];
    expect(controls().map((control) => control.tabIndex)).toEqual([-1, -1, -1]);
    rerender(<AttachmentCard connectionId="connection-1" blobId="photo" tabIndex={0} />);
    expect(controls().map((control) => control.tabIndex)).toEqual([0, 0, 0]);
    // The Files tab passes none: ordinary Tab stops.
    rerender(<AttachmentCard connectionId="connection-1" blobId="photo" />);
    expect(controls().map((control) => control.tabIndex)).toEqual([0, 0, 0]);
  });

  describe('two files with the same name (Q3-13)', () => {
    const today = new Date();
    const at = (hour: number, minute: number, second = 0) =>
      new Date(
        today.getFullYear(),
        today.getMonth(),
        today.getDate(),
        hour,
        minute,
        second
      ).getTime();

    it('names each card’s controls and meta with its own post time', async () => {
      installBlobs({
        first: { name: 'gina-assay.csv', size: 100 },
        second: { name: 'gina-assay.csv', size: 100 },
        other: { name: 'plate.csv', size: 100 },
      });
      render(
        <AttachmentIndexProvider>
          <AttachmentCard connectionId="connection-1" blobId="first" postedAt={at(18, 54)} />
          <AttachmentCard connectionId="connection-1" blobId="second" postedAt={at(18, 56)} />
          <AttachmentCard connectionId="connection-1" blobId="other" postedAt={at(18, 57)} />
        </AttachmentIndexProvider>
      );
      expect(
        await screen.findByRole('button', { name: 'Save gina-assay.csv, 6:54 PM' })
      ).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Save gina-assay.csv, 6:56 PM' })
      ).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'More actions for gina-assay.csv, 6:54 PM' })
      ).toBeInTheDocument();
      expect(screen.getByText('100 bytes · 6:54 PM')).toBeInTheDocument();
      expect(screen.getByText('100 bytes · 6:56 PM')).toBeInTheDocument();
      // A name of its own keeps its plain controls and meta.
      expect(screen.getByRole('button', { name: 'Save plate.csv' })).toBeInTheDocument();
      expect(screen.getByText('100 bytes')).toBeInTheDocument();
    });

    it('counts them when even the times read the same, and forgets a card that leaves', async () => {
      installBlobs({
        first: { name: 'gina-assay.csv' },
        second: { name: 'gina-assay.csv' },
      });
      const { rerender } = render(
        <AttachmentIndexProvider>
          <AttachmentCard connectionId="connection-1" blobId="first" postedAt={at(18, 54, 1)} />
          <AttachmentCard connectionId="connection-1" blobId="second" postedAt={at(18, 54, 40)} />
        </AttachmentIndexProvider>
      );
      expect(
        await screen.findByRole('button', { name: 'Save gina-assay.csv, 6:54 PM, 1 of 2' })
      ).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Save gina-assay.csv, 6:54 PM, 2 of 2' })
      ).toBeInTheDocument();
      rerender(
        <AttachmentIndexProvider>
          <AttachmentCard connectionId="connection-1" blobId="first" postedAt={at(18, 54, 1)} />
        </AttachmentIndexProvider>
      );
      expect(
        await screen.findByRole('button', { name: 'Save gina-assay.csv' })
      ).toBeInTheDocument();
    });
  });

  it('draws download progress along the bottom edge and offers Pause while it moves', async () => {
    mocks.listTransfers.mockResolvedValue([download('counts')]);
    mocks.pauseTransfer.mockResolvedValue({});
    render(<AttachmentCard connectionId="connection-1" blobId="counts" />);
    const bar = await screen.findByRole('progressbar', { name: 'counts.csv: Downloading 25%' });
    expect(bar).toHaveAttribute('aria-valuenow', '25');
    expect(screen.getByRole('button', { name: 'Save counts.csv' })).toBeDisabled();

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
    await userEvent.setup().click(screen.getByRole('button', { name: 'Save counts.csv' }));
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
