import { withSteerRecoveries, type SteerRecovery } from '../utils/steerRecovery';
/**
 * ProgressiveMessageList Component
 *
 * Renders a long chat in batches so opening it does not block the window, with
 * one quiet "Loading messages…" line while batches remain. ⌘F renders the rest
 * at once for find.
 *
 * WHICH END FIRST (vision V2-R7-2). A live chat reads bottom up and opens at its
 * newest message, so it draws its NEWEST batch first and adds older batches
 * above it while the bottom of the transcript stays where it is (`opensAt`
 * 'end'). It used to draw the OLDEST batch first and append below, so opening a
 * 52-message chat painted its first question, then its middle, then its end
 * (and an 82-figure chat sat away from its end for 1.6 s). A saved transcript
 * (SessionHistoryView) reads top down from its first message and keeps that
 * order ('start').
 */

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Message } from '../api';
import BioRouterMessage from './BioRouterMessage';
import UserMessage from './UserMessage';
import { SystemNotificationInline } from './context_management/SystemNotificationInline';
import { NotificationEvent } from '../types/message';
import LoadingBioRouter from './LoadingBioRouter';
import { ChatType } from '../types/chat';
import TurnActivityIndicator from './TurnActivityIndicator';
import { deriveTrailingActivity, type PendingSteer } from '../utils/trailingActivity';
import { ChatState } from '../types/chatState';
import type { ArtifactSource } from './artifacts/artifactTypes';
import { cn } from '../utils';
import {
  isToolResponseOnly,
  messageGaps,
  ROW_GAP_CLASS,
  rowsUnderAnswerActions,
  trailingGap,
} from './transcript/transcriptRhythm';
import { useLiveRows } from './transcript/useLiveRows';
import { transcriptCopy } from './transcript/copy';

/** A row's key: its id, or its place and time for a row the daemon has not named yet. */
const rowKey = (message: Message, index: number) => message.id ?? `msg-${index}-${message.created}`;

/**
 * The transcript's viewport: the scroll area's own, else the nearest scroll
 * container. ⚠ Radix's viewport is `overflow-y: hidden` in the first commit (its
 * scrollbar turns `scroll` on in an effect), and a hidden box still scrolls, so
 * `hidden` counts: skipping it found no scroller on the open that matters most.
 */
function scrollingAncestor(element: HTMLElement): HTMLElement | null {
  const viewport = element.closest<HTMLElement>('[data-radix-scroll-area-viewport]');
  if (viewport) return viewport;
  for (let node = element.parentElement; node; node = node.parentElement) {
    const { overflowY } = window.getComputedStyle(node);
    if (overflowY !== 'visible' && overflowY !== 'clip') return node;
  }
  return null;
}

/** From the content's end to the viewport's bottom edge: 0 is the end in view. */
const bottomGap = (scroller: HTMLElement) =>
  scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;

