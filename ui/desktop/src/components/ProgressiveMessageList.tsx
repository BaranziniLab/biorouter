/**
 * ProgressiveMessageList Component
 *
 * A performance-optimized message list that renders messages progressively
 * to prevent UI blocking when loading long chat sessions. This component
 * renders messages in batches with a loading indicator, maintaining full
 * compatibility with the search functionality.
 *
 * Key Features:
 * - Progressive rendering in configurable batches
 * - Loading indicator during batch processing
 * - Maintains search functionality compatibility
 * - Smooth user experience with responsive UI
 * - Configurable batch size and delay
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Message } from '../api';
import BioRouterMessage from './BioRouterMessage';
import UserMessage from './UserMessage';
import { SystemNotificationInline } from './context_management/SystemNotificationInline';
import { NotificationEvent } from '../types/message';
import LoadingBioRouter from './LoadingBioRouter';
import { ChatType } from '../types/chat';
import { identifyConsecutiveToolCalls, isInChain } from '../utils/toolCallChaining';
import TurnActivityIndicator from './TurnActivityIndicator';
import { deriveTrailingActivity, type PendingSteer } from '../utils/trailingActivity';
import { ChatState } from '../types/chatState';
import type { ArtifactSource } from './artifacts/artifactTypes';

interface ProgressiveMessageListProps {
  messages: Message[];
  chat: Pick<ChatType, 'sessionId'>;
  toolCallNotifications?: Map<string, NotificationEvent[]>; // Make optional
  isUserMessage: (message: Message) => boolean;
  batchSize?: number;
  batchDelay?: number;
  showLoadingThreshold?: number; // Only show loading if more than X messages
  // Custom render function for messages
  renderMessage?: (message: Message, index: number) => React.ReactNode | null;
  isStreamingMessage?: boolean; // Whether messages are currently being streamed
  onMessageUpdate?: (messageId: string, newContent: string) => void;
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
  /**
   * Whether the reader can stop the running turn from this tab. Only a
   * delegated subagent's tab in a browser answers false: it has no composer
   * (SD-8), so the trailing indicator's nudge must not point at one.
   */
  canStopTurn?: boolean;
}

