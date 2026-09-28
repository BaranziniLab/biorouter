import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react';
import type { Channel, Snapshot } from '../crewApi';
import { clearPublishedTransfers } from '../crewTransfers';
import { refreshCrewTransfers } from '../files/useCrewTransfers';
import { channelName } from '../identity/objectNames';
import { crewActionCopy } from './copy';
import {
  forgetStashedDraft,
  resetBetweenTests,
  type DraftAttempt,
  type MessageAttempt,
} from './draftStash';
import { failureCode, failureMessage } from './observationFailure';
import type {
  ActionKey,
  ActOptions,
  DraftFile,
  DraftReference,
  ErrorSource,
  ObservedPrivacy,
} from './types';

/**
 * The composer's unsent content, and the idempotency attempt that goes with its text.
 *
 * The attempt belongs to the text, not to the component (RENDERER-4). It is one object
 * ({@link DraftAttempt}) that moves with the text: into the draft stash when the text is put aside
 * (a channel switch, a connection switch, leaving Crew) and back when the text comes back. So the
 * key lasts exactly as long as the text can be sent again, whichever screen holds it: a Crew opened
 * again sends the text it gets back under the key it was first sent with.
 */
export interface CrewDraftState {
  body: string;
  /**
   * Write the composer's text. Emptying it lets its attempt go (no text, nothing to send again),
   * and so does any write to a text whose post was already taken (`delivered`): the same words
   * written anew are a new message. Otherwise the attempt stays, and the next send decides by
   * comparing what it would post with what the attempt was made for.
   */
  setBody: Dispatch<SetStateAction<string>>;
  attachments: DraftFile[];
  setAttachments: Dispatch<SetStateAction<DraftFile[]>>;
  references: DraftReference[];
  setReferences: Dispatch<SetStateAction<DraftReference[]>>;
  contextChannels: string[];
  setContextChannels: Dispatch<SetStateAction<string[]>>;
  /**
   * The attempt that goes with the composer's text, when that text is `channelId`'s on
   * `connectionId`: what is kept beside the text when it is put aside.
   */
  attemptFor(connectionId: string, channelId: string): DraftAttempt | null;
  /**
   * The attempt a post of the composer's text to `channelId` goes with: the text's own, else a new,
   * empty one that becomes the text's. Synchronous, so the text carries it from the moment Send is
   * pressed, even if it is put aside before the post is on its way.
   */
  claimAttempt(connectionId: string, channelId: string): DraftAttempt;
  /** Put a kept draft back into the composer, with the attempt it was kept with. */
  restoreBody(connectionId: string, channelId: string, body: string, attempt?: DraftAttempt): void;
  /** The context channels the observer checks against each verified snapshot. */
  selectedSources: MutableRefObject<string[]>;
  /** Clear the body (and its attempt), attachments, references and context channels. */
  clearDraft(): void;
  addAttachment(file: DraftFile): void;
  removeAttachment(id: string): void;
  addReference(reference: DraftReference): void;
  removeReference(id: string): void;
  clearBodyIfEquals(seed: string): void;
}

/** The attempt that goes with the composer's text, and the channel that text belongs to. */
interface ComposerAttempt {
  destination: string;
  attempt: DraftAttempt;
}

