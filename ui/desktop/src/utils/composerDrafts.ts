import type { DroppedFile } from '../hooks/useFileDrop';

/**
 * What a NEW chat's composer holds while nobody has sent it — text, staged
 * images and dropped files — kept for the TAB it was typed into (or for Home),
 * so it outlives the composer that shows it.
 *
 * Existing chats use a separate tab+session namespace in this same renderer-memory
 * store. Their drafts survive tab switches but are released when the tab closes
 * or binds to a different session; they never enter sessionless-tab adoption.
 *
 * A fresh tab's composer is rebuilt far more often than a person would guess,
 * and every rebuild used to start empty:
 *
 *   • a failed start. `BaseChat` moves its composer between the centred empty
 *     state and the bar under a transcript as `isCreatingSession` flips, which
 *     remounts it — on the way up AND on the way down;
 *   • switching tabs. Only a pane's ACTIVE tab mounts a `BaseChat`;
 *   • splitting, collapsing or dragging a tab into another pane. The pane tree is
 *     keyed by its shape (`branch-${path}`, a new group), so the `BaseChat` of a
 *     tab that did not itself move is rebuilt too;
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
 * by `composerDraftKeyForTab`, `existingChatComposerDraftKey`, or Home's key, and nothing reaches a
 * composer except under the key it was mounted with.
 *
 * THE STORE IS WRITTEN ON EVERY CHANGE, NOT ON THE WAY OUT. A replacement
 * composer reads its seed while it RENDERS, and React renders the new tree
 * before it commits the old one's unmount — so a composer that only saved as it
 * went handed its replacement whatever it had held at its PREVIOUS save.
 * Measured on the first version of this module, in the production bundle: a
 * split showed "PROD A" over "PROD A PROD B", collapsing it brought back an
 * older copy over newer text, and a new tab dragged into a new pane arrived
 * empty. Every write carries a stamp (`composerDraftVersion`), so a composer
 * mounted after a write it did not see adopts it rather than saving its stale
 * seed over it.
 *
 * LIFETIME — definite events, never a clock:
 *   • the composer SAVES what it holds whenever that changes and whenever a
 *     give-back lands on it. At the instant it hands a message to a send it
 *     clears the draft and marks the key SENDING until the send answers
 *     (`beginComposerSend`) — so a start that succeeds leaves nothing behind to
 *     come back, and the tab a message is still in flight from survives a trip
 *     to Settings;
 *   • `ChatGroupsProvider` RETAINS only the keys of tabs that exist and have no
 *     chat. Closing the tab, or the tab binding to the chat its message started,
 *     drops the draft and deletes the temp images it owned. Home's key is one
 *     entry, spent by Home's own sends;
 *   • a tab holding a draft is never given to another chat: a started chat binds
 *     to the tab its message was typed in, and the reducer never adopts a tab
 *     that holds a draft (`OpenTabPayload.originTabId` / `unsentTabs`);
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
 * WHY IT CANNOT GROW: at most one entry per sessionless tab or existing tab/session
 * pair, plus Home's, each one message's text, at most the composer's per-message
 * image cap, and its dropped files. An empty draft is deleted, not stored. The
 * stamps and send marks are one number per such key.
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
const EXISTING_CHAT_KEY_PREFIX = 'existing-chat:';

export function existingChatComposerDraftKey(tabId: string, sessionId: string): string {
  return `${EXISTING_CHAT_KEY_PREFIX}${JSON.stringify([tabId, sessionId])}`;
}

/** The key a chat TAB's composer is mounted with. The only tab constructor. */
export function composerDraftKeyForTab(tabId: string): string {
  return `${TAB_KEY_PREFIX}${tabId}`;
}

/**
 * Home's composer: one per window, left and come back to exactly as a tab is.
 * Without a key, a message whose start was still in flight when the person
 * clicked away was lost with the unmounted composer — and its staged image left
 * in the temp directory — under a toast saying it was kept.
 */
export const HOME_COMPOSER_DRAFT_KEY = 'home';

export function isEmptyComposerDraft(draft: ComposerDraft | undefined): boolean {
  return !draft || (!draft.text.trim() && draft.images.length === 0 && draft.files.length === 0);
}

