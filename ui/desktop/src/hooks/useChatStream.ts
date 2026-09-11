import { useEffect, useMemo, useSyncExternalStore } from 'react';
import { ChatState } from '../types/chatState';
import { Message, Session, TokenState } from '../api';
import { NotificationEvent, UserAttachment } from '../types/message';
import {
  useChatStreamController,
  type PendingContinuationView,
  type PendingToolCallView,
  type PinnedModelView,
  type StopConfirmedView,
} from './chatStreamStore';
import type { ContinuationRecoveryAction } from '../utils/continuationLease';
import type { ChatTurnErrorData } from '../types/turnError';
import type { PendingSteer } from '../utils/trailingActivity';

interface UseChatStreamProps {
  sessionId: string;
  onStreamFinish: () => void;
  onSessionLoaded?: () => void;
}

interface UseChatStreamReturn {
  session?: Session;
  messages: Message[];
  chatState: ChatState;
  setChatState: (state: ChatState) => void;
  /**
   * Send a user message. Resolves FALSE when the submit was refused silently
   * and the caller still owns the text (see `ChatStreamController.handleSubmit`
   * for the full contract). A caller holding the user's words must put them
   * back rather than clear them.
   */
  handleSubmit: (userMessage: string, attachments?: UserAttachment[]) => Promise<boolean>;
  /**
   * Re-run the last turn after a retryable failure (backend blip, provider
   * error, dropped stream, transient cold-load failure). Never duplicates the
   * trailing user message. Wired to the inline turn-error Retry action.
   */
  retryTurn: () => Promise<void>;
  submitSystemMessage: (message: Message) => Promise<void>;
  submitElicitationResponse: (
    elicitationId: string,
    userData: Record<string, unknown>
  ) => Promise<void>;
  setWorkflowUserParams: (values: Record<string, string>) => Promise<void>;
  /** Resolves true once the server confirms the cancelled turn released its slot. */
  stopStreaming: (continuationPending?: boolean) => Promise<boolean>;
  /** Explicitly release a Stop-and-Send admission when its queued message is discarded. */
  abandonContinuation: () => Promise<void>;
  /** Resolve a recovered or foreign-owned Stop-and-Send gap explicitly. */
  recoverPendingContinuation: (action: ContinuationRecoveryAction) => Promise<void>;
  /** BR-61: inject a message into the running turn without cancelling it. */
  steer: (text: string) => Promise<boolean>;
  sessionLoadError?: string;
  turnError?: ChatTurnErrorData;
  tokenState: TokenState;
  /** Client clock (ms) when the current turn was submitted; undefined while idle. */
  turnStartedAt?: number;
  /** Client clock (ms) when the most recent live Message event was applied. */
  lastMessageAt?: number;
  /** BR-61: a soft interrupt issued but not yet echoed back by the agent. */
  pendingSteer?: PendingSteer;
  pendingContinuation?: PendingContinuationView;
  /** F5: the daemon confirmed that this chat's last Stop ended a running turn. Transient. */
  stopConfirmed?: StopConfirmedView;
  /**
   * Whether the session's model + extensions have finished loading. The
   * transcript is up well before this — anything reading AGENT state must gate
   * on it rather than on `session`.
   */
  agentReady: boolean;
  notifications: Map<string, NotificationEvent[]>;
  /**
   * §6.1b — tool calls the model has begun emitting whose arguments are still
   * streaming. Advisory skeleton state, never part of `messages`; each is
   * removed the instant its authoritative `ToolRequest` lands.
   */
  pendingToolCalls: PendingToolCallView[];
  /**
   * Issue #56 Gate B: the binding the privacy barrier pinned this chat to, or
   * `undefined` when no turn has been repaired. Not by itself a statement that
   * anything is wrong — see `privacy/pinnedModel.ts`.
   */
  pinnedModel?: PinnedModelView;
  onMessageUpdate: (
    messageId: string,
    newContent: string,
    editType?: 'diverge' | 'edit'
  ) => Promise<void>;
}

export function useChatStream({
  sessionId,
  onStreamFinish,
  onSessionLoaded,
}: UseChatStreamProps): UseChatStreamReturn {
  const controller = useChatStreamController(sessionId);
  const snapshot = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot
  );

  useEffect(() => {
    return controller.subscribeFinish(onStreamFinish);
  }, [controller, onStreamFinish]);

  useEffect(() => {
    void controller.loadSession(onSessionLoaded);
  }, [controller, onSessionLoaded]);

  const notificationsMap = useMemo(() => {
    return snapshot.notifications.reduce((map, notification) => {
      const key = notification.request_id;
      if (!map.has(key)) {
        map.set(key, []);
      }
      map.get(key)!.push(notification);
      return map;
    }, new Map<string, NotificationEvent[]>());
  }, [snapshot.notifications]);

  return {
    sessionLoadError: snapshot.sessionLoadError,
    turnError: snapshot.turnError,
    messages: snapshot.messages,
    session: snapshot.session,
    chatState: snapshot.chatState,
    setChatState: controller.setChatState,
    handleSubmit: controller.handleSubmit,
    retryTurn: controller.retryTurn,
    submitSystemMessage: controller.submitSystemMessage,
    submitElicitationResponse: controller.submitElicitationResponse,
    stopStreaming: controller.stopStreaming,
    abandonContinuation: controller.abandonContinuation,
    recoverPendingContinuation: controller.recoverPendingContinuation,
    steer: controller.steer,
    setWorkflowUserParams: controller.setWorkflowUserParams,
    tokenState: snapshot.tokenState,
    turnStartedAt: snapshot.turnStartedAt,
    lastMessageAt: snapshot.lastMessageAt,
    pendingSteer: snapshot.pendingSteer,
    pendingContinuation: snapshot.pendingContinuation,
    stopConfirmed: snapshot.stopConfirmed,
    agentReady: snapshot.agentReady,
    notifications: notificationsMap,
    pendingToolCalls: snapshot.pendingToolCalls,
    pinnedModel: snapshot.pinnedModel,
    onMessageUpdate: controller.onMessageUpdate,
  };
}
