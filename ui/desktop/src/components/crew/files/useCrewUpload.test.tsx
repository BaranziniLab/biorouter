import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Composer } from '../composer/Composer';
import type { CrewController } from '../state/types';
import { filesCopy } from './copy';
import { crewTestController, CrewTestProvider } from './crewTestController';

const mocks = vi.hoisted(() => ({
  beginTransfer: vi.fn(),
  listTransfers: vi.fn(),
}));

vi.mock('../crewTransfers', () => ({
  beginTransfer: mocks.beginTransfer,
  forgetTransfer: vi.fn(),
  listTransfers: mocks.listTransfers,
  pauseTransfer: vi.fn(),
  previewAttachment: vi.fn(),
  resumeTransfer: vi.fn(),
}));

/** A verified view whose privacy the observer has not confirmed for this connection. */
const privacyUnverified: Partial<CrewController> = {
  observedPrivacy: {
    connectionId: 'another-connection',
    mode: 'private',
    institutionId: null,
    policyEpoch: 1,
  },
};
const observedPublic: Partial<CrewController> = {
  observedPrivacy: {
    connectionId: 'connection-1',
    mode: 'public',
    institutionId: null,
    policyEpoch: 3,
  },
};

function renderComposer(overrides: Partial<CrewController> = {}) {
  return render(
    <CrewTestProvider controller={crewTestController(overrides)}>
      <Composer />
    </CrewTestProvider>
  );
}

async function chooseUpload() {
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: 'Attach' }));
  await user.click(await screen.findByRole('menuitem', { name: 'Upload a file…' }));
}

function completedUpload(id: string, blobId: string, name: string) {
  return {
    id,
    request_id: `request-${id}`,
    connection_id: 'connection-1',
    channel_id: 'channel-1',
    direction: 'upload' as const,
    name,
    size: 103,
    sha256: 'a'.repeat(64),
    offset: 103,
    blob_id: blobId,
    state: 'completed',
    error: null,
  };
}

const originalElectron = (window as { electron?: unknown }).electron;

describe('Crew upload privacy handoff, from the Attach menu', () => {
  beforeEach(() => {
    mocks.beginTransfer.mockReset();
    mocks.listTransfers.mockReset().mockResolvedValue([]);
  });
  afterEach(() => {
    (window as { electron?: unknown }).electron = originalElectron;
  });

  it('refuses to open an upload when the authoritative privacy mode is unavailable', async () => {
    renderComposer(privacyUnverified);

    await chooseUpload();

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(
      'Refresh the workspace to verify connection privacy before uploading.'
    );
    expect(screen.getAllByRole('alert')).toHaveLength(1);
    expect(filesCopy.privacyPending).toBe(
      'Refresh the workspace to verify connection privacy before uploading.'
    );
    expect(mocks.beginTransfer).not.toHaveBeenCalled();
  });

  it('passes the observed mode into the transfer request before the picker resolves', async () => {
    mocks.beginTransfer.mockResolvedValue(null);
    renderComposer(observedPublic);

    await chooseUpload();

    await waitFor(() =>
      expect(mocks.beginTransfer).toHaveBeenCalledWith({
        expected_mode: 'public',
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'upload',
      })
    );
    expect(mocks.beginTransfer.mock.calls[0]).toHaveLength(1);
    expect(Object.keys(mocks.beginTransfer.mock.calls[0][0]).sort()).toEqual([
      'channel_id',
      'connection_id',
      'direction',
      'expected_mode',
    ]);
  });

  it('adds a finished upload to the draft exactly once', async () => {
    const addAttachment = vi.fn();
    let transfers: unknown[] = [];
    mocks.listTransfers.mockImplementation(async () => transfers);
    mocks.beginTransfer.mockImplementation(async () => {
      transfers = [completedUpload('transfer-1', 'blob-1', 'counts.csv')];
      return { id: 'transfer-1' };
    });
    renderComposer({ addAttachment });

    await chooseUpload();

    await waitFor(() =>
      expect(addAttachment).toHaveBeenCalledWith({ id: 'blob-1', name: 'counts.csv' })
    );
    await act(async () => {
      const { refreshCrewTransfers } = await import('./useCrewTransfers');
      await refreshCrewTransfers('connection-1');
    });
    expect(addAttachment).toHaveBeenCalledTimes(1);
  });

  it('never adds an upload it did not start', async () => {
    const addAttachment = vi.fn();
    mocks.listTransfers.mockResolvedValue([completedUpload('other', 'blob-2', 'old.csv')]);
    renderComposer({ addAttachment });
    await act(async () => undefined);
    expect(addAttachment).not.toHaveBeenCalled();
  });
});