function sameComposerDraft(a: ComposerDraft | undefined, b: ComposerDraft | undefined): boolean {
  if (isEmptyComposerDraft(a) || isEmptyComposerDraft(b)) {
    return isEmptyComposerDraft(a) && isEmptyComposerDraft(b);
  }
  const x = a as ComposerDraft;
  const y = b as ComposerDraft;
  return (
    x.text === y.text &&
    x.images.length === y.images.length &&
    x.images.every(
      (image, i) =>
        image.id === y.images[i].id &&
        image.filePath === y.images[i].filePath &&
        image.dataUrl === y.images[i].dataUrl
    ) &&
    x.files.length === y.files.length &&
    x.files.every((file, i) => file.id === y.files[i].id && file.path === y.files[i].path)
  );
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
/** Every write stamps its key with the next number; a key never written is 0. */
const versions = new Map<string, number>();
let lastVersion = 0;
/** Keys with a message handed to a send that has not answered yet, and how many. */
const sending = new Map<string, Set<symbol>>();
const giveBackListeners = new Map<string, Set<(returned: ComposerDraft) => void>>();

function stamp(key: string): number {
  lastVersion += 1;
  versions.set(key, lastVersion);
  return lastVersion;
}

export function readComposerDraft(key: string): ComposerDraft | undefined {
  return drafts.get(key);
}

/**
 * The stamp of the last write under `key`. A composer remembers the stamp its
 * box reflects; a different one means someone else wrote since, so what the
 * store holds is newer than the composer's copy.
 */
export function composerDraftVersion(key: string): number {
  return versions.get(key) ?? 0;
}

export function hasComposerDraft(key: string): boolean {
  return !isEmptyComposerDraft(drafts.get(key));
}

export function isComposerSending(key: string): boolean {
  return (sending.get(key)?.size ?? 0) > 0;
}

/** A draft, or a message still in flight: either way, not a blank tab. */
export function holdsUnsentMessage(key: string): boolean {
  return hasComposerDraft(key) || isComposerSending(key);
}

/**
 * The composer's own copy of what it holds. An empty draft deletes the key, so
 * "this tab holds nothing unsent" and "this map never heard of the tab" are one
 * state. Writing what the store already holds is not a write and keeps its
 * stamp. Returns the key's stamp after the call.
 */
export function saveComposerDraft(key: string, draft: ComposerDraft): number {
  if (sameComposerDraft(drafts.get(key), draft)) return composerDraftVersion(key);
  if (isEmptyComposerDraft(draft)) drafts.delete(key);
  else drafts.set(key, draft);
  return stamp(key);
}

/**
 * Hand a message that was NOT sent back to the composer for `key`.
 *
 * Written into the map FIRST, synchronously, so a composer mounted after this —
 * the replacement a failed start builds, or the one a person comes back to —
 * finds it whatever order React and the failure land in. THEN each composer
 * listening under that key merges it into what it holds right now and saves
 * that.
 */
export function giveBackToComposer(key: string, returned: ComposerDraft): void {
  if (isEmptyComposerDraft(returned)) return;
  saveComposerDraft(key, mergeComposerDraft(drafts.get(key), returned));
  for (const listener of [...(giveBackListeners.get(key) ?? [])]) listener(returned);
}

export type ComposerSend = {
  /** The send answered and there is nothing to hand back. Idempotent. */
  settle(): void;
  /** The send did not take the message: hand it back under the key, then settle. */
  giveBack(returned: ComposerDraft): void;
};

/**
 * A composer is handing its message to a send. The draft is cleared — the
 * message now belongs to the send, and a start that succeeds carries it to the
 * new chat as cargo, so a draft still holding it would hand it back as well —
 * and the key is marked SENDING until the send answers.
 *
 * The mark is what keeps the TAB. A person who clicks Settings while a start is
 * in flight and comes back before it fails must find the tab the message will be
 * handed back to. Measured on the first version of this module: the tab had
 * already been pruned, a blank one opened, the late give-back went under a key no
 * tab had, and the next layout change deleted it and its image.
 */
export function beginComposerSend(key: string): ComposerSend {
  saveComposerDraft(key, EMPTY_COMPOSER_DRAFT);
  const token = Symbol();
  const tokens = sending.get(key) ?? new Set<symbol>();
  tokens.add(token);
  sending.set(key, tokens);
  let open = true;
  const settle = () => {
    if (!open) return;
    open = false;
    const current = sending.get(key);
    if (!current?.delete(token)) return;
    if (current.size === 0) sending.delete(key);
  };
  return {
    settle,
    giveBack(returned) {
      if (!open) return;
      if (sending.get(key)?.has(token)) giveBackToComposer(key, returned);
      else for (const image of returned.images) window.electron?.deleteTempFile(image.filePath);
      settle();
    },
  };
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
 * Which tabs are not blank, for deciding where a started chat goes: `drafted`
 * hold text, images or files the person has not sent; `sending` have a message
 * in flight.
 */
export function unsentComposerTabs(): { drafted: string[]; sending: string[] } {
  const tabIdsOf = (keys: string[]) =>
    keys
      .filter((key) => key.startsWith(TAB_KEY_PREFIX))
      .map((key) => key.slice(TAB_KEY_PREFIX.length));
  return {
    drafted: tabIdsOf(
      [...drafts].filter(([, draft]) => !isEmptyComposerDraft(draft)).map(([key]) => key)
    ),
    sending: tabIdsOf([...sending].filter(([, tokens]) => tokens.size > 0).map(([key]) => key)),
  };
}

/**
 * Keep the drafts of these tabs — the ones that exist and have no chat — and no
 * other tab's. Anything else under the tab namespace is gone for good (closed,
 * or bound to the chat its message started), so its draft is dropped and the
 * temp images it owned are deleted. Dropped files are never deleted: their
 * paths are the user's own files. Home's key is not a tab's and is untouched.
 */
export function retainTabComposerDrafts(sessionlessTabIds: Iterable<string>): void {
  const live = new Set([...sessionlessTabIds].map(composerDraftKeyForTab));
  retainDraftKeys(TAB_KEY_PREFIX, live);
}

/** Existing chats keep drafts per tab; rebinding a tab must never carry its old chat's draft. */
export function retainExistingChatComposerDrafts(
  tabs: Iterable<{ tabId: string; sessionId: string }>
): void {
  retainDraftKeys(
    EXISTING_CHAT_KEY_PREFIX,
    new Set([...tabs].map((tab) => existingChatComposerDraftKey(tab.tabId, tab.sessionId)))
  );
}

function retainDraftKeys(prefix: string, live: ReadonlySet<string>): void {
  const keys = new Set([...drafts.keys(), ...versions.keys(), ...sending.keys()]);
  for (const key of keys) {
    if (!key.startsWith(prefix) || live.has(key)) continue;
    const draft = drafts.get(key);
    drafts.delete(key);
    versions.delete(key);
    sending.delete(key);
    for (const image of draft?.images ?? []) window.electron?.deleteTempFile(image.filePath);
  }
}

export function resetComposerDraftsForTests(): void {
  drafts.clear();
  versions.clear();
  sending.clear();
  lastVersion = 0;
  giveBackListeners.clear();
}
