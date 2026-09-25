import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CrewMessage } from '../crewApi';
import type { CrewTransfer } from '../crewTransfers';
import type { CrewController } from '../state/types';
import { filesCopy } from './copy';
import { crewTestController, CrewTestProvider } from './crewTestController';
import { FilesTab } from './FilesTab';

const mocks = vi.hoisted(() => ({
  crewRequest: vi.fn(),
  listTransfers: vi.fn(),
  pauseTransfer: vi.fn(),
  resumeTransfer: vi.fn(),
  forgetTransfer: vi.fn(),
}));

vi.mock('../crewApi', () => ({ crewRequest: mocks.crewRequest }));
vi.mock('../crewTransfers', () => ({
  listTransfers: mocks.listTransfers,
  pauseTransfer: mocks.pauseTransfer,
  resumeTransfer: mocks.resumeTransfer,
  forgetTransfer: mocks.forgetTransfer,
  beginTransfer: vi.fn(),
  previewAttachment: vi.fn(),
}));

const transfer = (overrides: Partial<CrewTransfer>): CrewTransfer => ({
  id: 'transfer-1',
  request_id: 'request-1',
  connection_id: 'connection-1',
  channel_id: 'channel-1',
  direction: 'upload',
  name: 'counts.csv',
  size: 2048,
  sha256: 'a'.repeat(64),
  offset: 1024,
  blob_id: null,
  state: 'uploading',
  error: null,
  ...overrides,
});

const message = (overrides: Partial<CrewMessage>): CrewMessage => ({
  id: 'message-1',
  sequence: '1',
  channel_id: 'channel-1',
  actor_id: 'person-1',
  body: 'here you go',
  created_at: 1,
  restricted: false,
  source_channels: [],
  attachments: [],
  ...overrides,
});

function renderTab(overrides: Partial<CrewController> = {}) {
  return render(
    <CrewTestProvider controller={crewTestController(overrides)}>
      <FilesTab />
    </CrewTestProvider>
  );
}