export function useCrewDraft(): CrewDraftState {
  const [body, setBodyState] = useState('');
  const [attachments, setAttachments] = useState<DraftFile[]>([]);
  const [references, setReferences] = useState<DraftReference[]>([]);
  const [contextChannels, setContextChannels] = useState<string[]>([]);
  const composerAttempt = useRef<ComposerAttempt | null>(null);
  // The body as last rendered, for the one write that decides by it (`clearBodyIfEquals`).
  const bodyNow = useRef(body);
  bodyNow.current = body;
  const setBody = useCallback((value: SetStateAction<string>) => {
    const held = composerAttempt.current;
    if (held && (value === '' || held.attempt.current?.delivered)) composerAttempt.current = null;
    setBodyState(value);
  }, []);
  const attemptFor = useCallback((connectionId: string, channelId: string) => {
    const held = composerAttempt.current;
    return held?.destination === postDestination(connectionId, channelId) ? held.attempt : null;
  }, []);
  const claimAttempt = useCallback(
    (connectionId: string, channelId: string) => {
      const held = attemptFor(connectionId, channelId);
      if (held) return held;
      const attempt: DraftAttempt = { current: null };
      composerAttempt.current = { destination: postDestination(connectionId, channelId), attempt };
      return attempt;
    },
    [attemptFor]
  );
  const restoreBody = useCallback(
    (connectionId: string, channelId: string, text: string, attempt?: DraftAttempt) => {
      composerAttempt.current = attempt
        ? { destination: postDestination(connectionId, channelId), attempt }
        : null;
      setBodyState(text);
    },
    []
  );
  const selectedSources = useRef(contextChannels);
  useEffect(() => {
    selectedSources.current = contextChannels;
  }, [contextChannels]);
  const clearDraft = useCallback(() => {
    setBody('');
    setAttachments([]);
    setReferences([]);
    setContextChannels([]);
  }, [setBody]);
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
  const clearBodyIfEquals = useCallback((seed: string) => {
    // The text left for somewhere else (a task's prompt): its attempt does not go with it.
    if (bodyNow.current === seed) composerAttempt.current = null;
    setBodyState((current) => (current === seed ? '' : current));
  }, []);
  return {
    body,
    setBody,
    attachments,
    setAttachments,
    references,
    setReferences,
    contextChannels,
    setContextChannels,
    attemptFor,
    claimAttempt,
    restoreBody,
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

// ---------------------------------------------------------------------------------------------
// Posts on their way
// ---------------------------------------------------------------------------------------------

/**
 * The destinations ({@link postDestination}) with a `message.post` on its way, each with the
 * flight that holds it. Module scope, not a component's: a post outlives the Crew screen that sent
 * it (the person can leave Crew while it is out), and a Crew opened again must hold that channel's
 * Send until the answer comes, rather than let a second post go out beside the first. A post in
 * one channel never holds another channel's Send (RENDERER-4).
 */
const postsInFlight = new Map<string, object>();
const flightListeners = new Set<() => void>();

function notifyFlights(): void {
  for (const listener of [...flightListeners]) listener();
}

function subscribeFlights(listener: () => void): () => void {
  flightListeners.add(listener);
  return () => {
    flightListeners.delete(listener);
  };
}

/** Start a post to `destination`: its flight, or null while another is on its way there. */
function beginPost(destination: string): object | null {
  if (postsInFlight.has(destination)) return null;
  const flight = {};
  postsInFlight.set(destination, flight);
  notifyFlights();
  return flight;
}

/** The post `flight` was for has its answer. */
function endPost(destination: string, flight: object): void {
  if (postsInFlight.get(destination) !== flight) return;
  postsInFlight.delete(destination);
  notifyFlights();
}

/**
 * Whether a post to `channelId` on `connectionId` is on its way, sent from this Crew screen or from
 * an earlier one.
 */
export function usePostInFlight(connectionId: string, channelId: string): boolean {
  const destination = postDestination(connectionId, channelId);
  const read = useCallback(() => postsInFlight.has(destination), [destination]);
  return useSyncExternalStore(subscribeFlights, read, read);
}

// ---------------------------------------------------------------------------------------------
// The Crew screens open now
// ---------------------------------------------------------------------------------------------

/** A Crew screen a post's refusal can be told to: what it shows, and its error slot. */
interface SendScreen {
  selection: MutableRefObject<{ connectionId: string; channelId: string }>;
  reportError(message: string, source?: ErrorSource, code?: string): void;
}

/** The Crew screens open now, newest last. */
const openScreens: SendScreen[] = [];

/**
 * Keep this Crew screen among the open ones while it is mounted, so a refusal of a post an earlier
 * screen sent (the person left Crew while it was out, and came back) is told here rather than to
 * the screen that is gone.
 */
export function useOpenSendScreen(
  selection: SendScreen['selection'],
  reportError: SendScreen['reportError']
): void {
  useEffect(() => {
    const screen: SendScreen = { selection, reportError };
    openScreens.push(screen);
    return () => {
      const index = openScreens.indexOf(screen);
      if (index >= 0) openScreens.splice(index, 1);
    };
  }, [selection, reportError]);
}

/** The screen to tell about a post sent from `selection`'s screen: it if open, else the newest. */
function screenToTell(selection: SendScreen['selection']): SendScreen | null {
  return (
    openScreens.find((screen) => screen.selection === selection) ??
    openScreens[openScreens.length - 1] ??
    null
  );
}

/** SHA-256, as hex, of a post's fingerprint: what an attempt keeps instead of the post itself. */
async function fingerprintDigest(fingerprint: unknown): Promise<string> {
  const bytes = new TextEncoder().encode(JSON.stringify(fingerprint));
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('');
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
  /**
   * The connection and channel selected now, as of the latest render. Also this screen's identity
   * among the open ones ({@link useOpenSendScreen}).
   */
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
 * Single flight: a second Enter or Send while a post to the same channel is on its way does
 * nothing, even from a Crew screen opened after the one that sent it. The send is not optimistic:
 * the draft stays until the broker answers, and a retry of an unchanged payload to the same channel
 * reuses the same idempotency key, while any change rotates it. Success clears only what was sent
 * and never refreshes the verified workspace. Posting while a history page is shown returns to the
 * live tail.
 *
 * The key goes with the text (see {@link CrewDraftState}), so it lasts exactly as long as the text
 * can be sent again. An answer to the composer that sent it clears the text, and the attempt goes
 * with it. An answer after that composer moved on forgets the text kept aside for the channel (it
 * cannot come back over the conversation), and the attempt with it. A composer that still holds
 * the words (the person came back, or the view was observed again) keeps them, as they were told,
 * and their attempt, marked delivered: sending them again is this message to the broker, never a
 * second one, and the first edit lets it go, so the same words written anew are a new message.
 *
 * A post also reads the channel up to the posted message (Q3-10), silently, so the person's own
 * message never sits under the "New" rule. And the transfer records it forgot are re-listed at
 * once (Q3-03), so the Files tab stops calling a sent file "not sent" without waiting for a remount.
 *
 * A post belongs to the channel it was sent in (RENDERER-4). A person who sends in #methods and
 * moves to #analysis can write and send there while #methods' post is still on its way. If that
 * post is then refused, the composer on screen is not the one that sent it: the refusal goes to the
 * connection bar, naming #methods, whose kept draft still holds the text. While the person is still
 * in the channel it went to, it shows in the composer as before. When the screen that sent it has
 * closed, the refusal is told to the Crew screen open now, if any.
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
  const { body, attachments, references } = draft;
  const destination = postDestination(connectionId, channelId);
  return async () => {
    if (
      busy ||
      postsInFlight.has(destination) ||
      channel?.archived ||
      (!body.trim() && attachments.length === 0 && references.length === 0)
    )
      return;
    const flight = beginPost(destination);
    if (!flight) return;
    // Claimed before anything is awaited: if the text is put aside while the post is prepared,
    // this attempt goes with it and is filled in wherever it is.
    const attempt = draft.claimAttempt(connectionId, channelId);
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
        const digest = await fingerprintDigest({ connectionId, ...payload });
        const made: MessageAttempt =
          attempt.current?.digest === digest
            ? attempt.current
            : { digest, key: crypto.randomUUID() };
        attempt.current = made;
        let posted: unknown;
        try {
          posted = await request<unknown>(
            'message.post',
            { ...payload, idempotency_key: made.key },
            { mutation: true }
          );
        } catch (failure) {
          const screen = screenToTell(selection);
          // No Crew screen is open: the words wait in the kept draft for the person's return.
          if (!screen) return;
          const there = screen.selection.current;
          const inDestination =
            there.connectionId === connectionId && there.channelId === channelId;
          if (screen.selection === selection && inDestination) throw failure;
          if (inDestination) {
            screen.reportError(
              failureMessage(failure, crewActionCopy.actionFallback),
              'composer',
              failureCode(failure)
            );
            return;
          }
          // The composer on screen belongs to another channel now: say which post failed, in the
          // connection bar, rather than above a draft that was never sent.
          screen.reportError(
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
        // Sent: no earlier draft kept for this channel may come back over the conversation, and
        // its attempt goes with it.
        forgetStashedDraft(connectionId, channelId);
        if (current !== generation.current) {
          // A composer that still holds these words keeps them and this attempt, as delivered.
          if (attempt.current === made) attempt.current = { ...made, delivered: true };
          return;
        }
        attempt.current = null;
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
      endPost(destination, flight);
    }
  };
}

/** Forget every post on its way (vitest only): one left unanswered must not hold the next Send. */
function resetPostsInFlightForTests(): void {
  if (postsInFlight.size === 0) return;
  postsInFlight.clear();
  notifyFlights();
}

resetBetweenTests(resetPostsInFlightForTests);