describe('Crew files dropped or pasted', () => {
  beforeEach(() => {
    mocks.beginTransfer.mockReset();
    mocks.listTransfers.mockReset().mockResolvedValue([]);
  });
  afterEach(() => {
    (window as { electron?: unknown }).electron = originalElectron;
  });

  const file = (name: string, size?: number) => {
    const item = new File(['x'], name, { type: 'text/csv' });
    if (size !== undefined) Object.defineProperty(item, 'size', { value: size });
    return item;
  };
  const dragData = (files: File[], directory = false) => ({
    types: ['Files'],
    files,
    items: files.map(() => ({
      kind: 'file',
      webkitGetAsEntry: () => ({ isDirectory: directory }),
    })),
    dropEffect: 'none',
  });
  const dropZone = () =>
    screen.getByLabelText('Message #general').closest('[data-drop-zone="true"]') as HTMLElement;

  it('shows where a drag will land, and routes the drop through the same picker', async () => {
    let finish!: () => void;
    mocks.beginTransfer.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = () => resolve(null);
        })
    );
    renderComposer(observedPublic);
    const zone = dropZone();
    const data = dragData([file('counts.csv')]);

    fireEvent.dragEnter(zone, { dataTransfer: data });
    expect(screen.getByText('Drop to attach in #general')).toBeInTheDocument();

    fireEvent.drop(zone, { dataTransfer: data });
    expect(screen.queryByText('Drop to attach in #general')).toBeNull();
    expect(await screen.findByRole('status')).toHaveTextContent(
      'A file window opened. Select counts.csv there and choose Open to share it.'
    );
    expect(mocks.beginTransfer).toHaveBeenCalledWith({
      expected_mode: 'public',
      connection_id: 'connection-1',
      channel_id: 'channel-1',
      direction: 'upload',
    });

    await act(async () => finish());
    expect(screen.queryByText(/A file window opened/)).toBeNull();
  });

  it('ignores a drag that carries no files', () => {
    renderComposer(observedPublic);
    fireEvent.dragEnter(dropZone(), { dataTransfer: { types: ['text/plain'], files: [] } });
    expect(screen.queryByText('Drop to attach in #general')).toBeNull();
  });

  it('says one file at a time when several are dropped', async () => {
    mocks.beginTransfer.mockReturnValue(new Promise(() => undefined));
    renderComposer(observedPublic);
    fireEvent.drop(dropZone(), { dataTransfer: dragData([file('a.csv'), file('b.csv')]) });
    expect(await screen.findByRole('status')).toHaveTextContent(
      'A file window opened. Select a.csv there and choose Open to share it. Crew shares one file at a time.'
    );
    expect(mocks.beginTransfer).toHaveBeenCalledTimes(1);
  });

  it('refuses a folder and a file over the limit before any picker opens', async () => {
    renderComposer(observedPublic);
    fireEvent.drop(dropZone(), { dataTransfer: dragData([file('data')], true) });
    expect(await screen.findByRole('alert')).toHaveTextContent(filesCopy.folderRefused);

    fireEvent.drop(dropZone(), {
      dataTransfer: dragData([file('huge.bam', 1024 * 1024 * 1024 + 1)]),
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'huge.bam is larger than 1 GB. Crew can share files up to 1 GB.'
    );
    expect(screen.getAllByRole('alert')).toHaveLength(1);
    expect(mocks.beginTransfer).not.toHaveBeenCalled();
  });

  it('routes a pasted file through the same picker, and leaves pasted text alone', async () => {
    mocks.beginTransfer.mockResolvedValue(null);
    renderComposer(observedPublic);
    const input = screen.getByLabelText('Message #general');

    const text = fireEvent.paste(input, { clipboardData: { files: [], getData: () => 'hi' } });
    expect(text).toBe(true);
    expect(mocks.beginTransfer).not.toHaveBeenCalled();

    const pasted = fireEvent.paste(input, { clipboardData: { files: [file('notes.txt')] } });
    expect(pasted).toBe(false);
    await waitFor(() =>
      expect(mocks.beginTransfer).toHaveBeenCalledWith({
        expected_mode: 'public',
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'upload',
      })
    );
  });

  it('pastes the words when copied text comes with a picture of itself', () => {
    (window as { electron?: unknown }).electron = {
      crewSelectTransferFile: vi.fn(),
      getPathForFile: () => '',
    };
    renderComposer(observedPublic);
    const pasted = fireEvent.paste(screen.getByLabelText('Message #general'), {
      clipboardData: { files: [file('cells.png')], getData: () => 'A1\tB1' },
    });
    expect(pasted).toBe(true);
    expect(mocks.beginTransfer).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('uploads a file copied on this computer even when its name comes along as text', async () => {
    (window as { electron?: unknown }).electron = {
      crewSelectTransferFile: vi.fn(),
      getPathForFile: () => '/Users/alice/counts.csv',
    };
    mocks.beginTransfer.mockResolvedValue(null);
    renderComposer(observedPublic);
    const pasted = fireEvent.paste(screen.getByLabelText('Message #general'), {
      clipboardData: { files: [file('counts.csv')], getData: () => 'counts.csv' },
    });
    expect(pasted).toBe(false);
    await waitFor(() => expect(mocks.beginTransfer).toHaveBeenCalledTimes(1));
  });

  it('tells the person to save pasted data that has no file behind it', async () => {
    (window as { electron?: unknown }).electron = {
      crewSelectTransferFile: vi.fn(),
      getPathForFile: () => '',
    };
    renderComposer(observedPublic);
    fireEvent.paste(screen.getByLabelText('Message #general'), {
      clipboardData: { files: [file('image.png')] },
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(filesCopy.notSaved);
    // The main process's twin says the same: share it again, never "attach it" (Q3-25).
    expect(filesCopy.notSaved).toBe(
      'Crew can share saved files only. Save it as a file first, then share it again.'
    );
    expect(mocks.beginTransfer).not.toHaveBeenCalled();
  });

  it('opens the picker for a dropped file that the preload can locate', async () => {
    const getPathForFile = vi.fn(() => '/Users/alice/counts.csv');
    (window as { electron?: unknown }).electron = {
      crewSelectTransferFile: vi.fn(),
      getPathForFile,
    };
    mocks.beginTransfer.mockResolvedValue(null);
    renderComposer(observedPublic);
    fireEvent.drop(dropZone(), { dataTransfer: dragData([file('counts.csv')]) });
    await waitFor(() => expect(mocks.beginTransfer).toHaveBeenCalledTimes(1));
    expect(getPathForFile).toHaveBeenCalledTimes(1);
    // The path is only looked at, never sent: the request is the exact Attach-menu payload.
    expect(JSON.stringify(mocks.beginTransfer.mock.calls[0])).not.toContain('/Users/alice');
  });

  it('takes no drop in an archived channel', () => {
    renderComposer({
      ...observedPublic,
      channel: { ...crewTestController().channel!, archived: true },
    });
    const zone = screen
      .getByText('This channel is archived.')
      .closest('[data-drop-zone="true"]') as HTMLElement;
    fireEvent.dragEnter(zone, { dataTransfer: dragData([file('counts.csv')]) });
    fireEvent.drop(zone, { dataTransfer: dragData([file('counts.csv')]) });
    expect(screen.queryByText(/Drop to attach/)).toBeNull();
    expect(mocks.beginTransfer).not.toHaveBeenCalled();
  });
});