describe('FilesTab', () => {
  beforeEach(() => {
    for (const mock of Object.values(mocks)) mock.mockReset();
    mocks.listTransfers.mockResolvedValue([]);
    mocks.crewRequest.mockImplementation(async (_id: string, method: string, params: object) => {
      if (method === 'blob.status')
        return {
          id: (params as { blob_id: string }).blob_id,
          channel_id: 'channel-1',
          name: 'shared.csv',
          size: 10,
          sha256: 'b'.repeat(64),
          complete: true,
          media_type: 'text/csv',
        };
      if (method === 'reference.get')
        return { path: '/home/alice/results/run-7', label: 'Run 7', verified: false };
      throw new Error(`unexpected ${method}`);
    });
  });

  it('says so when the channel has no files', async () => {
    renderTab();
    expect(await screen.findByText(filesCopy.noFiles)).toBeInTheDocument();
  });

  it('lists this channel’s unfinished transfers with a state word and Pause', async () => {
    mocks.listTransfers.mockResolvedValue([
      transfer({}),
      transfer({ id: 'elsewhere', channel_id: 'channel-2', name: 'other.csv' }),
    ]);
    mocks.pauseTransfer.mockResolvedValue({});
    renderTab();
    const section = (await screen.findByRole('heading', { name: 'In progress' })).closest(
      'section'
    ) as HTMLElement;
    expect(within(section).getByText('counts.csv')).toBeInTheDocument();
    expect(within(section).getByText('Uploading 50% · 2 KB')).toBeInTheDocument();
    expect(screen.queryByText('other.csv')).toBeNull();
    await userEvent
      .setup()
      .click(within(section).getByRole('button', { name: 'Pause counts.csv' }));
    expect(mocks.pauseTransfer).toHaveBeenCalledWith('transfer-1');
    await waitFor(() => expect(mocks.listTransfers).toHaveBeenCalledTimes(2));
  });

  it('shows a failure in the daemon’s words and offers Resume… from the row menu', async () => {
    mocks.listTransfers.mockResolvedValue([
      transfer({ state: 'needs_file_selection', error: 'The source file changed.' }),
    ]);
    mocks.resumeTransfer.mockResolvedValue(null);
    renderTab();
    expect(await screen.findByText('The source file changed.')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'More actions for counts.csv' }));
    await user.click(await screen.findByRole('menuitem', { name: /Resume…/ }));
    expect(mocks.resumeTransfer).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'transfer-1' })
    );
  });

  it('attaches a finished upload only after re-reading its shared file', async () => {
    const addAttachment = vi.fn();
    const request = vi.fn(async () => ({
      id: 'blob-1',
      channel_id: 'channel-1',
      name: 'counts.csv',
      size: 2048,
      sha256: 'a'.repeat(64),
      complete: true,
      media_type: 'text/csv',
    }));
    mocks.listTransfers.mockResolvedValue([
      transfer({ state: 'completed', offset: 2048, blob_id: 'blob-1' }),
    ]);
    renderTab({ addAttachment, request: request as CrewController['request'] });
    const attach = await screen.findByRole('button', { name: 'Attach counts.csv' });
    expect(screen.getByRole('heading', { name: 'Uploaded, not sent' })).toBeInTheDocument();
    await userEvent.setup().click(attach);
    expect(request).toHaveBeenCalledWith('blob.status', { blob_id: 'blob-1' });
    expect(addAttachment).toHaveBeenCalledWith({ id: 'blob-1', name: 'counts.csv' });
  });

  it('refuses to attach an upload whose shared file no longer matches', async () => {
    const addAttachment = vi.fn();
    const request = vi.fn(async () => ({
      id: 'blob-1',
      channel_id: 'channel-1',
      name: 'counts.csv',
      size: 2048,
      sha256: 'c'.repeat(64),
      complete: true,
      media_type: 'text/csv',
    }));
    mocks.listTransfers.mockResolvedValue([
      transfer({ state: 'completed', offset: 2048, blob_id: 'blob-1' }),
    ]);
    renderTab({ addAttachment, request: request as CrewController['request'] });
    await userEvent.setup().click(await screen.findByRole('button', { name: 'Attach counts.csv' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(filesCopy.attachMismatch);
    expect(addAttachment).not.toHaveBeenCalled();
  });

  it('lists the composer’s files as in your message, not sent yet, with nothing to press (Q3-03)', async () => {
    mocks.listTransfers.mockResolvedValue([
      transfer({ state: 'completed', offset: 2048, blob_id: 'blob-1' }),
    ]);
    renderTab({
      draft: { body: '', attachments: [{ id: 'blob-1', name: 'counts.csv' }], references: [] },
    });
    const section = (
      await screen.findByRole('heading', { name: 'In your message, not sent yet' })
    ).closest('section') as HTMLElement;
    expect(within(section).getByText('counts.csv')).toBeInTheDocument();
    await waitFor(() => expect(within(section).getByText('2 KB')).toBeInTheDocument());
    // The chip's × is the action; here there is none, and it is not also "Uploaded, not sent".
    expect(within(section).queryByRole('button')).toBeNull();
    expect(screen.queryByRole('heading', { name: filesCopy.uploadedNotSent })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Attach counts.csv' })).toBeNull();
    expect(screen.queryByText(filesCopy.noFiles)).toBeNull();
  });

  it('never calls a sent file “Uploaded, not sent”, or offers to attach it again (Q3-03)', async () => {
    // After Send: the composer is empty, the upload record is still listed, and the message
    // carrying the file has arrived.
    mocks.listTransfers.mockResolvedValue([
      transfer({ state: 'completed', offset: 2048, blob_id: 'blob-1' }),
    ]);
    renderTab({ messages: [message({ id: 'm1', attachments: ['blob-1'] })] });
    const section = (await screen.findByRole('heading', { name: 'In this channel' })).closest(
      'section'
    ) as HTMLElement;
    expect(await within(section).findByText('shared.csv')).toBeInTheDocument();
    await waitFor(() => expect(mocks.listTransfers).toHaveBeenCalled());
    expect(screen.queryByRole('heading', { name: filesCopy.uploadedNotSent })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Attach counts.csv' })).toBeNull();
    expect(screen.queryByRole('heading', { name: filesCopy.inYourMessage })).toBeNull();
  });

  it('says who shared each file in the channel and when (Q3-13)', async () => {
    const posted = new Date();
    posted.setHours(18, 54, 0, 0);
    renderTab({
      snapshot: {
        ...crewTestController().snapshot!,
        principals: [
          { id: 'person-1', uid: 1000, username: 'alice', nickname: 'Alice' },
          { id: 'person-2', uid: 1001, username: 'bob', nickname: 'Bob Lee' },
        ] as never,
      },
      messages: [
        message({
          id: 'm1',
          actor_id: 'person-2',
          attachments: ['blob-9'],
          created_at: Math.floor(posted.getTime() / 1000),
        }),
      ],
    });
    const section = (await screen.findByRole('heading', { name: 'In this channel' })).closest(
      'section'
    ) as HTMLElement;
    const row = (await within(section).findByText('shared.csv')).closest('li') as HTMLElement;
    expect(row).toHaveTextContent(/Shared by Bob Lee.* · 6:54 PM/);
    expect(row.textContent).not.toMatch(/person-2/);
  });

  it('lists the files and server paths shared in the loaded messages, once each', async () => {
    renderTab({
      messages: [
        message({ id: 'm1', attachments: ['blob-9'] }),
        message({ id: 'm2', attachments: ['blob-9'], references: ['ref-1'] }),
      ],
    });
    const section = (await screen.findByRole('heading', { name: 'In this channel' })).closest(
      'section'
    ) as HTMLElement;
    expect(await within(section).findByText('shared.csv')).toBeInTheDocument();
    expect(within(section).getAllByText('shared.csv')).toHaveLength(1);
    expect(await within(section).findByText('Run 7')).toBeInTheDocument();
    expect(within(section).getByText('Not uploaded')).toBeInTheDocument();
    expect(within(section).getByRole('button', { name: 'Copy server path' })).toBeInTheDocument();
  });
});
