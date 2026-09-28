import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { CrewHttpError, type CrewMessage } from '../crewApi';
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

/**
 * A post still on its way when the person leaves Crew and opens it again (RENDERER-4, U1). The
 * screen opened again gets the words back from the kept draft, and must treat the post as on its
 * way: Send held, no second post beside the first, no "Sending…" row for a draft it never sent,
 * and a retry under the key the words were first sent with.
 */
describe('a post still on its way when Crew is opened again', () => {
  function heldPost() {
    let answer!: { resolve: (value: unknown) => void; reject: (reason: unknown) => void };
    const first = new Promise<unknown>((resolve, reject) => {
      answer = { resolve, reject };
    });
    const keys: string[] = [];
    daemon.state.request = (method, params) => {
      if (method !== 'message.post') return undefined;
      keys.push(String(params.idempotency_key));
      return keys.length === 1 ? first : { sequence: `sequence-${keys.length}` };
    };
    return { answer, keys };
  }

  /** Send the words from #general, leave Crew while the post is out, and open Crew again. */
  async function sendThenReopen(keys: string[]) {
    const left = renderCrew();
    const composer = await channelReady();
    fireEvent.change(composer, { target: { value: sent } });
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    await waitFor(() => expect(keys).toHaveLength(1));
    left.unmount();
    renderCrew();
    const again = await channelReady();
    await waitFor(() => expect(again).toHaveValue(sent));
    return again;
  }

  it('holds Send, and sends the words again under their key once the post is refused', async () => {
    const { answer, keys } = heldPost();
    const again = await sendThenReopen(keys);
    expect(again).toHaveAttribute('readonly');
    expect(screen.getByRole('button', { name: composerCopy.send })).toHaveAttribute(
      'aria-busy',
      'true'
    );
    fireEvent.keyDown(again, { key: 'Enter', code: 'Enter', keyCode: 13 });
    expect(keys).toHaveLength(1);

    await act(async () => {
      answer.reject(new CrewHttpError('The computer did not answer in time', 504));
    });
    expect(await screen.findByText('The computer did not answer in time')).toBeInTheDocument();
    await waitFor(() => expect(again).not.toHaveAttribute('readonly'));
    expect(pendingRow()).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).toBe(keys[0]);
  });

  it('draws no “Sending…” row for a post it did not send, when that post is taken', async () => {
    const { answer, keys } = heldPost();
    const again = await sendThenReopen(keys);
    await act(async () => {
      answer.resolve({ sequence: 'sequence-1' });
    });
    await waitFor(() => expect(again).not.toHaveAttribute('readonly'));
    // The words stay where the person was told they are kept; nothing stands in for a post.
    expect(again).toHaveValue(sent);
    expect(pendingRow()).toBeNull();
  });
});
