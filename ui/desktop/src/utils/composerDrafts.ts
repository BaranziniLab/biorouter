import type { DroppedFile } from '../hooks/useFileDrop';

/**
 * What a NEW chat's composer holds while nobody has sent it — text, staged
 * images and dropped files — kept for the TAB it was typed into, so it outlives
 * the composer that shows it.
 *
 * A fresh tab's composer is rebuilt far more often than a person would guess,
 * and every rebuild used to start empty:
 *
 *   • a failed start. `BaseChat` moves its composer between the centred empty
 *     state and the bar under a transcript as `isCreatingSession` flips, which
 *     remounts it — on the way up AND on the way down;
 *   • switching tabs. Only a pane's ACTIVE tab mounts a `BaseChat`;
 *   • leaving /pair. `ChatGroupsProvider` is mounted under that route only and
 *     re-reads the layout from storage when it comes back, where a tab with no
 *     chat was pruned (`chatGroupsStorage.loadChatGroups`).
 *
 * WHAT THIS REPLACED. #303 parked the message in a module-level map keyed by
 * CHAT and deleted it one task later: a pre-session composer has no chat id, so
 * the key could not tell one pane's new tab from another's, and the timer lost
 * the retry. #305 moved it into `BaseChat`'s own state, which fixed the retry —
 * but component state dies on a tab switch and a route change, it carried text
 * only, and the give-back itself stayed a `restore-chat-input` BROADCAST that
 * every mounted fresh-tab composer matched (`'' === ''`). Measured in the dev
 * app on 1.90.4: a failed start in the left pane REPLACED the unsent draft in
 * the right one.
 *
 * SO THE IDENTITY IS THE TAB, and the channel is addressed. A key is made only
 * by `composerDraftKeyForTab`, and nothing reaches a composer except under the
 * key it was mounted with.
 *
 * LIFETIME — definite events, never a clock:
 *   • the composer SAVES what it holds on its way out (unmount, or its key
 *     changing) and whenever a give-back lands on it, and CLEARS its key at the
 *     instant it hands a message to a send — so a start that succeeds leaves
 *     nothing behind to come back;
 *   • `ChatGroupsProvider` RETAINS only the keys of tabs that exist and have no
 *     chat. Closing the tab, or the tab binding to the chat its message started,
 *     drops the draft and deletes the temp images it owned;
 *   • it is renderer memory. A RELOAD is a new renderer: nothing here survives
 *     it, and the provider's load prunes the tab exactly as it always did. That
 *     is deliberate. "A reload restores nothing" is the property #305 verified,
 *     a staged image is a temp file nothing promises to keep across a reload,
 *     and persisting unsent message text to disk is a privacy decision this does
 *     not get to make on the way past.
 *
 * WHY IT CANNOT LEAK: another window is another renderer and never sees this
 * map; a key names one tab, and a tab dragged into another pane is still that
 * tab, so its draft follows it; a draft is only ever handed to a composer
 * mounted under its own key.
 *
 * WHY IT CANNOT GROW: at most one entry per tab that exists and has no chat,
 * each one message's text, at most the composer's per-message image cap, and
 * its dropped files. An empty draft is deleted, not stored.
 */

/** A staged image the composer owns: a temp file it wrote (paste, annotation). */
export type DraftImage = {
  id: string;
  filePath: string;
  /** The preview; `''` when it has not been read back from the file yet. */
  dataUrl: string;
};

export type ComposerDraft = {
  text: string;
  images: DraftImage[];
  files: DroppedFile[];
};

export const EMPTY_COMPOSER_DRAFT: ComposerDraft = { text: '', images: [], files: [] };

const TAB_KEY_PREFIX = 'tab:';

/** The key a chat TAB's composer is mounted with. The only constructor. */
export function composerDraftKeyForTab(tabId: string): string {
  return `${TAB_KEY_PREFIX}${tabId}`;
}

export function isEmptyComposerDraft(draft: ComposerDraft | undefined): boolean {
  return !draft || (!draft.text.trim() && draft.images.length === 0 && draft.files.length === 0);
}