export default function ProgressiveMessageList({
  messages,
  chat,
  toolCallNotifications = new Map(),
  isUserMessage,
  batchSize = 20,
  batchDelay = 20,
  showLoadingThreshold = 50,
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
  canStopTurn = true,
}: ProgressiveMessageListProps) {
  const [renderedCount, setRenderedCount] = useState(() => {
    // Initialize with either all messages (if small) or first batch (if large)
    return messages.length <= showLoadingThreshold
      ? messages.length
      : Math.min(batchSize, messages.length);
  });
  const [isLoading, setIsLoading] = useState(() => messages.length > showLoadingThreshold);
  const timeoutRef = useRef<number | null>(null);
  // The 50ms "the DOM has settled" timer that fires `onRenderingComplete`.
  // A SECOND ref, not `timeoutRef`: that one holds the NEXT batch's timer, and
  // the completion timer is armed from inside the batch loop at the moment the
  // last batch lands — so a single ref would have each one clobbering the
  // other's handle and neither would be cancellable. Uncleared, this timer
  // outlived the component: unmounting a long transcript inside its 50ms window
  // (switching tab, closing a pane, a test file ending) left it holding a
  // callback into a tree that no longer exists.
  const completionTimeoutRef = useRef<number | null>(null);
  const mountedRef = useRef(true);
  // Held in a ref so the batching effect below can CALL the latest callback
  // without taking its identity as a dependency. BaseChat memoises
  // `handleRenderingComplete` on `messages.length`, so during a stream its
  // identity changes on every event; as a dependency it re-armed the effect,
  // which sets state, which re-rendered the parent — the nested-update chain
  // that tripped React's depth limit.
  const onRenderingCompleteRef = useRef(onRenderingComplete);
  onRenderingCompleteRef.current = onRenderingComplete;
  const hasOnlyToolResponses = (message: Message) =>
    message.content.every((c) => c.type === 'toolResponse');

  const hasInlineSystemNotification = (message: Message): boolean => {
    return message.content.some(
      (content) =>
        content.type === 'systemNotification' && content.notificationType === 'inlineMessage'
    );
  };

  // Simple progressive loading - start immediately when component mounts if needed
  useEffect(() => {
    if (messages.length <= showLoadingThreshold) {
      setRenderedCount(messages.length);
      setIsLoading(false);
      // For small lists, call completion callback immediately
      window.clearTimeout(completionTimeoutRef.current ?? undefined);
      completionTimeoutRef.current = window.setTimeout(() => {
        completionTimeoutRef.current = null;
        onRenderingCompleteRef.current?.();
      }, 50);
      return () => {
        window.clearTimeout(completionTimeoutRef.current ?? undefined);
        completionTimeoutRef.current = null;
      };
    }

    // Large list - start progressive loading
    const loadNextBatch = () => {
      setRenderedCount((current) => {
        const nextCount = Math.min(current + batchSize, messages.length);

        if (nextCount >= messages.length) {
          setIsLoading(false);
          // Call the completion callback after a brief delay to ensure DOM is
          // updated. Recorded so the cleanup below can cancel it — see
          // `completionTimeoutRef`. Cleared first because React may invoke this
          // updater more than once for a single dispatch (StrictMode does), and
          // an unrecorded handle is an uncancellable timer.
          window.clearTimeout(completionTimeoutRef.current ?? undefined);
          completionTimeoutRef.current = window.setTimeout(() => {
            completionTimeoutRef.current = null;
            onRenderingCompleteRef.current?.();
          }, 50);
        } else {
          // Schedule next batch
          timeoutRef.current = window.setTimeout(loadNextBatch, batchDelay);
        }

        return nextCount;
      });
    };

    // Start loading after a short delay
    timeoutRef.current = window.setTimeout(loadNextBatch, batchDelay);

    // ⚠ The COMPLETION timer is deliberately not cancelled here, only on
    // unmount. This effect re-arms on every `messages.length` change, so during
    // a stream it re-runs constantly; cancelling the completion callback on
    // each re-run would swallow the "the transcript has settled" signal for the
    // batch that had just finished. The batch timer below is a different
    // object — it schedules FUTURE work this effect is about to redo, so
    // cancelling it on a re-run is exactly right.
    return () => {
      if (timeoutRef.current) {
        window.clearTimeout(timeoutRef.current);
        timeoutRef.current = null;
      }
    };
    // `renderedCount` is deliberately NOT a dependency: this effect only ever
    // WRITES it, via `setRenderedCount(current => …)`. Listing it made the
    // effect re-arm itself on its own write, and `onRenderingComplete` (see the
    // ref above) re-armed it again on every stream event. Together they formed
    // the nested-update chain behind "Maximum update depth exceeded".
  }, [messages.length, batchSize, batchDelay, showLoadingThreshold]);

  // Cleanup on unmount
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (timeoutRef.current) {
        window.clearTimeout(timeoutRef.current);
      }
      if (completionTimeoutRef.current) {
        window.clearTimeout(completionTimeoutRef.current);
        completionTimeoutRef.current = null;
      }
    };
  }, []);

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
        // Immediately render all messages when search is triggered
        setRenderedCount(messages.length);
        setIsLoading(false);
        if (timeoutRef.current) {
          window.clearTimeout(timeoutRef.current);
          timeoutRef.current = null;
        }
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isLoading, messages.length]);

  // Detect tool call chains
  const toolCallChains = useMemo(() => identifyConsecutiveToolCalls(messages), [messages]);

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

  // Render messages up to the current rendered count
  const renderMessages = useCallback(() => {
    const messagesToRender = messages.slice(0, renderedCount);
    return messagesToRender
      .map((message, index) => {
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

        // System notifications are never user messages, handle them first
        if (hasInlineSystemNotification(message)) {
          return (
            <div
              key={message.id ?? `msg-${index}-${message.created}`}
              className={`relative ${index === 0 ? 'mt-0' : 'mt-4'} assistant`}
              data-testid="message-container"
            >
              <SystemNotificationInline message={message} />
            </div>
          );
        }

        const isUser = isUserMessage(message);
        const messageIsInChain = isInChain(index, toolCallChains);

        return (
          <div
            key={message.id ?? `msg-${index}-${message.created}`}
            className={`relative ${index === 0 ? 'mt-0' : 'mt-4'} ${isUser ? 'user' : 'assistant'} ${messageIsInChain ? 'in-chain' : ''}`}
            data-testid="message-container"
          >
            {isUser ? (
              !hasOnlyToolResponses(message) && (
                <UserMessage message={message} onMessageUpdate={onMessageUpdate} />
              )
            ) : (
              <BioRouterMessage
                sessionId={chat.sessionId}
                message={message}
                messages={messages}
                messageIndex={index}
                toolCallChains={toolCallChains}
                toolCallNotifications={toolCallNotifications}
                turnActive={isStreamingMessage}
                isStreaming={
                  isStreamingMessage &&
                  !isUser &&
                  index === messagesToRender.length - 1 &&
                  message.role === 'assistant'
                }
                submitElicitationResponse={submitElicitationResponse}
                onOpenArtifact={onOpenArtifact}
                onRunInTerminal={onRunInTerminal}
                workingDir={workingDir}
              />
            )}
          </div>
        );
      })
      .filter(Boolean);
  }, [
    messages,
    renderedCount,
    renderMessage,
    isUserMessage,
    chat,
    toolCallNotifications,
    isStreamingMessage,
    onMessageUpdate,
    toolCallChains,
    submitElicitationResponse,
    onOpenArtifact,
    onRunInTerminal,
    workingDir,
  ]);

  return (
    <>
      {renderMessages()}

      {/* Trails the last tool card, INSIDE the scroll area, so the transcript
          never goes blank during the model round-trip after a tool returns.
          The wrapper classes mirror the message wrapper above so the vertical
          rhythm matches exactly. */}
      {trailingActivity && (
        <div className="relative mt-4 assistant" data-testid="trailing-activity-container">
          <TurnActivityIndicator activity={trailingActivity} canStopHere={canStopTurn} />
        </div>
      )}

      {/* Loading indicator when progressively rendering */}
      {isLoading && (
        <div className="flex flex-col items-center justify-center py-8">
          <LoadingBioRouter message={`Loading messages... (${renderedCount}/${messages.length})`} />
          <div className="text-xs text-text-muted mt-2">
            Press Cmd/Ctrl+F to load all messages immediately for search
          </div>
        </div>
      )}
    </>
  );
}
