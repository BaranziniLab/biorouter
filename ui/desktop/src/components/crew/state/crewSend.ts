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
import { channelName } from '../identity/objectNames';
import { crewActionCopy } from './copy';
import { forgetStashedDraft } from './draftStash';
import { failureCode, failureMessage } from './observationFailure';
import type {
  ActionKey,
  ActOptions,
  DraftFile,
  DraftReference,
  ErrorSource,
  ObservedPrivacy,
} from './types';

/** One `message.post` attempt: the payload it carried and the idempotency key it went with. */
export interface MessageAttempt {
  fingerprint: string;
  key: string;
}

/** The composer's unsent content, each channel's idempotency attempt and single flight. */
export interface CrewDraftState {
  body: string;
  setBody: Dispatch<SetStateAction<string>>;
  attachments: DraftFile[];
  setAttachments: Dispatch<SetStateAction<DraftFile[]>>;
  references: DraftReference[];
  setReferences: Dispatch<SetStateAction<DraftReference[]>>;
  contextChannels: string[];
  setContextChannels: Dispatch<SetStateAction<string[]>>;
  /**
   * Each destination's ({@link postDestination}) message attempt, whose idempotency key a retry of
   * the same payload to the same channel reuses. One per destination, like `sendingMessage`,
   * because posts to two channels can be on their way at once (RENDERER-4): a single slot let a
   * post in #analysis replace #methods' attempt, so a retry in #methods drew a new key and the
   * broker posted the same message twice. An entry lasts while the text it was made for can still
   * be sent again from its channel's composer, and no longer (see {@link createSend}).
   */
  pendingMessages: MutableRefObject<Map<string, MessageAttempt>>;
  /**
   * The destinations ({@link postDestination}) with a `message.post` in flight, read and written
   * synchronously: Enter and Send share it, so a channel posts one message at a time. A post in
   * one channel never holds another channel's composer (RENDERER-4).
   */
  sendingMessage: MutableRefObject<Set<string>>;
  /** The same destinations, as state: what a render asks to know whether its channel is posting. */
  postingTo: ReadonlySet<string>;
  setPosting(destination: string, posting: boolean): void;
  /** The context channels the observer checks against each verified snapshot. */
  selectedSources: MutableRefObject<string[]>;
  /**
   * Clear the body, attachments, references and context channels, and forget every unanswered
   * attempt on `connectionId`: that connection's drafts are being thrown away with this one.
   */
  clearDraft(connectionId: string): void;
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
  const pendingMessages = useRef(new Map<string, MessageAttempt>());
  const sendingMessage = useRef(new Set<string>());
  const [postingTo, setPostingTo] = useState<ReadonlySet<string>>(() => new Set());
  const setPosting = useCallback(
    (destination: string, posting: boolean) =>
      setPostingTo((current) => {
        if (current.has(destination) === posting) return current;
        const next = new Set(current);
        if (posting) next.add(destination);
        else next.delete(destination);
        return next;
      }),
    []
  );
  const selectedSources = useRef(contextChannels);
  useEffect(() => {
    selectedSources.current = contextChannels;
  }, [contextChannels]);
  const clearDraft = useCallback((connectionId: string) => {
    setBody('');
    setAttachments([]);
    setReferences([]);
    setContextChannels([]);
    for (const destination of [...pendingMessages.current.keys()])
      if (destinationConnection(destination) === connectionId)
        pendingMessages.current.delete(destination);
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
    pendingMessages,
    sendingMessage,
    postingTo,
    setPosting,
    selectedSources,
    clearDraft,
    addAttachment,
    removeAttachment,
    addReference,
    removeReference,
    clearBodyIfEquals,
  };
}

/** Where a post goes: its connection and channel, as one key. */
export function postDestination(connectionId: string, channelId: string): string {
  return `${connectionId}\n${channelId}`;
}

/** The connection a {@link postDestination} key names. */
function destinationConnection(destination: string): string {
  return destination.slice(0, destination.indexOf('\n'));
}

export interface CrewSendContext {
  draft: CrewDraftState;
  /** Another action is pending (a post in another channel does not count). */
  busy: boolean;
  connectionId: string;
  channelId: string;
  channel: Channel | null;
  snapshot: Snapshot | null;
  observedPrivacy: ObservedPrivacy | null;
  generation: MutableRefObject<number>;
  /** The connection and channel selected now, as of the latest render. */
  selection: MutableRefObject<{ connectionId: string; channelId: string }>;
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
 * optimistic: the draft stays until the broker answers, and a retry of an unchanged payload to the
 * same channel reuses the same idempotency key, while any change rotates it. The attempt belongs to
 * its channel, so a post elsewhere in between never takes the key away (RENDERER-4). Success
 * clears only what was sent and never refreshes the verified workspace. Posting while a history
 * page is shown returns to the live tail.
 *
 * The attempt lasts exactly as long as its text can be sent again from a composer. An answer to
 * the composer that sent it clears the text and the attempt together. An answer after the view
 * moved on (the person left the channel and came back, or its updates were observed again) changes
 * nothing in the composer, so the text the person was told is kept is still there, and so is its
 * key: sending it again is the same message to the broker, never a second one. An answer while the
 * person is in another channel forgets that channel's kept draft, so the text cannot come back,
 * and the attempt goes with it: kept, its key would stand in for the next message written there
 * with the same words, and the broker would take that new message for this one.
 *
 * A post also reads the channel up to the posted message (Q3-10), silently, so the person's own
 * message never sits under the "New" rule. And the transfer records it forgot are re-listed at
 * once (Q3-03), so the Files tab stops calling a sent file "not sent" without waiting for a remount.
 *
 * A post belongs to the channel it was sent in (RENDERER-4). Single flight is per channel, so a
 * person who sends in #methods and moves to #analysis can write and send there while #methods'
 * post is still on its way. If that post is then refused, the composer on screen is not the one
 * that sent it: the refusal goes to the connection bar, naming #methods, whose kept draft still
 * holds the text. While the person is still in the channel it went to, it shows in the composer
 * as before.
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
    selection,
    historyPage,
    setHistoryBefore,
    restartObservation,
    request,
    markRead,
    act,
    reportError,
  } = context;
  const { body, attachments, references, pendingMessages, sendingMessage } = draft;
  const destination = postDestination(connectionId, channelId);
  /** The person is still in the channel this post went to. */
  const stillHere = () =>
    selection.current.connectionId === connectionId && selection.current.channelId === channelId;
  return async () => {
    if (
      busy ||
      sendingMessage.current.has(destination) ||
      channel?.archived ||
      (!body.trim() && attachments.length === 0 && references.length === 0)
    )
      return;
    sendingMessage.current.add(destination);
    draft.setPosting(destination, true);
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
        const kept = pendingMessages.current.get(destination);
        const attempt: MessageAttempt =
          kept?.fingerprint === fingerprint ? kept : { fingerprint, key: crypto.randomUUID() };
        pendingMessages.current.set(destination, attempt);
        let posted: unknown;
        try {
          posted = await request<unknown>(
            'message.post',
            { ...payload, idempotency_key: attempt.key },
            { mutation: true }
          );
        } catch (failure) {
          if (stillHere()) throw failure;
          // The composer on screen belongs to another channel now: say which post failed, in the
          // connection bar, rather than above a draft that was never sent.
          reportError(
            crewActionCopy.sendFailedIn(
              channelName(channel),
              failureMessage(failure, crewActionCopy.actionFallback)
            ),
            'global',
            failureCode(failure)
          );
          return;
        }
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
        const retire = () => {
          if (pendingMessages.current.get(destination) === attempt)
            pendingMessages.current.delete(destination);
        };
        if (current !== generation.current) {
          // Elsewhere, the text is gone with the kept draft just forgotten: the attempt goes too.
          // Here again, the composer still holds it, and keeps the key for sending it again.
          if (!stillHere()) retire();
          return;
        }
        retire();
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
      sendingMessage.current.delete(destination);
      draft.setPosting(destination, false);
    }
  };
}
