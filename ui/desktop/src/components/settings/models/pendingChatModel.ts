import { createContext, useContext } from 'react';
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
}

export const PendingChatModelContext = createContext<PendingChatModel | null>(null);

/** The unsent chat this composer belongs to, or `null` (Home, or a started chat). */
export function usePendingChatModel(): PendingChatModel | null {
  return useContext(PendingChatModelContext);
}
