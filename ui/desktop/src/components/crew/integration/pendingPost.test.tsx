import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { composerCopy } from '../composer/copy';
import { POST_CHECK_TIMEOUT_MS } from '../state/crewSend';
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
    // Send is held, and says so; the words stay writable (QA M8).
    expect(again).not.toHaveAttribute('readonly');
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
    await waitFor(() =>
      expect(screen.getByRole('button', { name: composerCopy.send })).not.toHaveAttribute(
        'aria-busy'
      )
    );
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
    await waitFor(() =>
      expect(screen.getByRole('button', { name: composerCopy.send })).not.toHaveAttribute(
        'aria-busy'
      )
    );
    // The words stay where the person was told they are kept; nothing stands in for a post.
    expect(again).toHaveValue(sent);
    expect(pendingRow()).toBeNull();
  });
});

/**
 * QA R-4: a post the broker committed while the bridge died on the way back was reported as
 * "Couldn't send. Crew SSH failure […]" beside the message in the timeline, and a resend under the
 * same key (which the broker answers with that message) drew a dimmed "Sending…" ghost under it.
 */
describe('a post whose outcome is unknown (QA R-4)', () => {
  const words = 'Did this reach you?';
  const landed: CrewMessage = {
    ...earlier,
    id: ids.messages[2],
    sequence: ids.messages[2],
    actor_id: ids.alice,
    body: words,
    created_at: Math.floor(Date.now() / 1000),
  };
  const lost = new CrewHttpError(
    'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: SSH connection closed; reconnect. Submitted operation outcome may be unknown; inspect history before retrying',
    400,
    'crew_request_refused'
  );

  /** Every post: the first is lost after it was written, the rest answer with the landed message. */
  function loseFirstPost(failure: unknown = lost) {
    const keys: string[] = [];
    daemon.state.request = (method, params) => {
      if (method !== 'message.post') return undefined;
      keys.push(String(params.idempotency_key));
      return keys.length === 1 ? Promise.reject(failure) : landed;
    };
    return keys;
  }

  async function sendWords() {
    renderCrew();
    const box = await channelReady();
    fireEvent.change(box, { target: { value: words } });
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    await screen.findByText(composerCopy.checking);
    return box;
  }

  const deliver = (messages: CrewMessage[]) =>
    act(() =>
      daemon.emit({
        type: 'messages',
        channel_id: ids.general,
        messages,
        cursor: messages[messages.length - 1].sequence,
      })
    );

  it('checks the channel instead of saying it failed, and says it was sent once it is there', async () => {
    loseFirstPost();
    const box = await sendWords();
    expect(screen.queryByText(composerCopy.sendErrorLead)).toBeNull();
    expect(screen.queryByText(/Crew SSH failure/)).toBeNull();
    expect(box).toHaveValue(words);

    deliver([landed]);
    expect(await screen.findByText(composerCopy.confirmed)).toBeInTheDocument();
    expect(box).toHaveValue('');
    expect(pendingRow()).toBeNull();
    expect(screen.queryByText(composerCopy.checking)).toBeNull();
  });

  /** Send the words, lose the post, and let the channel be read twice without finding it. */
  async function unconfirmed(keys: string[]) {
    const box = await sendWords();
    act(() => daemon.emitState());
    expect(screen.getByText(composerCopy.checking)).toBeInTheDocument();
    act(() => daemon.emitState());
    expect(await screen.findByText(composerCopy.unconfirmed)).toBeInTheDocument();
    expect(box).toHaveValue(words);
    expect(keys).toHaveLength(1);
    return box;
  }

  it('says it could not confirm it once the channel was read again, and sends it again safely', async () => {
    const keys = loseFirstPost(
      new CrewHttpError(
        'Crew couldn’t confirm whether this reached lab.',
        503,
        'crew_outcome_unknown'
      )
    );
    const box = await unconfirmed(keys);
    // Sending again is the same message to the broker, which answers with the one it has.
    fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
    await waitFor(() => expect(keys).toHaveLength(2));
    expect(keys[1]).toBe(keys[0]);
    await waitFor(() => expect(box).toHaveValue(''));
    expect(pendingRow()).toHaveTextContent(words);
    deliver([landed]);
    await waitFor(() => expect(pendingRow()).toBeNull());
    expect(within(screen.getByRole('log')).getAllByText(words)).toHaveLength(1);
  });

  /**
   * W2-UIC-6: the time bound was remembered by channel. Once one check there had timed out, the
   * next post in doubt in that channel was called unconfirmed after a single view, not after its
   * own views or its own 30 s, which invites the edit-and-resend duplicate R-4 is there to prevent.
   */
  it('gives a second post in doubt in the same channel its own time, after the first one’s ran out', async () => {
    const keys: string[] = [];
    daemon.state.request = (method, params) => {
      if (method !== 'message.post') return undefined;
      keys.push(String(params.idempotency_key));
      return Promise.reject(
        new CrewHttpError(
          'Crew couldn’t confirm whether this reached lab.',
          503,
          'crew_outcome_unknown'
        )
      );
    };
    const now = Date.now();
    const clock = vi.spyOn(Date, 'now').mockReturnValue(now);
    try {
      const box = await sendWords();
      // Its 30 s pass with the channel read only once: the time bound decides it.
      clock.mockReturnValue(now + POST_CHECK_TIMEOUT_MS + 1_000);
      act(() => daemon.emitState());
      expect(await screen.findByText(composerCopy.unconfirmed)).toBeInTheDocument();

      // Other words, and that post is in doubt too.
      fireEvent.change(box, { target: { value: 'Different words, sent again' } });
      fireEvent.click(screen.getByRole('button', { name: composerCopy.send }));
      await waitFor(() => expect(keys).toHaveLength(2));
      expect(keys[1]).not.toBe(keys[0]);
      expect(await screen.findByText(composerCopy.checking)).toBeInTheDocument();
      // One view later it is still being checked: the first check's time is not this one's.
      act(() => daemon.emitState());
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 20));
      });
      expect(screen.getByText(composerCopy.checking)).toBeInTheDocument();
      expect(screen.queryByText(composerCopy.unconfirmed)).toBeNull();
      // Its own views decide it.
      act(() => daemon.emitState());
      expect(await screen.findByText(composerCopy.unconfirmed)).toBeInTheDocument();
      expect(box).toHaveValue('Different words, sent again');
    } finally {
      clock.mockRestore();
    }
  });

  it('still says it was sent when the message turns up after it could not be confirmed', async () => {
    const keys = loseFirstPost();
    const box = await unconfirmed(keys);
    deliver([landed]);
    expect(await screen.findByText(composerCopy.confirmed)).toBeInTheDocument();
    expect(box).toHaveValue('');
    expect(screen.queryByText(composerCopy.unconfirmed)).toBeNull();
  });
});