interface ProgressiveMessageListProps {
  messages: Message[];
  chat: Pick<ChatType, 'sessionId'>;
  toolCallNotifications?: Map<string, NotificationEvent[]>; // Make optional
  isUserMessage: (message: Message) => boolean;
  batchSize?: number;
  batchDelay?: number;
  showLoadingThreshold?: number; // Only show loading if more than X messages
  /**
   * Which end of a long chat is drawn first. 'end' (the default when live
   * `chatState` is supplied): the newest batch, opened at its end, with older
   * batches added above while the bottom stays put. 'start' (the default for a
   * read-only replay): the first batch, with newer batches added below.
   */
  opensAt?: 'start' | 'end';
  // Custom render function for messages
  renderMessage?: (message: Message, index: number) => React.ReactNode | null;
  isStreamingMessage?: boolean; // Whether messages are currently being streamed
  /** An edit's save; `false` refuses it before anything changed (a chat held for Crew, F1). */
  onMessageUpdate?: (
    messageId: string,
    newContent: string,
    editType?: 'diverge' | 'edit'
  ) => unknown;
  onRenderingComplete?: () => void; // Callback when all messages are rendered
  submitElicitationResponse?: (
    elicitationId: string,
    userData: Record<string, unknown>
  ) => Promise<void>;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
  /**
   * Hand a shell code block to this chat's in-app terminal, or `null` on a
   * surface with no terminal (a saved transcript). Required and nullable so
   * every surface has to answer — see BioRouterMessage's prop of the same name.
   */
  onRunInTerminal: ((command: string) => boolean) | null;
  /**
   * Live turn state, supplied only by an interactive chat. Read-only replays
   * (SessionHistoryView) omit these, which is what guarantees the trailing
   * activity indicator can never appear on a saved session.
   */
  chatState?: ChatState;
  turnStartedAt?: number;
  lastMessageAt?: number;
  /** BR-61: a soft interrupt awaiting the agent, shown as a trailing chip. */
  pendingSteer?: PendingSteer;
  steerRecoveries?: SteerRecovery[];
  /** D4: re-send the text of a steer the daemon stored as unanswered. */
  onSendAgain?: (text: string) => void;
  /**
   * Whether the reader can stop the running turn from this tab. Only a
   * delegated subagent's tab in a browser answers false: it has no composer
   * (SD-8), so the trailing indicator's nudge must not point at one.
   */
  canStopTurn?: boolean;
}

