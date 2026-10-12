import type { Session } from '../../api';
import { ChatState } from '../../types/chatState';
import { useLoadingChatRow } from './useLoadingChatRow';

/**
 * What a chat whose transcript has not landed yet should look like.
 *
 * - `chat`: its row says it has messages. Lay it out as the chat it is: the
 *   summary rail at rest, "Ask a follow-up", and the row's own model on the chip.
 * - `new`: it has no messages, or it was created to send the message that is on
 *   its way (Home's composer, a launcher message, a recovery hint). The new-chat
 *   composer is what it is about to show.
 * - `pending`: nothing has said which yet. The column paints nothing: no
 *   composer, so no placeholder and no model, and no rail. The chat then appears
 *   once, in its place, when its row answers or its transcript lands.
 *
 * Why `pending` rather than a guess (vision V2-R5-6, round 7). After a reload the
 * listing of every chat is still on its way (11,785 rows on the review machine)
 * and the row read waits behind the app's other requests, so a chat opened from
 * the sidebar had no row for 140 to 430 ms. Without one it was drawn as a new
 * chat: "Ask Biorouter anything…", the new-chat model and padlock on a chat bound
 * to another model, and no rail, so the column sat 140px right of where it
 * landed. Guessing the other way is wrong too: a chat created from Workflows has
 * no messages and no message on its way, and would draw the rail and drop it.
 * Neither listing ever holds an empty chat, so a missing row proves nothing.
 *
 * The composer is held at opacity 0, not `visibility: hidden` and not `inert`:
 * it keeps the focus it takes on mount, so what the person types while the row
 * is read is there when it appears, and it stays reachable by keyboard and
 * screen reader (spec principle 12).
 */
export type LoadingChatLook = 'chat' | 'new' | 'pending';

export interface LoadingChatLookInputs {
  /** The chat's row, from the listing or a read of the row alone. */
  row: Pick<Session, 'message_count'> | undefined;
  /** A message or attachments are on their way into this chat. */
  startingWithCargo: boolean;
}

export function loadingChatLook({
  row,
  startingWithCargo,
}: LoadingChatLookInputs): LoadingChatLook {
  if (row) return row.message_count > 0 ? 'chat' : 'new';
  return startingWithCargo ? 'new' : 'pending';
}

export interface AwaitingFirstReadInputs {
  sessionId: string;
  /** The chat stream's session, once its transcript (or the cache's) is in. */
  hasSession: boolean;
  hasLoadError: boolean;
  chatState: ChatState;
  messageCount: number;
  /** BaseChat's `isCreatingSession`: the chat is being created here. */
  creatingChat: boolean;
}

/**
 * The commit before the load starts: the route names a chat, and its store has
 * nothing yet. A store made for a chat this window has not opened starts at
 * `Idle` with no session, and only `loadSession`, an effect, moves it to
 * `LoadingConversation`, which reaches React on the next frame. Read as `Idle`,
 * that commit was the empty state, greeting and all.
 */
export function awaitingFirstRead({
  sessionId,
  hasSession,
  hasLoadError,
  chatState,
  messageCount,
  creatingChat,
}: AwaitingFirstReadInputs): boolean {
  return (
    !!sessionId &&
    !hasSession &&
    !hasLoadError &&
    chatState === ChatState.Idle &&
    messageCount === 0 &&
    !creatingChat
  );
}

export interface LoadingChat {
  /** `LoadingConversation`, or the commit before it ({@link awaitingFirstRead}). */
  loading: boolean;
  /** The chat's row while it loads; `undefined` once it has its session. */
  row: Session | undefined;
  /**
   * How it looks while it loads; `null` once it has its session, and for a chat
   * the route does not name (a new chat has nothing to look up).
   */
  look: LoadingChatLook | null;
  /** The row's message count while it loads, else 0. */
  knownMessages: number;
}

export interface UseLoadingChatInputs extends Omit<AwaitingFirstReadInputs, 'hasSession'> {
  session: Session | undefined;
  startingWithCargo: boolean;
}

/** BaseChat's one answer to "is this chat loading, and what does it look like meanwhile". */
export function useLoadingChat({
  session,
  startingWithCargo,
  ...inputs
}: UseLoadingChatInputs): LoadingChat {
  const loading =
    inputs.chatState === ChatState.LoadingConversation ||
    awaitingFirstRead({ ...inputs, hasSession: !!session });
  const row = useLoadingChatRow(inputs.sessionId, loading);
  return {
    loading,
    row,
    look:
      loading && inputs.sessionId
        ? // A chat being created here is a new chat whose first message is on its way.
          loadingChatLook({ row, startingWithCargo: startingWithCargo || inputs.creatingChat })
        : null,
    knownMessages: row?.message_count ?? 0,
  };
}
