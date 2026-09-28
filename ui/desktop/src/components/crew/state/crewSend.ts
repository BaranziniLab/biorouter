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
import { isOutcomeUnknown } from '../api/errors';
import { composerCopy } from '../composer/copy';
import { messageTooLong, sendFailure, type SendFailureContext } from '../composer/sendFailure';
import type { Channel, CrewMessage, Snapshot } from '../crewApi';
import { clearPublishedTransfers } from '../crewTransfers';
import { cachedBlob } from '../files/blobMetadataCache';
import { visibleFileText } from '../files/fileName';
import { refreshCrewTransfers } from '../files/useCrewTransfers';
import { buildPeopleDirectory, personLabel } from '../identity';
import { channelName } from '../identity/objectNames';
import { crewActionCopy } from './copy';
import {
  forgetStashedDraft,
  noteKeptDraft,
  resetBetweenTests,
  type DraftAttempt,
  type MessageAttempt,
} from './draftStash';
import type {
  ActionKey,
  ActOptions,
  DraftFile,
  DraftReference,
  ErrorDetails,
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
  /**
   * Take `sent` out of the composer after it was posted: the whole text when that is all it holds,
   * or the start of it when the person kept typing while the post was on its way (QA M8). Text
   * that no longer starts with what was sent was edited, and stays as it is.
   */
  clearSentBody(sent: string): void;
}

/**
 * The composer's text once `sent` has been posted from it: what was typed after it, or nothing.
 * Text that does not start with `sent` was changed meanwhile and is left as it is.
 */