export default function ProgressiveMessageList({
  messages: storedMessages,
  chat,
  toolCallNotifications = new Map(),
  isUserMessage,
  batchSize = 20,
  batchDelay = 20,
  showLoadingThreshold = 50,
  opensAt,
  renderMessage, // Custom render function
  isStreamingMessage = false, // Whether messages are currently being streamed
  onMessageUpdate,
  onRenderingComplete,
  submitElicitationResponse,
  onOpenArtifact,
  onRunInTerminal,
  workingDir,
  chatState,
  turnStartedAt,
  lastMessageAt,
  pendingSteer,
  steerRecoveries,
  onSendAgain,
  canStopTurn = true,
}: ProgressiveMessageListProps) {
  const messages = useMemo(
    () => withSteerRecoveries(storedMessages, steerRecoveries),
    [storedMessages, steerRecoveries]
  );
  const recoveryIds = useMemo(
    () =>
      new Set(
        steerRecoveries
          ?.filter(({ message }) => !storedMessages.some((stored) => stored.id === message.id))
          .map(({ message }) => message.id)
      ),
    [storedMessages, steerRecoveries]
  );
  // A live chat reads bottom up; a saved transcript top down.
  const holdsEnd = (opensAt ?? (chatState !== undefined ? 'end' : 'start')) === 'end';

  // How many messages are not drawn yet: the OLDEST ones when the chat opens at
  // its end, the NEWEST ones when it opens at its start. A message appended
  // while batches remain is drawn at once either way, and a drawn row is never
  // taken back out (a row that left and came back would lose what the reader
  // had opened in it).
  const firstHidden = (count: number) =>
    count > showLoadingThreshold ? Math.max(0, count - batchSize) : 0;
  const [hiddenState, setHidden] = useState(() => firstHidden(messages.length));
  // Messages that arrive after the list mounted (BaseChat while the chat loads)
  // start their batches then, from the end the chat opens at, instead of all
  // being drawn in the commit they arrive in.
  const [wasEmpty, setWasEmpty] = useState(messages.length === 0);
  if (wasEmpty !== (messages.length === 0)) {
    setWasEmpty(messages.length === 0);
    if (messages.length > 0) setHidden(firstHidden(messages.length));
  }
  const hidden =
    messages.length <= showLoadingThreshold
      ? 0
      : Math.min(hiddenState, Math.max(0, messages.length - batchSize));
  const isLoading = hidden > 0;
  // The drawn range, in the chat's own indices.
  const shownFrom = holdsEnd ? hidden : 0;
  const shownTo = holdsEnd ? messages.length : messages.length - hidden;

  const timeoutRef = useRef<number | null>(null);
  // The 50ms "the DOM has settled" timer that fires `onRenderingComplete`.
  // A SECOND ref, not `timeoutRef`: that one holds the NEXT batch's timer, and
  // the two must stay separately cancellable. Uncleared, this timer outlived
  // the component: unmounting a long transcript inside its 50ms window
  // (switching tab, closing a pane, a test file ending) left it holding a
  // callback into a tree that no longer exists.
  const completionTimeoutRef = useRef<number | null>(null);
  // Held in a ref so the effects below can CALL the latest callback without
  // taking its identity as a dependency. BaseChat memoises
  // `handleRenderingComplete` on `messages.length`, so during a stream its
  // identity changes on every event; as a dependency it re-armed the batching,
  // which sets state, which re-rendered the parent — the nested-update chain
  // that tripped React's depth limit.
  const onRenderingCompleteRef = useRef(onRenderingComplete);
  onRenderingCompleteRef.current = onRenderingComplete;

  // HOLDING THE END (opensAt 'end'). The transcript's viewport, found from the
  // loading line the first time it is drawn; the distance from the content's
  // end to the viewport's bottom edge before the next batch lands; and the
  // first drawn index at the last commit.
  const loadingLineRef = useRef<HTMLDivElement | null>(null);
  const scrollerRef = useRef<HTMLElement | null>(null);
  const endGapRef = useRef<number | null>(null);
  const shownFromRef = useRef<number | null>(null);
  const rememberEnd = useCallback(() => {
    const scroller = scrollerRef.current;
    if (scroller) endGapRef.current = bottomGap(scroller);
  }, []);

  const hasInlineSystemNotification = (message: Message): boolean => {
    return message.content.some(
      (content) =>
        content.type === 'systemNotification' && content.notificationType === 'inlineMessage'
    );
  };

  // One batch per `batchDelay` while messages remain hidden. Keyed on `hidden`
  // only: a message streamed in during the load neither postpones nor repeats a
  // batch.
  useEffect(() => {
    if (!isLoading) return;
    const timer = window.setTimeout(() => {
      timeoutRef.current = null;
      rememberEnd();
      setHidden(Math.max(0, hidden - batchSize));
    }, batchDelay);
    timeoutRef.current = timer;
    return () => {
      window.clearTimeout(timer);
      if (timeoutRef.current === timer) timeoutRef.current = null;
    };
  }, [isLoading, hidden, batchSize, batchDelay, rememberEnd]);

  // "The transcript has settled": 50ms after the last batch lands, and after
  // every change in the number of messages once none is hidden. Re-armed, not
  // cancelled, by a change inside the window, so a stream settles once it
  // pauses. Cancelled by two things only: unmounting, and batches starting. A
  // chat whose messages arrive inside the 50ms of the empty first render would
  // otherwise be reported settled with a batch or two drawn and the rest still
  // to come (WS-SUMMARY-2-FYI); "settled" means the last batch landed.
  useEffect(() => {
    window.clearTimeout(completionTimeoutRef.current ?? undefined);
    completionTimeoutRef.current = null;
    if (isLoading) return;
    completionTimeoutRef.current = window.setTimeout(() => {
      completionTimeoutRef.current = null;
      onRenderingCompleteRef.current?.();
    }, 50);
  }, [isLoading, messages.length]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (timeoutRef.current) {
        window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
      if (completionTimeoutRef.current) {
        window.clearTimeout(completionTimeoutRef.current);
        completionTimeoutRef.current = null;
      }
    };
  }, []);

  // Before a batch is drawn above the end, where the end was: a reader who
  // scrolls while batches remain keeps the place they scrolled to.
  useEffect(() => {
    const scroller = scrollerRef.current;
    if (!holdsEnd || !isLoading || !scroller) return;
    scroller.addEventListener('scroll', rememberEnd, { passive: true });
    return () => scroller.removeEventListener('scroll', rememberEnd);
  }, [holdsEnd, isLoading, rememberEnd]);

  // In the commit, before anything is painted: a long chat opens at its end,
  // and each older batch drawn above it leaves the end where it was. The write
  // is absolute (the content height minus the remembered gap), so it lands in
  // the same place whether or not the browser's own scroll anchoring already
  // moved the viewport, and it does not depend on the scroll area's follow
  // state, which only reacts to its own children and to resizes.
  useLayoutEffect(() => {
    if (!holdsEnd) return;
    if (!scrollerRef.current && loadingLineRef.current) {
      scrollerRef.current = scrollingAncestor(loadingLineRef.current);
    }
    const scroller = scrollerRef.current;
    const previous = shownFromRef.current;
    shownFromRef.current = shownFrom;
    if (!scroller || previous === shownFrom) return;
    if (isLoading && (previous === null || shownFrom > previous)) {
      scroller.scrollTop = scroller.scrollHeight;
      endGapRef.current = bottomGap(scroller);
      return;
    }
    if (previous !== null && shownFrom < previous && endGapRef.current !== null) {
      const top = Math.max(0, scroller.scrollHeight - scroller.clientHeight - endGapRef.current);
      if (Math.abs(scroller.scrollTop - top) >= 1) scroller.scrollTop = top;
    }
  });

  // Force complete rendering when search is active
  useEffect(() => {
    // Only add listener if we're actually loading
    if (!isLoading) {
      return;
    }

    const handleKeyDown = (e: KeyboardEvent) => {
      const isMac = window.electron.platform === 'darwin';
      const isSearchShortcut = (isMac ? e.metaKey : e.ctrlKey) && e.key === 'f';

      if (isSearchShortcut) {
        // Immediately render all messages when search is triggered (the batch
        // timer is cleared by the effect above when `hidden` changes).
        rememberEnd();
        setHidden(0);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isLoading, rememberEnd]);

  // One gap per row (24px between turns, 8px between blocks, 4px between tool
  // rows), and the `.br-enter` entrance only for rows appended live: a history
  // load, a tab switch and a reload draw at rest (spec 3.6).
  const gaps = useMemo(() => messageGaps(messages, isUserMessage), [messages, isUserMessage]);
  // The rows an answer's floating actions rise into: their last tool row ends
  // the cluster's width short, so a hover covers no label (`message-meta.css`).
  const underActions = useMemo(
    () => rowsUnderAnswerActions(messages, isUserMessage, gaps),
    [messages, isUserMessage, gaps]
  );
  const rowKeys = useMemo(() => messages.map(rowKey), [messages]);
  const liveRows = useLiveRows(
    rowKeys,
    chatState !== undefined,
    chatState === ChatState.LoadingConversation
  );

  // Pure, memoised on the same identities the list already re-renders for, so
  // it adds no scan the list was not already paying for.
  const trailingActivity = useMemo(
    () =>
      deriveTrailingActivity({
        messages,
        isTurnActive: isStreamingMessage,
        chatState,
        turnStartedAt,
        lastMessageAt,
        pendingSteer,
      }),
    [messages, isStreamingMessage, chatState, turnStartedAt, lastMessageAt, pendingSteer]
  );

  // The drawn range. Every index below is the message's place in the whole
  // chat (keys, gaps, `messageIndex`), whichever end the batches started from.
  const renderMessages = useCallback(() => {
    return messages
      .slice(shownFrom, shownTo)
      .map((message, offset) => {
        const index = shownFrom + offset;
        if (!message.metadata.userVisible) {
          return null;
        }
        if (renderMessage) {
          return renderMessage(message, index);
        }

        // Default rendering logic (for BaseChat)
        if (!chat) {
          console.warn(
            'ProgressiveMessageList: chat prop is required when not using custom renderMessage'
          );
          return null;
        }

        const key = rowKey(message, index);
        const live = liveRows.has(key);
        const gap = gaps[index] ?? 'block';
        const rowClassName = cn('relative', ROW_GAP_CLASS[gap], live && 'br-enter');
        // A row that is not live also holds its tool rows still: a tool row
        // rises 4px when it is appended to a live answer, and mounts at rest
        // from history, a tab switch or a reload (principle 9; WS-TOOLS reads
        // `[data-motion-still] .biorouter-message-tool`).
        const motionStill = live ? undefined : '';

        // System notifications are never user messages, handle them first
        if (hasInlineSystemNotification(message)) {
          return (
            <div
              key={key}
              className={rowClassName}
              data-role="assistant"
              data-gap={gap}
              data-motion-still={motionStill}
              data-testid="message-container"
            >
              <SystemNotificationInline message={message} />
            </div>
          );
        }

        const isUser = isUserMessage(message);

        return (
          <div
            key={key}
            className={rowClassName}
            data-role={isUser ? 'user' : 'assistant'}
            // The gap above the row, for the floating actions' rise
            // (`message-meta.css`): a bubble 8px under another rises less.
            data-gap={gap}
            // The answer under this row floats its actions into it.
            data-actions-below={underActions[index] ? '' : undefined}
            data-motion-still={motionStill}
            data-testid="message-container"
          >
            {isUser ? (
              !isToolResponseOnly(message) && (
                <UserMessage
                  message={message}
                  onMessageUpdate={recoveryIds.has(message.id) ? undefined : onMessageUpdate}
                  deliveryUnconfirmed={recoveryIds.has(message.id)}
                  onSendAgain={isStreamingMessage ? undefined : onSendAgain}
                />
              )
            ) : (
              <BioRouterMessage
                sessionId={chat.sessionId}
                message={message}
                messages={messages}
                messageIndex={index}
                toolCallNotifications={toolCallNotifications}
                turnActive={isStreamingMessage}
                isStreaming={
                  isStreamingMessage &&
                  !isUser &&
                  index === messages.length - 1 &&
                  message.role === 'assistant'
                }
                submitElicitationResponse={submitElicitationResponse}
                onOpenArtifact={onOpenArtifact}
                onRunInTerminal={onRunInTerminal}
                workingDir={workingDir}
                live={live}
              />
            )}
          </div>
        );
      })
      .filter(Boolean);
  }, [
    messages,
    shownFrom,
    shownTo,
    renderMessage,
    isUserMessage,
    chat,
    toolCallNotifications,
    isStreamingMessage,
    onMessageUpdate,
    submitElicitationResponse,
    onOpenArtifact,
    onRunInTerminal,
    workingDir,
    onSendAgain,
    recoveryIds,
    gaps,
    underActions,
    liveRows,
  ]);

  // While a long chat renders in batches: one quiet line, where the batches
  // are being added (above the rows when the chat opens at its end). ⌘F still
  // renders everything at once (the handler above); the tip that said so
  // belongs to the find bar's own help.
  const loadingLine = isLoading && (
    <div
      ref={loadingLineRef}
      className={holdsEnd ? 'flex justify-center pt-7 mb-6' : 'mt-6 flex justify-center'}
      data-testid="transcript-loading-line"
    >
      <LoadingBioRouter message={transcriptCopy.activity.loadingMessages} />
    </div>
  );

  return (
    <>
      {holdsEnd && loadingLine}
      {renderMessages()}

      {/* Trails the last tool card, INSIDE the scroll area, so the transcript
          never goes blank during the model round-trip after a tool returns.
          The wrapper classes mirror the message wrapper above so the vertical
          rhythm matches exactly. */}
      {trailingActivity && (
        <div
          className={cn('relative', ROW_GAP_CLASS[trailingGap(messages, isUserMessage)])}
          data-role="assistant"
          data-testid="trailing-activity-container"
        >
          <TurnActivityIndicator activity={trailingActivity} canStopHere={canStopTurn} />
        </div>
      )}

      {!holdsEnd && loadingLine}
    </>
  );
}
