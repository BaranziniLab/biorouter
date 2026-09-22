import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  EMPTY_COMPOSER_DRAFT,
  existingChatComposerDraftKey,
  retainExistingChatComposerDrafts,
  HOME_COMPOSER_DRAFT_KEY,
  beginComposerSend,
  composerDraftVersion,
  holdsUnsentMessage,
  isComposerSending,
  unsentComposerTabs,
  composerDraftKeyForTab,
  giveBackToComposer,
  hasComposerDraft,
  mergeComposerDraft,
  readComposerDraft,
  resetComposerDraftsForTests,
  retainTabComposerDrafts,
  saveComposerDraft,
  subscribeComposerGiveBack,
  type ComposerDraft,
} from './composerDrafts';
import type { DroppedFile } from '../hooks/useFileDrop';

const deleteTempFile = vi.fn();

beforeEach(() => {
  resetComposerDraftsForTests();
  deleteTempFile.mockReset();
  Object.assign(window, { electron: { deleteTempFile } });
});

const image = (n: number) => ({ id: `img-${n}`, filePath: `/tmp/p-${n}.png`, dataUrl: 'data:x' });
const file = (id: string): DroppedFile => ({
  id,
  path: `/Users/me/${id}`,
  name: id,
  type: '',
  isImage: false,
});
const draft = (text: string, extra: Partial<ComposerDraft> = {}): ComposerDraft => ({
  text,
  images: [],
  files: [],
  ...extra,
});

describe('mergeComposerDraft — a give-back never replaces what the box holds', () => {
  it('fills an empty box with the returned message', () => {
    expect(mergeComposerDraft(undefined, draft('sent'))).toEqual(draft('sent'));
    expect(mergeComposerDraft(draft('  '), draft('sent')).text).toBe('sent');
  });

  it('keeps text typed since, after the returned message', () => {
    expect(mergeComposerDraft(draft('typed since'), draft('sent')).text).toBe(
      'sent\n\ntyped since'
    );
  });

  it('does not double identical text, or keep an empty give-back over real text', () => {
    expect(mergeComposerDraft(draft('same'), draft('same')).text).toBe('same');
    expect(mergeComposerDraft(draft('mine'), draft('', { images: [image(1)] })).text).toBe('mine');
  });

  it('stages each image and file once', () => {
    const merged = mergeComposerDraft(
      draft('', { images: [image(1), image(2)], files: [file('a')] }),
      draft('', { images: [image(2), image(3)], files: [file('a'), file('b')] })
    );
    expect(merged.images.map((i) => i.id)).toEqual(['img-2', 'img-3', 'img-1']);
    expect(merged.files.map((f) => f.id)).toEqual(['a', 'b']);
  });
});

describe('the store', () => {
  it('deletes an empty draft rather than storing it', () => {
    const key = composerDraftKeyForTab('tab-1');
    saveComposerDraft(key, draft('x'));
    expect(hasComposerDraft(key)).toBe(true);
    saveComposerDraft(key, EMPTY_COMPOSER_DRAFT);
    expect(hasComposerDraft(key)).toBe(false);
    expect(readComposerDraft(key)).toBeUndefined();
  });

  it('gives back to the store FIRST, then to the composers under that key only', () => {
    const mine = composerDraftKeyForTab('tab-1');
    const other = composerDraftKeyForTab('tab-2');
    const heardMine = vi.fn();
    const heardOther = vi.fn();
    subscribeComposerGiveBack(mine, (returned) => {
      // A composer mounted after this reads the store; it must already hold it.
      expect(readComposerDraft(mine)?.text).toBe('sent');
      heardMine(returned);
    });
    subscribeComposerGiveBack(other, heardOther);
    saveComposerDraft(other, draft('the other tab'));

    giveBackToComposer(mine, draft('sent'));

    expect(heardMine).toHaveBeenCalledWith(draft('sent'));
    expect(heardOther).not.toHaveBeenCalled();
    expect(readComposerDraft(other)?.text).toBe('the other tab');
  });

  it('merges a give-back into a draft nobody is showing (a tab not in view)', () => {
    const key = composerDraftKeyForTab('tab-1');
    saveComposerDraft(key, draft('typed while it failed'));
    giveBackToComposer(key, draft('sent'));
    expect(readComposerDraft(key)?.text).toBe('sent\n\ntyped while it failed');
  });

  it('stops telling a composer that unsubscribed', () => {
    const key = composerDraftKeyForTab('tab-1');
    const heard = vi.fn();
    const unsubscribe = subscribeComposerGiveBack(key, heard);
    unsubscribe();
    giveBackToComposer(key, draft('sent'));
    expect(heard).not.toHaveBeenCalled();
  });
});

describe('retainTabComposerDrafts — bounded by the tabs that exist and have no chat', () => {
  it('drops a closed or bound tab’s draft and deletes the images it owned, not its files', () => {
    const kept = composerDraftKeyForTab('tab-1');
    const gone = composerDraftKeyForTab('tab-2');
    saveComposerDraft(kept, draft('still here', { images: [image(1)] }));
    saveComposerDraft(gone, draft('closed', { images: [image(2)], files: [file('a')] }));

    retainTabComposerDrafts(['tab-1']);

    expect(hasComposerDraft(kept)).toBe(true);
    expect(hasComposerDraft(gone)).toBe(false);
    expect(deleteTempFile).toHaveBeenCalledTimes(1);
    expect(deleteTempFile).toHaveBeenCalledWith('/tmp/p-2.png');
  });

  it('leaves keys outside the tab namespace alone', () => {
    saveComposerDraft('elsewhere', draft('not a tab'));
    retainTabComposerDrafts([]);
    expect(hasComposerDraft('elsewhere')).toBe(true);
  });
});

