import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react';
import type { Channel, Snapshot } from '../crewApi';
import { clearPublishedTransfers } from '../crewTransfers';
import { refreshCrewTransfers } from '../files/useCrewTransfers';
import { crewActionCopy } from './copy';
import { forgetStashedDraft } from './draftStash';
import type {
  ActionKey,
  ActOptions,
  DraftFile,
  DraftReference,
  ErrorSource,
  ObservedPrivacy,
} from './types';

/** The composer's unsent content, its idempotency attempt and the single-flight flag. */
export interface CrewDraftState {
  body: string;
  setBody: Dispatch<SetStateAction<string>>;
  attachments: DraftFile[];
  setAttachments: Dispatch<SetStateAction<DraftFile[]>>;
  references: DraftReference[];
  setReferences: Dispatch<SetStateAction<DraftReference[]>>;
  contextChannels: string[];
  setContextChannels: Dispatch<SetStateAction<string[]>>;
  /** The message attempt whose idempotency key a retry of the same payload reuses. */
  pendingMessage: MutableRefObject<{ fingerprint: string; key: string } | null>;
  /** True while a `message.post` is in flight; Enter and Send share it. */
  sendingMessage: MutableRefObject<boolean>;
  /** The context channels the observer checks against each verified snapshot. */
  selectedSources: MutableRefObject<string[]>;
  /** Clear the body, attachments, references, context channels and the pending attempt. */
  clearDraft(): void;
  addAttachment(file: DraftFile): void;
  removeAttachment(id: string): void;
  addReference(reference: DraftReference): void;
  removeReference(id: string): void;
  clearBodyIfEquals(seed: string): void;
}

export function useCrewDraft(): CrewDraftState {
  const [body, setBody] = useState('');
  const [attachments, setAttachments] = useState<DraftFile[]>([]);
  const [references, setReferences] = useState<DraftReference[]>([]);
  const [contextChannels, setContextChannels] = useState<string[]>([]);
  const pendingMessage = useRef<{ fingerprint: string; key: string } | null>(null);
  const sendingMessage = useRef(false);
  const selectedSources = useRef(contextChannels);
  useEffect(() => {
    selectedSources.current = contextChannels;
  }, [contextChannels]);
  const clearDraft = useCallback(() => {
    setBody('');
    setAttachments([]);
    setReferences([]);
    setContextChannels([]);
    pendingMessage.current = null;
  }, []);
  const addAttachment = useCallback(
    (file: DraftFile) =>
      setAttachments((items) =>
        items.some((item) => item.id === file.id) ? items : [...items, file]
      ),
    []
  );
  const removeAttachment = useCallback(
    (id: string) => setAttachments((items) => items.filter((item) => item.id !== id)),
    []
  );
  const addReference = useCallback(
    (reference: DraftReference) => setReferences((items) => [...items, reference]),
    []
  );
  const removeReference = useCallback(
    (id: string) => setReferences((items) => items.filter((item) => item.id !== id)),
    []
  );
  const clearBodyIfEquals = useCallback(
    (seed: string) => setBody((current) => (current === seed ? '' : current)),
    []
  );
  return {
    body,
    setBody,
    attachments,
    setAttachments,
    references,
    setReferences,
    contextChannels,
    setContextChannels,
    pendingMessage,
    sendingMessage,
    selectedSources,
    clearDraft,
    addAttachment,
    removeAttachment,
    addReference,
    removeReference,
    clearBodyIfEquals,
  };
}