/**
 * What a composer holds once a message it did not just type is handed back.
 *
 * NEVER A REPLACEMENT. The give-back is the message this composer sent; what it
 * holds NOW is whatever the person typed while that send was failing. Both are
 * theirs, so both stay — the returned message first, because it was written
 * first. Identical text is not doubled, and an image or file already in the box
 * is not staged twice.
 *
 * Pure, and the one rule the store and a mounted composer both apply.
 */
export function mergeComposerDraft(
  current: ComposerDraft | undefined,
  returned: ComposerDraft
): ComposerDraft {
  const now = current ?? EMPTY_COMPOSER_DRAFT;
  let text: string;
  if (!now.text.trim()) text = returned.text;
  else if (!returned.text.trim() || now.text === returned.text) text = now.text;
  else text = `${returned.text}\n\n${now.text}`;

  const returnedPaths = new Set(returned.images.map((image) => image.filePath));
  const returnedFileIds = new Set(returned.files.map((file) => file.id));
  return {
    text,
    images: [
      ...returned.images,
      ...now.images.filter((image) => !returnedPaths.has(image.filePath)),
    ],
    files: [...returned.files, ...now.files.filter((file) => !returnedFileIds.has(file.id))],
  };
}

const drafts = new Map<string, ComposerDraft>();
const giveBackListeners = new Map<string, Set<(returned: ComposerDraft) => void>>();

export function readComposerDraft(key: string): ComposerDraft | undefined {
  return drafts.get(key);
}

export function hasComposerDraft(key: string): boolean {
  return !isEmptyComposerDraft(drafts.get(key));
}

/**
 * The composer's own copy of what it holds. An empty draft deletes the key, so
 * "this tab holds nothing unsent" and "this map never heard of the tab" are one
 * state.
 */
export function saveComposerDraft(key: string, draft: ComposerDraft): void {
  if (isEmptyComposerDraft(draft)) drafts.delete(key);
  else drafts.set(key, draft);
}

/**
 * Hand a message that was NOT sent back to the composer for `key`.
 *
 * Written into the map FIRST, synchronously, so a composer mounted after this —
 * the replacement a failed start builds — finds it whatever order React and the
 * failure land in. THEN each composer listening under that key merges it into
 * what it holds right now (newer than the map's copy by any keystrokes since it
 * last saved) and saves that.
 */
export function giveBackToComposer(key: string, returned: ComposerDraft): void {
  if (isEmptyComposerDraft(returned)) return;
  saveComposerDraft(key, mergeComposerDraft(drafts.get(key), returned));
  for (const listener of [...(giveBackListeners.get(key) ?? [])]) listener(returned);
}

export function subscribeComposerGiveBack(
  key: string,
  listener: (returned: ComposerDraft) => void
): () => void {
  const listeners = giveBackListeners.get(key) ?? new Set();
  listeners.add(listener);
  giveBackListeners.set(key, listeners);
  return () => {
    const current = giveBackListeners.get(key);
    current?.delete(listener);
    if (current?.size === 0) giveBackListeners.delete(key);
  };
}

/**
 * Keep the drafts of these tabs — the ones that exist and have no chat — and no
 * other tab's. Anything else under the tab namespace is gone for good (closed,
 * or bound to the chat its message started), so its draft is dropped and the
 * temp images it owned are deleted. Dropped files are never deleted: their
 * paths are the user's own files.
 */
export function retainTabComposerDrafts(sessionlessTabIds: Iterable<string>): void {
  const live = new Set([...sessionlessTabIds].map(composerDraftKeyForTab));
  for (const [key, draft] of [...drafts]) {
    if (!key.startsWith(TAB_KEY_PREFIX) || live.has(key)) continue;
    drafts.delete(key);
    for (const image of draft.images) window.electron?.deleteTempFile(image.filePath);
  }
}

export function resetComposerDraftsForTests(): void {
  drafts.clear();
  giveBackListeners.clear();
}