describe('stamps — how a composer knows the store moved on without it (D2)', () => {
  it('stamps every real write, and not a write of what is already there', () => {
    const key = composerDraftKeyForTab('stamp');
    expect(composerDraftVersion(key)).toBe(0);
    const first = saveComposerDraft(key, draft('a'));
    expect(first).toBeGreaterThan(0);
    expect(saveComposerDraft(key, draft('a'))).toBe(first);
    const second = saveComposerDraft(key, draft('ab'));
    expect(second).toBeGreaterThan(first);
    expect(composerDraftVersion(key)).toBe(second);
  });

  it('a give-back moves the stamp even with no composer listening', () => {
    const key = composerDraftKeyForTab('stamp-gb');
    const before = saveComposerDraft(key, draft('typed'));
    giveBackToComposer(key, draft('returned'));
    expect(composerDraftVersion(key)).toBeGreaterThan(before);
  });
});

describe('beginComposerSend — a message in flight (D4)', () => {
  it('empties the draft and marks the key until the send answers', () => {
    const key = composerDraftKeyForTab('fly');
    saveComposerDraft(key, draft('about to send', { images: [image(1)] }));

    const send = beginComposerSend(key);

    expect(hasComposerDraft(key)).toBe(false);
    expect(isComposerSending(key)).toBe(true);
    expect(holdsUnsentMessage(key)).toBe(true);
    expect(unsentComposerTabs()).toEqual({ drafted: [], sending: ['fly'] });

    send.settle();
    send.settle();
    expect(isComposerSending(key)).toBe(false);
    expect(holdsUnsentMessage(key)).toBe(false);
    // Taking the message is not deleting its image: the send carries it.
    expect(deleteTempFile).not.toHaveBeenCalled();
  });

  it('a refused send hands the message back under the key and settles', () => {
    const key = composerDraftKeyForTab('refused');
    const send = beginComposerSend(key);

    send.giveBack(draft('not taken', { images: [image(2)] }));

    expect(readComposerDraft(key)).toEqual(draft('not taken', { images: [image(2)] }));
    expect(isComposerSending(key)).toBe(false);
    expect(unsentComposerTabs()).toEqual({ drafted: ['refused'], sending: [] });
  });

  it('Home is not a tab: never listed, never released by the tab strip', () => {
    saveComposerDraft(HOME_COMPOSER_DRAFT_KEY, draft('home', { images: [image(3)] }));
    beginComposerSend(HOME_COMPOSER_DRAFT_KEY);
    saveComposerDraft(HOME_COMPOSER_DRAFT_KEY, draft('home again'));

    retainTabComposerDrafts([]);

    expect(unsentComposerTabs()).toEqual({ drafted: [], sending: [] });
    expect(readComposerDraft(HOME_COMPOSER_DRAFT_KEY)?.text).toBe('home again');
  });

  it('a closed tab takes its stamp and its in-flight mark with it', () => {
    const key = composerDraftKeyForTab('closed');
    saveComposerDraft(key, draft('x'));
    beginComposerSend(key);

    retainTabComposerDrafts([]);

    expect(composerDraftVersion(key)).toBe(0);
    expect(isComposerSending(key)).toBe(false);
  });
});

describe('existing-chat draft lifetime', () => {
  it('retains only open tab/session pairs and leaves sessionless ownership separate', () => {
    const key = existingChatComposerDraftKey('tab-a', 'session-a');
    const newChatKey = composerDraftKeyForTab('new-tab');
    saveComposerDraft(key, {
      text: 'quote and draft',
      images: [{ id: 'image', filePath: '/tmp/owned-quote.png', dataUrl: '' }],
      files: [],
    });
    saveComposerDraft(newChatKey, { text: 'new chat', images: [], files: [] });
    retainTabComposerDrafts(['new-tab']);
    retainExistingChatComposerDrafts([{ tabId: 'tab-a', sessionId: 'session-a' }]);
    expect(readComposerDraft(key)?.text).toBe('quote and draft');
    expect(deleteTempFile).not.toHaveBeenCalled();
    expect(unsentComposerTabs().drafted).toEqual(['new-tab']);
    retainExistingChatComposerDrafts([{ tabId: 'tab-a', sessionId: 'session-b' }]);
    expect(readComposerDraft(key)).toBeUndefined();
    expect(deleteTempFile).toHaveBeenCalledWith('/tmp/owned-quote.png');
    expect(readComposerDraft(newChatKey)?.text).toBe('new chat');
  });
});

describe('send ownership after an existing tab disappears', () => {
  it('discards a late failed send after close or rebinding instead of reviving an orphan', () => {
    const key = existingChatComposerDraftKey('tab', 'original');
    const send = beginComposerSend(key);
    retainExistingChatComposerDrafts([{ tabId: 'tab', sessionId: 'replacement' }]);
    send.giveBack(draft('late quote', { images: [image(4)] }));
    expect(readComposerDraft(key)).toBeUndefined();
    expect(isComposerSending(key)).toBe(false);
    expect(deleteTempFile).toHaveBeenCalledWith(image(4).filePath);
  });
  it('retains a failed send across ordinary tab switching and does not settle a newer owner', () => {
    const key = existingChatComposerDraftKey('tab', 'same-session');
    const gone = beginComposerSend(key);
    retainExistingChatComposerDrafts([]);
    const current = beginComposerSend(key);
    gone.giveBack(draft('old owner', { images: [image(5)] }));
    expect(isComposerSending(key)).toBe(true);
    expect(readComposerDraft(key)).toBeUndefined();
    retainExistingChatComposerDrafts([{ tabId: 'tab', sessionId: 'same-session' }]);
    current.giveBack(draft('current question'));
    expect(readComposerDraft(key)?.text).toBe('current question');
    expect(isComposerSending(key)).toBe(false);
  });
});