export function bodyAfterSending(current: string, sent: string): string {
  if (current === sent) return '';
  if (!sent || !current.startsWith(sent)) return current;
  const rest = current.slice(sent.length);
  return rest.trim() ? rest : '';
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
  const clearSentBody = useCallback((sent: string) => {
    // What is left is a new message, or nothing: the attempt made for `sent` does not go with it.
    // An edit of the sent words keeps them, and their attempt, as they are.
    if (bodyNow.current.startsWith(sent)) composerAttempt.current = null;
    setBodyState((current) => bodyAfterSending(current, sent));
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
    clearSentBody,
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

/**
 * What became of the last post to a destination, recorded before its flight ends, so whatever
 * watches the flight end (the timeline's "Sending…" row) reads the answer rather than guessing it
 * from the draft (QA M8, R-4):
 * - `accepted`: the broker took it and the composer that sent it let the words go. `messageId` is
 *   the message the broker answered with: a resend under the same key answers with the message
 *   already on screen, which is then delivered at once rather than drawn again as "Sending…".
 * - `kept`: taken, but the composer that sent it had moved on and still holds the words.
 * - `refused`: not taken.
 * - `unknown`: the bridge was lost after the post was written; the channel is read to find it.
 */
export type PostOutcome =
  | { kind: 'accepted'; messageId: string | null }
  | { kind: 'kept' }
  | { kind: 'refused' }
  | { kind: 'unknown' };

const postOutcomes = new Map<string, PostOutcome>();

/** The outcome of the last post to `channelId` on `connectionId` that has ended, if any. */
export function lastPostOutcome(connectionId: string, channelId: string): PostOutcome | null {
  return postOutcomes.get(postDestination(connectionId, channelId)) ?? null;
}

/**
 * Record what became of a post, as a send does before its flight ends (tests only): a stand-in
 * controller posts nothing, so a timeline under it is told the outcome this way.
 */
export function notePostOutcomeForTests(
  connectionId: string,
  channelId: string,
  outcome: PostOutcome
): void {
  postOutcomes.set(postDestination(connectionId, channelId), outcome);
}

/** Start a post to `destination`: its flight, or null while another is on its way there. */
function beginPost(destination: string): object | null {
  if (postsInFlight.has(destination)) return null;
  const flight = {};
  postsInFlight.set(destination, flight);
  postOutcomes.delete(destination);
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
  reportError(message: string, source?: ErrorSource, code?: string, details?: ErrorDetails): void;
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

/**
 * Codes the composer's own notes carry (never the daemon's): each has its own tone above the card
 * (`Composer.tsx`) and none takes the "Couldn't send." lead.
 */
export const POST_NOTE_CODES = {
  /** A post whose outcome is unknown, while the channel is read to find it (QA R-4). */
  checking: 'crew_post_checking',
  /** It was not found once the channel was read again: the draft and its key stay. */
  unconfirmed: 'crew_post_unconfirmed',
  /** It was found: sent after all. */
  confirmed: 'crew_post_confirmed',
} as const;
/** A message over the broker's limit, caught before it was sent (QA M5). Takes the lead. */
export const POST_TOO_LONG_CODE = 'crew_post_too_long';

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
  reportError(message: string, source?: ErrorSource, code?: string, details?: ErrorDetails): void;
  /**
   * The IDs of the channel's messages on screen as Send is pressed: a post whose outcome turns out
   * unknown is looked for among the others. Absent: none.
   */
  messageIdsNow?(): ReadonlySet<string>;
  /** How many verified views the observer has shown: a post's check waits for later ones. */
  verifiedViews?(): number;
}

/** The host and the file a refusal may name, from the view the post was sent from. */
function failureContext(
  snapshot: Snapshot | null,
  connectionId: string,
  attachments: readonly DraftFile[]
): SendFailureContext {
  const dir = buildPeopleDirectory(snapshot);
  const host = dir.host && !dir.host.isFormer ? personLabel(dir.host, 'authority', dir) : null;
  const only = attachments.length === 1 ? attachments[0] : null;
  const blob = only ? cachedBlob(connectionId, only.id) : null;
  const shared = blob ? snapshot?.channels.find((item) => item.id === blob.channel_id) : undefined;
  return {
    host,
    isHost: dir.viewerIsHost,
    fileName: only ? visibleFileText(only.name) : null,
    fileChannel: shared ? channelName(shared) : null,
  };
}

/**
 * Post the composer's draft to the selected channel.
 *
 * Single flight: a second Enter or Send while a post to the same channel is on its way does
 * nothing, even from a Crew screen opened after the one that sent it. The send is not optimistic:
 * the draft stays until the broker answers, and a retry of an unchanged payload to the same channel
 * reuses the same idempotency key, while any change rotates it. Success clears only what was sent,
 * so words typed while the post was on its way stay (QA M8), and never refreshes the verified
 * workspace. Posting while a history page is shown returns to the live tail.
 *
 * A message over the broker's 64 KB is not sent at all: the composer says so and keeps it (QA
 * M5). A refusal is told in words for a person (`composer/sendFailure.ts`), and belongs to the
 * channel it was sent in (`destination`): another channel's composer never shows it.
 *
 * The key goes with the text (see {@link CrewDraftState}), so it lasts exactly as long as the text
 * can be sent again. An answer to the composer that sent it clears the text, and the attempt goes
 * with it. An answer after that composer moved on forgets the text kept aside for the channel (it
 * cannot come back over the conversation), and the attempt with it. A composer that still holds
 * the words (the person came back, or the view was observed again) keeps them, as they were told,
 * and their attempt, marked delivered: sending them again is this message to the broker, never a
 * second one, and the first edit lets it go, so the same words written anew are a new message.
 *
 * A post whose outcome is unknown (QA R-4: the bridge died after the broker committed it) is not a
 * refusal: the composer says it is checking, and {@link usePostCheck} looks for the message in the
 * channel once it has been read again. Found, it is sent; not found, the composer says it could
 * not confirm it, and the words wait with the same key, so sending them again is safe.
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
    if (messageTooLong(body)) {
      reportError(composerCopy.tooLong, 'composer', POST_TOO_LONG_CODE, { destination });
      return;
    }
    const flight = beginPost(destination);
    if (!flight) return;
    // Claimed before anything is awaited: if the text is put aside while the post is prepared,
    // this attempt goes with it and is filled in wherever it is.
    const attempt = draft.claimAttempt(connectionId, channelId);
    const before = context.messageIdsNow?.() ?? new Set<string>();
    const views = context.verifiedViews?.() ?? 0;
    let outcome: PostOutcome = { kind: 'refused' };

    /**
     * Tell the Crew screen open now about this post: in its composer while it shows the channel
     * the post went to, else in its connection bar, naming that channel. No screen: the words wait
     * in the kept draft for the person's return, and what was said about them goes with them, to
     * be shown when they come back (a refusal while Crew was closed used to be told nowhere).
     */
    const tell = (words: string, code: string | undefined, transport: boolean, lead = true) => {
      const screen = screenToTell(selection);
      if (!screen) {
        noteKeptDraft(connectionId, channelId, {
          message: words,
          ...(code !== undefined ? { code } : {}),
          ...(transport ? { transport } : {}),
        });
        return;
      }
      const there = screen.selection.current;
      if (there.connectionId === connectionId && there.channelId === channelId) {
        screen.reportError(words, 'composer', code, { destination, transport });
        return;
      }
      screen.reportError(
        lead ? crewActionCopy.sendFailedIn(channelName(channel), words) : words,
        'global',
        code,
        { transport }
      );
    };

    try {
      await act('composer', 'send', async () => {
        if (!snapshot || observedPrivacy?.connectionId !== connectionId) {
          tell(crewActionCopy.sendPrivacyUnverified, undefined, false);
          return;
        }
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
          if (isOutcomeUnknown(failure)) {
            // Committed or not, nobody knows yet: look for it once the channel is read again. The
            // words and their key stay, so sending them again meanwhile is the same message.
            outcome = { kind: 'unknown' };
            startCheck(destination, {
              body,
              attachments: payload.attachments,
              before,
              views,
              attempt,
              made,
            });
            tell(composerCopy.checking, POST_NOTE_CODES.checking, false, false);
            return;
          }
          const refused = sendFailure(failure, failureContext(snapshot, connectionId, attachments));
          tell(refused.text, refused.code, refused.transport);
          return;
        }
        // Your own post is read: move the read position to it. A failure changes nothing on show.
        const answer =
          posted !== null && typeof posted === 'object'
            ? (posted as { sequence?: unknown; id?: unknown })
            : {};
        const sequence = answer.sequence;
        if (typeof sequence === 'string' && sequence)
          void markRead(channelId, sequence).catch(() => undefined);
        // Taken: a check still waiting for an earlier try of these words has its answer.
        endCheck(destination);
        try {
          await clearPublishedTransfers(connectionId, payload.attachments);
          // The shared transfer list re-lists only while a transfer moves: ask for it now.
          if (payload.attachments.length > 0) void refreshCrewTransfers(connectionId);
        } catch {
          if (current === generation.current)
            reportError(crewActionCopy.sendTransferRecordKept, 'composer', undefined, {
              destination,
            });
        }
        // Sent: no earlier draft kept for this channel may come back over the conversation, and
        // its attempt goes with it.
        forgetStashedDraft(connectionId, channelId);
        if (current !== generation.current) {
          outcome = { kind: 'kept' };
          // A composer that still holds these words keeps them and this attempt, as delivered.
          if (attempt.current === made) attempt.current = { ...made, delivered: true };
          return;
        }
        outcome = {
          kind: 'accepted',
          messageId: typeof answer.id === 'string' && answer.id ? answer.id : null,
        };
        attempt.current = null;
        if (historyPage.current !== null) {
          historyPage.current = null;
          setHistoryBefore(null);
          restartObservation();
        }
        draft.setReferences((items) =>
          items.filter((item) => !payload.references.includes(item.id))
        );
        draft.clearSentBody(body);
        draft.setAttachments((items) =>
          items.filter((item) => !payload.attachments.includes(item.id))
        );
      });
    } finally {
      postOutcomes.set(destination, outcome);
      endPost(destination, flight);
    }
  };
}