export interface CrewSendContext {
  draft: CrewDraftState;
  busy: boolean;
  connectionId: string;
  channelId: string;
  channel: Channel | null;
  snapshot: Snapshot | null;
  observedPrivacy: ObservedPrivacy | null;
  generation: MutableRefObject<number>;
  historyPage: MutableRefObject<string | null>;
  setHistoryBefore: Dispatch<SetStateAction<string | null>>;
  restartObservation(): void;
  request<T>(
    method: string,
    params?: Record<string, unknown>,
    opts?: { mutation?: boolean }
  ): Promise<T>;
  /** `channel.read` up to `sequence`, never a refresh. */
  markRead(channelId: string, sequence: string): Promise<void>;
  act<T>(
    source: ErrorSource,
    key: ActionKey,
    fn: () => Promise<T>,
    options?: ActOptions
  ): Promise<T | undefined>;
  reportError(message: string, source?: ErrorSource, code?: string): void;
}

/**
 * Post the composer's draft to the selected channel.
 *
 * Single flight: a second Enter or Send while a post is in flight does nothing. The send is not
 * optimistic: the draft stays until the broker answers, and a retry of an unchanged payload reuses
 * the same idempotency key, while any change rotates it. Success clears only what was sent and
 * never refreshes the verified workspace. Posting while a history page is shown returns to the
 * live tail.
 *
 * A post also reads the channel up to the posted message (Q3-10), silently, so the person's own
 * message never sits under the "New" rule. And the transfer records it forgot are re-listed at
 * once (Q3-03), so the Files tab stops calling a sent file "not sent" without waiting for a remount.
 */
export function createSend(context: CrewSendContext): () => Promise<void> {
  const {
    draft,
    busy,
    connectionId,
    channelId,
    channel,
    snapshot,
    observedPrivacy,
    generation,
    historyPage,
    setHistoryBefore,
    restartObservation,
    request,
    markRead,
    act,
    reportError,
  } = context;
  const { body, attachments, references, pendingMessage, sendingMessage } = draft;
  return async () => {
    if (
      busy ||
      sendingMessage.current ||
      channel?.archived ||
      (!body.trim() && attachments.length === 0 && references.length === 0)
    )
      return;
    sendingMessage.current = true;
    try {
      await act('composer', 'send', async () => {
        if (!snapshot || observedPrivacy?.connectionId !== connectionId)
          throw new Error(crewActionCopy.sendPrivacyUnverified);
        const current = generation.current;
        const payload = {
          personal_mode: observedPrivacy.mode,
          channel_id: channelId,
          body,
          attachments: attachments.map((item) => item.id),
          references: references.map((item) => item.id),
        };
        const fingerprint = JSON.stringify({ connectionId, ...payload });
        if (pendingMessage.current?.fingerprint !== fingerprint)
          pendingMessage.current = { fingerprint, key: crypto.randomUUID() };
        const attempt = pendingMessage.current;
        const posted = await request<unknown>(
          'message.post',
          { ...payload, idempotency_key: attempt.key },
          { mutation: true }
        );
        // Your own post is read: move the read position to it. A failure changes nothing on show.
        const sequence =
          posted !== null && typeof posted === 'object'
            ? (posted as { sequence?: unknown }).sequence
            : undefined;
        if (typeof sequence === 'string' && sequence)
          void markRead(channelId, sequence).catch(() => undefined);
        try {
          await clearPublishedTransfers(connectionId, payload.attachments);
          // The shared transfer list re-lists only while a transfer moves: ask for it now.
          if (payload.attachments.length > 0) void refreshCrewTransfers(connectionId);
        } catch {
          if (current === generation.current)
            reportError(crewActionCopy.sendTransferRecordKept, 'composer');
        }
        // Sent: no earlier draft kept for this channel may come back over the conversation.
        forgetStashedDraft(connectionId, channelId);
        if (current !== generation.current) return;
        if (pendingMessage.current === attempt) pendingMessage.current = null;
        if (historyPage.current !== null) {
          historyPage.current = null;
          setHistoryBefore(null);
          restartObservation();
        }
        draft.setReferences((items) =>
          items.filter((item) => !payload.references.includes(item.id))
        );
        draft.setBody('');
        draft.setAttachments((items) =>
          items.filter((item) => !payload.attachments.includes(item.id))
        );
      });
    } finally {
      sendingMessage.current = false;
    }
  };
}
