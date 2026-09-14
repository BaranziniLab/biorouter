import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  beginQueuedOffer,
  claimComposerQueue,
  composerQueueKey,
  endQueuedOffer,
  hasQueuedOfferInFlight,
  parkComposerQueue,
  readParkedComposerQueue,
  resetComposerQueuesForTests,
  retainComposerQueues,
  returnQueuedOffer,
  subscribeQueuedOfferReturns,
  type QueuedMessage,
} from './composerQueues';

const message = (id: string, owned: string[] = []): QueuedMessage => ({
  id,
  content: `message ${id}`,
  attachments: owned.map((path) => ({ path, kind: 'image' as const })),
  ownedTempAttachmentPaths: owned,
  timestamp: 1,
});

const idle = (messages: QueuedMessage[], sendWhenIdle = false) => ({
  messages,
  paused: false,
  interruption: null,
  sendWhenIdle,
});

const deleteTempFile = vi.fn();

beforeEach(() => {
  resetComposerQueuesForTests();
  deleteTempFile.mockReset();
  Object.assign(window, { electron: { ...(window.electron ?? {}), deleteTempFile } });
});

describe('composerQueueKey', () => {
  it('names a chat, and nothing for a composer that has none', () => {
    expect(composerQueueKey('s1')).toBe('chat:s1');
    expect(composerQueueKey(null)).toBeNull();
    expect(composerQueueKey('')).toBeNull();
  });
});

describe('parking and claiming', () => {
  it('hands the parked queue to exactly one claimer', () => {
    const key = composerQueueKey('s1')!;
    parkComposerQueue(key, idle([message('a'), message('b')], true));

    expect(claimComposerQueue(key)).toEqual(idle([message('a'), message('b')], true));
    expect(claimComposerQueue(key)).toBeUndefined();
  });

  it('keeps what was already parked ahead, without duplicates', () => {
    const key = composerQueueKey('s1')!;
    returnQueuedOffer(key, message('returned'));
    parkComposerQueue(key, idle([message('a'), message('returned')]));

    const parked = readParkedComposerQueue(key)!;
    expect(parked.messages.map((m) => m.id)).toEqual(['returned', 'a']);
    // A message handed back mid-drain was due to be sent; parking does not undo that.
    expect(parked.sendWhenIdle).toBe(true);
  });

  it('parks nothing for an empty queue', () => {
    const key = composerQueueKey('s1')!;
    parkComposerQueue(key, idle([]));
    expect(readParkedComposerQueue(key)).toBeUndefined();
  });

  it("never gives one chat's queue to another", () => {
    parkComposerQueue(composerQueueKey('s1')!, idle([message('a')]));
    expect(claimComposerQueue(composerQueueKey('s2')!)).toBeUndefined();
    expect(readParkedComposerQueue(composerQueueKey('s1')!)?.messages).toHaveLength(1);
  });
});

describe('a message a gone composer could not send', () => {
  it('goes to the composer mounted for the chat, and is not also parked', () => {
    const key = composerQueueKey('s1')!;
    const received: string[] = [];
    const unsubscribe = subscribeQueuedOfferReturns(key, (m) => received.push(m.id));

    returnQueuedOffer(key, message('a'));

    expect(received).toEqual(['a']);
    expect(readParkedComposerQueue(key)).toBeUndefined();
    unsubscribe();
  });

  it('waits at the head of the parked queue when no composer is mounted', () => {
    const key = composerQueueKey('s1')!;
    parkComposerQueue(key, idle([message('b')]));

    returnQueuedOffer(key, message('a'));

    expect(readParkedComposerQueue(key)!.messages.map((m) => m.id)).toEqual(['a', 'b']);
  });

  it('reaches only the chat it was queued in', () => {
    const received: string[] = [];
    const unsubscribe = subscribeQueuedOfferReturns(composerQueueKey('s2')!, (m) =>
      received.push(m.id)
    );
    returnQueuedOffer(composerQueueKey('s1')!, message('a'));
    expect(received).toEqual([]);
    unsubscribe();
  });
});

describe('offers in flight', () => {
  it('are tracked per chat until their submit answers', () => {
    const key = composerQueueKey('s1')!;
    beginQueuedOffer(key, 'a');
    expect(hasQueuedOfferInFlight(key)).toBe(true);
    expect(hasQueuedOfferInFlight(composerQueueKey('s2')!)).toBe(false);
    endQueuedOffer(key, 'a');
    expect(hasQueuedOfferInFlight(key)).toBe(false);
  });
});

describe('retainComposerQueues', () => {
  it('drops the queues of chats no tab shows, deleting only the temp images they owned', () => {
    parkComposerQueue(composerQueueKey('open')!, idle([message('a', ['/tmp/open.png'])]));
    parkComposerQueue(composerQueueKey('closed')!, idle([message('b', ['/tmp/closed.png'])]));

    retainComposerQueues(['open']);

    expect(readParkedComposerQueue(composerQueueKey('open')!)).toBeDefined();
    expect(readParkedComposerQueue(composerQueueKey('closed')!)).toBeUndefined();
    expect(deleteTempFile).toHaveBeenCalledTimes(1);
    expect(deleteTempFile).toHaveBeenCalledWith('/tmp/closed.png');
  });
});
