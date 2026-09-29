import { createContext, useCallback, useContext, useState, useSyncExternalStore } from 'react';
import type Model from './modelInterface';

/**
 * W2-PRV-6 — the model a chat that has not been sent yet will start on.
 *
 * An unsent chat has no session, so its model chip used to open the dialog for
 * the only thing a switch without a session could change: the model NEW chats
 * start on, in every window. An unsent "New chat" tab looks like a chat,
 * though, so picking a model in it rewrote the app-wide default: the scratch
 * chat case privacy-tiers §14.3 P4 was meant to close, one step earlier.
 *
 * So `BaseChat` provides this for a chat with no session. Switch models then
 * offers the same "this chat" scope as a started chat (with the same "Also use
 * for new chats" box), and `choose` holds the pick. `BaseChat` applies it with
 * the per-chat `/agent/update_provider` right after `/agent/start` creates the
 * chat, before the first message is sent. Home provides nothing, so its chip
 * keeps its explicit new-chats scope.
 */
export interface PendingChatModel {
  /** Hold `model` as this unsent chat's own model. */
  choose: (model: Model) => void;
  /**
   * T3-SH-2. The chat tab this unsent chat lives in, or `undefined` where there
   * is none. "Use other provider" carries it to the provider catalog, which is a
   * different route: the chat's `BaseChat` is unmounted while the catalog is up,
   * so the catalog's model step cannot call `choose`. It holds the pick under
   * this tab instead ({@link holdChatModel}), and the chat reads it back when
   * the catalog returns to it.
   */
  tabId?: string;
}

export const PendingChatModelContext = createContext<PendingChatModel | null>(null);

/** The unsent chat this composer belongs to, or `null` (Home, or a started chat). */
export function usePendingChatModel(): PendingChatModel | null {
  return useContext(PendingChatModelContext);
}

/**
 * T3-SH-2 — held picks, by chat tab.
 *
 * ⚠ **Renderer memory, keyed by the TAB, for the same reason composer drafts
 * are** (`utils/composerDrafts.ts`). A pick held in `BaseChat`'s own state died
 * whenever that `BaseChat` did: on a tab switch, and on every trip off `/pair`.
 * "Use other provider" is such a trip, so the catalog's model step had nowhere
 * to put a pick for the chat it was opened from and wrote the model every new
 * chat starts on instead (`BIOROUTER_PROVIDER`), the very write W2-PRV-6 took
 * away from the picker.
 *
 * Lifetime: an entry is dropped when its tab binds to the chat its first
 * message started (`BaseChat` clears it as soon as it has a session), or when
 * the pick is withdrawn. A tab closed while unsent leaves its entry behind; it
 * is one small object that is never read again, because a renderer never
 * reuses a tab id (`chatGroupsReducer`'s `seq`), and a reload clears the map.
 * Nothing here is persisted: which model a person was about to try is not
 * something to leave on disk.
 */
const heldByTab = new Map<string, Model>();
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function notify() {
  for (const listener of [...listeners]) listener();
}

/** Hold `model` for the unsent chat in `tabId`, or withdraw the pick with `null`. */
export function holdChatModel(tabId: string, model: Model | null): void {
  if (model) {
    if (heldByTab.get(tabId) === model) return;
    heldByTab.set(tabId, model);
  } else {
    if (!heldByTab.has(tabId)) return;
    heldByTab.delete(tabId);
  }
  notify();
}

/** The pick held for the unsent chat in `tabId`, or `null`. */
export function heldChatModel(tabId: string): Model | null {
  return heldByTab.get(tabId) ?? null;
}

/** Tests only: the map is module state and outlives a `cleanup()`. */
export function __resetHeldChatModelsForTests() {
  heldByTab.clear();
  listeners.clear();
}

/**
 * The held pick for the chat in `tabId`, and its setter.
 *
 * With no tab (a chat mounted outside the tabbed shell) the pick lives in this
 * component's own state, as it always did: there is no identity to hold it
 * under that would outlive the component.
 */
export function useHeldChatModel(
  tabId: string | undefined
): [Model | null, (model: Model | null) => void] {
  const [local, setLocal] = useState<Model | null>(null);
  const stored = useSyncExternalStore(subscribe, () => (tabId ? heldChatModel(tabId) : null));
  const set = useCallback(
    (model: Model | null) => {
      if (tabId) holdChatModel(tabId, model);
      else setLocal(model);
    },
    [tabId]
  );
  return [tabId ? stored : local, set];
}