// ---------------------------------------------------------------------------------------------
// A post whose outcome is unknown
// ---------------------------------------------------------------------------------------------

/**
 * A post the bridge lost after writing it (QA R-4), waiting to be found in its channel. Module
 * scope, like the flights: the person may leave the channel, or Crew, before it is decided, and it
 * is decided when they are back.
 */
interface PostCheck {
  body: string;
  attachments: readonly string[];
  /** The channel's messages on screen as Send was pressed: the post is none of them. */
  before: ReadonlySet<string>;
  /** Verified views shown when it was sent: it is decided only on later ones. */
  views: number;
  /** The words' attempt and the one this post made, so a found post lets the key go. */
  attempt: DraftAttempt;
  made: MessageAttempt;
  /**
   * Which check this is. A channel holds one at a time, and a later post in doubt replaces the
   * one before, so what is decided about a check (that its time ran out) is kept by this, never
   * by the channel.
   */
  serial: number;
  /** When the check began, for {@link POST_CHECK_TIMEOUT_MS}. */
  startedAt: number;
  /**
   * The composer already said it could not confirm the post. The check stays, so a message that
   * turns up later still says it was sent and lets the words go.
   */
  unconfirmed?: boolean;
}

const postChecks = new Map<string, PostCheck>();

/**
 * How many verified views after the post a check waits for before it says it could not confirm
 * the post: the observer reads the channel's new messages between two state frames, so the second
 * one comes after a read that would have found it.
 */
export const POST_CHECK_VIEWS = 2;
/** The longest a check waits for those views while its channel is shown. */
export const POST_CHECK_TIMEOUT_MS = 30_000;

let postCheckSerial = 0;

function startCheck(destination: string, check: Omit<PostCheck, 'serial' | 'startedAt'>): void {
  postCheckSerial += 1;
  postChecks.set(destination, { ...check, serial: postCheckSerial, startedAt: Date.now() });
}

