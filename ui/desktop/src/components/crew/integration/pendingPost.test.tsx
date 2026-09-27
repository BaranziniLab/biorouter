import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import type { CrewMessage } from '../crewApi';
import { clearBlobCache } from '../files/blobMetadataCache';
import { filesCopy } from '../files/copy';
import { installResizeObserverStub } from '../test/crewTestUtils';
import { timelineCopy } from '../timeline/copy';
import {
  channelReady,
  currentCrew,
  ids,
  installDaemon,
  mocked,
  renderCrew,
  richSnapshot,
  type ScriptedDaemon,
} from './harness';

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
// Stable across renders, as the real context's callbacks are.
const config = vi.hoisted(() => ({
  getProviders: async () => [],
  read: async () => '',
  getProviderModels: async () => [],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

/**
 * A post with a file, from Send to its arrival, in the real layout (live QA round 4):
 *
 * - Q4-19: the "Sending…" row had no file card and no "Today" band, so when the message landed the
 *   band came in above it, the card appeared under it, and the post jumped 57–70px.
 * - Q4-17: Files dropped the file from "In your message" the moment Send emptied the draft, and it
 *   was in no section for a second or two until the message came; then its card read a nameless
 *   "Attachment" while it asked for the file again.
 *
 * Every message before the post is from yesterday, so the post is the day's first.
 */

const yesterday = Math.floor(Date.now() / 1000) - 26 * 60 * 60;
const earlier: CrewMessage = {
  id: ids.messages[0],
  sequence: ids.messages[0],
  channel_id: ids.general,
  actor_id: ids.bob,
  body: 'Plate map for Friday?',
  created_at: yesterday,
  restricted: false,
  source_channels: [ids.general],
  attachments: [],
};

const sent = 'Here it is.';
const posted: CrewMessage = {
  ...earlier,
  id: ids.messages[1],
  sequence: ids.messages[1],
  actor_id: ids.alice,
  body: sent,
  created_at: Math.floor(Date.now() / 1000),
  attachments: [ids.blob],
};

let daemon: ScriptedDaemon;

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  clearBlobCache();
  daemon = installDaemon({
    snapshot: richSnapshot({ pending_joins: [] }),
    messages: [earlier],
  });
});

/** Open the channel, attach the file to a draft, open Files, and press Send. */
async function sendWithFile() {
  renderCrew();
  const composer = await channelReady();
  act(() => currentCrew().openPane({ mode: 'details', tab: 'files' }));
  act(() => currentCrew().addAttachment({ id: ids.blob, name: 'plate-map.csv' }));
  fireEvent.change(composer, { target: { value: sent } });
  const inMessage = await screen.findByRole('heading', { name: filesCopy.inYourMessage });
  expect(
    within(inMessage.closest('section') as HTMLElement).getByText('plate-map.csv')
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
  await waitFor(() =>
    expect(mocked.crewRequest.mock.calls.some(([, method]) => method === 'message.post')).toBe(true)
  );
  await waitFor(() => expect(composer).toHaveValue(''));
}

const pendingRow = () => document.querySelector<HTMLElement>('.crew-pending-row');

describe('a post with a file, from Send to its arrival', () => {
  it('stands in with its card and today’s band, so nothing appears when it lands (Q4-19)', async () => {
    await sendWithFile();
    const row = pendingRow() as HTMLElement;
    expect(row).toHaveTextContent(sent);
    expect(within(row).getByText(timelineCopy.sending)).toBeInTheDocument();
    // Its file, as a card with no controls, in the box the posted card will take.
    const card = row.querySelector<HTMLElement>('.crew-attachment-card[data-sending="true"]');
    expect(card).not.toBeNull();
    await waitFor(() => expect(card).toHaveTextContent('counts.csv'));
    expect(within(row).queryByRole('button', { hidden: true })).toBeNull();
    // The day's band over it: the earlier message is yesterday's.
    const band = row.closest('.crew-day')?.querySelector('.crew-day-label');
    expect(band).toHaveTextContent(timelineCopy.today);

    act(() =>
      daemon.emit({
        type: 'messages',
        channel_id: ids.general,
        messages: [posted],
        cursor: posted.sequence,
      })
    );
    await waitFor(() => expect(pendingRow()).toBeNull());
    const log = screen.getByRole('log');
    // One band for today, and the posted card is named at once: its answer was kept (Q4-17).
    expect(within(log).getAllByRole('separator', { name: timelineCopy.today })).toHaveLength(1);
    const article = within(log).getByText(sent).closest('article') as HTMLElement;
    expect(within(article).getByText('counts.csv')).toBeInTheDocument();
    expect(within(article).queryByText(filesCopy.attachment)).toBeNull();
  });

  it('keeps the file under “In your message” until the message carrying it is loaded (Q4-17)', async () => {
    await sendWithFile();
    // The draft is empty, and the file is still listed, as on its way.
    const heading = screen.getByRole('heading', { name: filesCopy.inYourMessage });
    const section = heading.closest('section') as HTMLElement;
    const row = within(section).getByText('plate-map.csv').closest('li') as HTMLElement;
    expect(row).toHaveAttribute('data-sending', 'true');
    expect(within(row).getByText(filesCopy.sending)).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: filesCopy.uploadedNotSent })).toBeNull();

    act(() =>
      daemon.emit({
        type: 'messages',
        channel_id: ids.general,
        messages: [posted],
        cursor: posted.sequence,
      })
    );
    const inChannel = await screen.findByRole('heading', { name: filesCopy.inThisChannel });
    expect(screen.queryByRole('heading', { name: filesCopy.inYourMessage })).toBeNull();
    const shared = inChannel.closest('section') as HTMLElement;
    // Named from the kept answer, never "Attachment" while it asks again.
    expect(within(shared).getByText('counts.csv')).toBeInTheDocument();
    expect(within(shared).queryByText(filesCopy.attachment)).toBeNull();
  });
});
