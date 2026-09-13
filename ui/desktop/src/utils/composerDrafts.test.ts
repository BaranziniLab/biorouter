import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  EMPTY_COMPOSER_DRAFT,
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
