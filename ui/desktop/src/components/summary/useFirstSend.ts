import { useCallback, useMemo, useState } from 'react';
import type { Message } from '../../api';

/**
 * A new chat's first send, from the commit that starts creating the chat here
 * to the commit in which the chat's store holds the message (vision V2-R8-2).
 *
 * A message typed into a chat that exists is in the transcript at once: the
 * store appends it before the turn starts. A chat's FIRST message cannot be:
 * the chat is created (`/agent/start`), bound to its model, navigated to and
 * loaded before its store will take a message. Measured at 1440 on the review
 * machine, the text left the composer at 81 ms and reached the transcript at
 * 753 ms, so for 0.67 s it was in neither place. Over the same span the summary
 * rail read "Tokens 0" (the counters of a chat with no session), then nothing
 * (the load), then "N/A" (the chat's own ledger).
 *
 * This holds what BaseChat knows from the send on:
 * - `creating`: the chat was created here, for this message, and its store does
 *   not hold the message yet. It is new and empty, so its counts are zero and
 *   are not waited for; its tokens are not known until its own ledger answers.
 * - `message`: the row the transcript shows meanwhile, the text as sent. It is
 *   drawn by the transcript's own list (`FirstSendTranscript`), so the stored
 *   row that replaces it is the same row in the same place.
 *
 * It ends when the store holds the chat's messages, when the chat could not be
 * created or bound (the composer has the text back), when the first submit is
 * refused (the text goes back to the composer), and the row is withheld while
 * the chat shows a turn error or a failed load: it would claim a send that did
 * not happen.
 */
interface FirstSendState {
  text: string;
  /** The created chat, once `/agent/start` has answered. */
  chatId: string | null;
}

export interface FirstSendInputs {
  /** The route's chat: empty while the chat is being created. */
  sessionId: string;
  /** How many messages the chat's store holds. */
  messageCount: number;
  /** A turn error or a failed load is on screen. */
  failed: boolean;
}

export interface FirstSend {
  creating: boolean;
  message: Message | null;
  /** The chat is about to be created to send `text`. */
  begin: (text: string) => void;
  /** `/agent/start` answered with the chat's id. */
  created: (chatId: string) => void;
  /** The send is over without the store taking it. */
  end: () => void;
}

/** The provisional row's id. One first send at a time, so one id. */
export const FIRST_SEND_ROW_ID = 'first-send';

/**
 * The row for a message on its way into a chat that has none yet: the text as
 * sent, nothing else. Images are left out: their bytes are read (and their
 * temporary files deleted) only by the store's own `createUserMessage`.
 */
export function firstSendMessage(text: string, created = Math.floor(Date.now() / 1000)): Message {
  return {
    id: FIRST_SEND_ROW_ID,
    role: 'user',
    created,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

export function useFirstSend({ sessionId, messageCount, failed }: FirstSendInputs): FirstSend {
  const [send, setSend] = useState<FirstSendState | null>(null);
  // The store holds the chat's messages: the send has landed. Adjusted during
  // render (React's "state from props"), so no commit draws both rows, and a
  // chat whose messages are later taken away never shows this one again.
  if (send && messageCount > 0) setSend(null);

  const begin = useCallback((text: string) => setSend({ text, chatId: null }), []);
  const created = useCallback(
    (chatId: string) => setSend((current) => (current ? { ...current, chatId } : current)),
    []
  );
  const end = useCallback(() => setSend(null), []);

  // This chat, or no chat yet: a tab that moves to another chat is not sending this.
  const creating = !!send && messageCount === 0 && (!sessionId || sessionId === send.chatId);
  const text = creating && !failed ? send.text : '';
  const message = useMemo(() => (text.trim() ? firstSendMessage(text) : null), [text]);
  return { creating, message, begin, created, end };
}