function endCheck(destination: string): void {
  postChecks.delete(destination);
}

/** Whether `message` is the checked post: the viewer's own, its words and files, and new. */
function isCheckedPost(check: PostCheck, message: CrewMessage, viewerId: string | null): boolean {
  if (viewerId === null || message.actor_id !== viewerId || message.run_id) return false;
  if (check.before.has(message.id) || message.body !== check.body) return false;
  const files = Array.isArray(message.attachments) ? message.attachments : [];
  return (
    files.length === check.attachments.length && check.attachments.every((id) => files.includes(id))
  );
}

export interface PostCheckInput {
  connectionId: string;
  channelId: string;
  /** The selected channel's messages, and whether they are its live tail, loaded to the end. */
  messages: readonly CrewMessage[];
  liveTailLoaded: boolean;
  /** Verified views shown so far (a count that only grows). */
  verifiedViews: number;
  viewerId: string | null;
  draft: CrewDraftState;
  markRead(channelId: string, sequence: string): Promise<void>;
  reportError(message: string, source?: ErrorSource, code?: string, details?: ErrorDetails): void;
}

/**
 * Decide the post whose outcome is unknown in the selected channel, if there is one (QA R-4):
 * - its message is in the channel: it was sent. The words go from the composer as after any send
 *   (only what was sent), their key with them, and the composer says so;
 * - the channel was read again ({@link POST_CHECK_VIEWS} verified views later, its live tail
 *   loaded) and it is not there, or {@link POST_CHECK_TIMEOUT_MS} passed: the composer says it
 *   could not confirm the post. The words and their key stay, so sending them again is the same
 *   message to the broker, which never posts it twice. A message that turns up after that still
 *   says it was sent; a send of the words that the broker takes ends the check.
 */
export function usePostCheck(input: PostCheckInput): void {
  const { connectionId, channelId, messages, liveTailLoaded, verifiedViews, viewerId } = input;
  const destination = postDestination(connectionId, channelId);
  const latest = useRef(input);
  latest.current = input;
  // The check whose time ran out, by its serial. Kept by channel, one check that timed out made the
  // next post in doubt there "could not be confirmed" after a single view, inviting the resend of
  // edited words that R-4 exists to prevent.
  const [timedOut, setTimedOut] = useState<number | null>(null);
  // Read as the screen renders (the composer's "Checking…" note is a render), so a check that
  // starts gets its own time bound at once, and a later one in the same channel its own again.
  const checkSerial = postChecks.get(destination)?.serial ?? null;

  useEffect(() => {
    const check = postChecks.get(destination);
    if (!check) return;
    const { draft, markRead, reportError } = latest.current;
    const found = messages.find((message) => isCheckedPost(check, message, viewerId));
    if (found) {
      endCheck(destination);
      if (check.attempt.current === check.made) check.attempt.current = null;
      forgetStashedDraft(connectionId, channelId);
      draft.clearSentBody(check.body);
      draft.setAttachments((items) => items.filter((item) => !check.attachments.includes(item.id)));
      if (typeof found.sequence === 'string' && found.sequence)
        void markRead(channelId, found.sequence).catch(() => undefined);
      reportError(composerCopy.confirmed, 'composer', POST_NOTE_CODES.confirmed, { destination });
      return;
    }
    if (check.unconfirmed) return;
    const read = liveTailLoaded && verifiedViews >= check.views + POST_CHECK_VIEWS;
    if (read || timedOut === check.serial) {
      check.unconfirmed = true;
      reportError(composerCopy.unconfirmed, 'composer', POST_NOTE_CODES.unconfirmed, {
        destination,
      });
    }
  }, [
    destination,
    connectionId,
    channelId,
    messages,
    liveTailLoaded,
    verifiedViews,
    viewerId,
    timedOut,
    checkSerial,
  ]);

  // The time bound of the check in the channel on screen, counted only while it is on screen.
  useEffect(() => {
    const check = postChecks.get(destination);
    if (!check || check.unconfirmed) return;
    const wait = Math.max(0, check.startedAt + POST_CHECK_TIMEOUT_MS - Date.now());
    const timer = setTimeout(() => setTimedOut(check.serial), wait);
    return () => clearTimeout(timer);
  }, [destination, verifiedViews, checkSerial]);
}

/** Forget every post on its way (vitest only): one left unanswered must not hold the next Send. */
function resetPostsInFlightForTests(): void {
  postOutcomes.clear();
  postChecks.clear();
  if (postsInFlight.size === 0) return;
  postsInFlight.clear();
  notifyFlights();
}

resetBetweenTests(resetPostsInFlightForTests);
