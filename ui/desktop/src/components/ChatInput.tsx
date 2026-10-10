import { onQuotedText, quoteReference } from '../utils/quotedText';
import React, { useRef, useState, useEffect, useLayoutEffect, useMemo, useCallback } from 'react';
import { annotationContextText, onArtifactAnnotation } from '../utils/annotationChannel';
import { ArrowUp, EyeOff } from './icons/app-icons';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';
import { Button } from './ui/button';
import type { View, ViewOptions } from '../utils/navigationUtils';
import Stop from './ui/Stop';
import { ChatState } from '../types/chatState';
import debounce from 'lodash/debounce';
import { LocalMessageStorage } from '../utils/localMessageStorage';
import { ToolsChip } from './bottom_menu/ToolsChip';
import { ModelEffortChip } from './bottom_menu/ModelEffortChip';
import { ComposerFooter } from './bottom_menu/ComposerFooter';
import { draftReasoningScope, sessionReasoningScope } from '../store/reasoningEffort';
import { InfoTip } from './ui/info-tip';
import { Spinner } from './ui/spinner';
import { ComposerChips } from './composer/ComposerChips';
import { ComposerPlusMenu, type ComposerTrigger } from './composer/ComposerPlusMenu';
import { useConfig } from './ConfigContext';
import { useModelAndProvider } from './ModelAndProviderContext';
import {
  NO_MODEL_COMPOSER_ACTION,
  NO_MODEL_COMPOSER_HINT,
  hasNoModelConfigured,
} from './composerNoProvider';
import MentionPopover, { DisplayItemWithMatch } from './MentionPopover';
import type { ModelCostRow } from '../hooks/useCostTracking';
import { DroppedFile, useFileDrop } from '../hooks/useFileDrop';
import { useDiverge } from '../hooks/useDiverge';
import { Workflow } from '../workflow';
import MessageQueue, { canSteerMessage } from './MessageQueue';
import { steerUnavailableReason } from './privacy/steerUnavailableCopy';
import { detectInterruption } from '../utils/interruptionDetector';
import { getSession, llamacppStatus, Message } from '../api';
import { userActionHeaders } from '../utils/userAction';
import type { SessionClassification } from '../api/types.gen';
import { getInitialWorkingDir } from '../utils/workingDir';
import { getPredefinedModelsFromEnv } from './settings/models/predefinedModelsUtils';
import { getSteerShortcutText } from '../utils/keyboardShortcuts';
import { COMPOSER_COPY } from './composer/copy';
import type { UserAttachment } from '../types/message';
import { useStopAcknowledgement } from '../hooks/useStopAcknowledgement';
import { isRunningState, type PinnedModelView } from '../hooks/chatStreamStore';
import { toastWarning } from '../toasts';
import { useCrewComposerHold } from './crew/access/ChatCrewAccessBar';
import { cn } from '../utils';
import {
  appendComposerRef,
  joinComposerText,
  removeComposerRefAt,
  splitComposerText,
} from '../utils/composerRefs';
import { findRefTags } from '../utils/resourceRefs';
import { RESTORE_CHAT_INPUT_EVENT, composerRestoreIsFor } from '../utils/composerRestore';
import {
  EMPTY_COMPOSER_DRAFT,
  beginComposerSend,
  composerDraftVersion,
  giveBackToComposer,
  mergeComposerDraft,
  readComposerDraft,
  saveComposerDraft,
  subscribeComposerGiveBack,
  type ComposerDraft,
  type DraftImage,
} from '../utils/composerDrafts';
import {
  beginQueuedOffer,
  claimComposerQueue,
  composerQueueKey,
  deleteOwnedTempAttachments,
  endQueuedOffer,
  hasQueuedOfferInFlight,
  parkComposerQueue,
  returnQueuedOffer,
  subscribeQueuedOfferReturns,
  type QueuedMessage,
} from '../utils/composerQueues';

/**
 * Queued messages as a draft, for a composer with no chat to keep a queue for.
 * Every image a queued message carries is a temp file the composer staged
 * (`canUploadDroppedImage` requires a staged path), so each comes back as one.
 */
function draftOfQueuedMessages(messages: readonly QueuedMessage[]): ComposerDraft {
  return {
    text: messages
      .map((message) => message.content.trim())
      .filter(Boolean)
      .join('\n\n'),
    images: messages.flatMap((message) =>
      (message.attachments ?? [])
        .filter((attachment) => attachment.kind === 'image')
        .map((attachment, index) => ({
          id: `queued-${message.id}-${index}`,
          filePath: attachment.path,
          dataUrl: '',
        }))
    ),
    files: [],
  };
}

/**
 * A queued message that a composer which is gone could not send. A chat's goes
 * back to that chat's queue; a composer with no chat hands it to its own draft.
 */
function handBackQueuedMessage(
  key: string | null,
  draftKey: string | undefined,
  message: QueuedMessage
): void {
  if (key) returnQueuedOffer(key, message);
  else if (draftKey) giveBackToComposer(draftKey, draftOfQueuedMessages([message]));
}

interface PastedImage {
  id: string;
  dataUrl: string; // For immediate preview
  filePath?: string; // Path on filesystem after saving
  isLoading: boolean;
  error?: string;
}

/**
 * The staged images a message carries when it is handed back or kept: the ones
 * written to a temp file. One still being read has no file yet and nothing to
 * send; one with an error was never going to be sent.
 */
function draftImagesOf(images: readonly PastedImage[]): DraftImage[] {
  return images
    .filter((image) => image.filePath && !image.error)
    .map((image) => ({
      id: image.id,
      filePath: image.filePath as string,
      dataUrl: image.dataUrl,
    }));
}

/** Back to the composer's own shape. A preview not read yet is read on apply. */
function pastedImagesOf(images: readonly DraftImage[]): PastedImage[] {
  return images.map((image) => ({
    id: image.id,
    filePath: image.filePath,
    dataUrl: image.dataUrl,
    isLoading: !image.dataUrl,
  }));
}

/** Dropped files that have finished staging. */
function draftFilesOf(files: readonly DroppedFile[]): DroppedFile[] {
  return files.filter((file) => !file.isLoading);
}

// Constants for image handling
const MAX_IMAGES_PER_MESSAGE = 5;
const MAX_IMAGE_SIZE_MB = 3;

// The context window assumed until the model's own is known.
const TOKEN_LIMIT_DEFAULT = 128000; // fallback for custom models that the backend doesn't know about

// Manual compact trigger message - must match backend constant
const MANUAL_COMPACT_TRIGGER = '/compact';

// Client-side slash command: branch the conversation into a new chat. Handled
// entirely in the renderer (never sent to the agent).
const DIVERGE_TRIGGER = '/diverge';

/**
 * How many times a queued message may be offered to a submit that refuses it
 * before the composer stops retrying and says so.
 *
 * A refusal on the drain is an ORDERING artefact, not a busy signal, so this is
 * a bound rather than a poll. The store holds its `submitInFlight` latch until
 * the finishing turn's promise chain unwinds, and the `isLoading` edge that
 * starts the drain is produced INSIDE that chain, so the first offer of every
 * turn can land while the latch is still set. Each remaining step of that
 * unwind is a microtask, so one macrotask hop clears it; the two extra attempts
 * are margin for a further scheduling hop (React's passive-effect flush, the
 * store's rAF/timeout notify race). Anything still refusing after three is a
 * real "cannot send right now" (no session, or another turn already started),
 * which is reported and left QUEUED for the next drain rather than spun on.
 */
const QUEUE_DRAIN_ATTEMPTS = 3;
/**
 * The composer's input type: `--text-body`, 14 on a 20px line — THE SAME ROLE
 * the transcript sets message text in.
 *
 * This was briefly a one-off `text-[16px] leading-6`, taken from the supplied
 * composer mockups and from a request that the text be plainly "visible". It is
 * back on the scale, and the reason is what the app looks like as a whole rather
 * than what the composer looks like alone: a sentence is the SAME OBJECT before
 * and after you send it, and at 16px it visibly changed size on its way into the
 * transcript. Two type sizes for one sentence is a seam the eye catches
 * immediately, and it made the composer read as a separate application docked to
 * the chat rather than as its entry point.
 *
 * So the rule is: the composer, the user's own bubble and the model's prose all
 * render at `--text-body`. If the input should get larger, `--text-body` is what
 * moves, and all three move together.
 *
 * Kept as a named constant even though it now names one role, because the pull
 * toward a bespoke input size is clearly recurring — this is the second time —
 * and a constant with this note attached is what makes the next attempt a
 * decision rather than an accident.
 */
const COMPOSER_INPUT_TYPE_CLASS = 'text-body';

function canonicalMimeType(mimeType: string): string {
  const normalized = mimeType.toLowerCase().trim();
  return normalized === 'image/jpg' ? 'image/jpeg' : normalized;
}

function mimeTypeAllowed(mimeTypes: string[] | null, mimeType: string): boolean {
  if (!mimeTypes) return true;
  const canonical = canonicalMimeType(mimeType);
  return mimeTypes.some((allowedMimeType) => canonicalMimeType(allowedMimeType) === canonical);
}

async function validateImageDataUrl(dataUrl: string): Promise<void> {
  if (
    typeof Image === 'undefined' ||
    typeof HTMLImageElement === 'undefined' ||
    typeof HTMLImageElement.prototype.decode !== 'function'
  ) {
    return;
  }

  const img = new Image();
  img.src = dataUrl;
  await img.decode();
}

interface ModelLimit {
  pattern: string;
  context_limit: number;
}

interface ChatInputProps {
  sessionId: string | null;
  /**
   * The chat's classification as the chat stream's cached session row holds it.
   *
   * Preferred over this component's own read (below) whenever it is present: the
   * reply stream now states the POST-ratchet classification in its own first
   * frames, so the row is current from turn START rather than from whenever the
   * probe happened to fire.
   */
  sessionRowPrivacyTier?: SessionClassification;
  /**
   * Issue #56 / F2 — what THIS chat actually runs on (`BaseChat.chatBinding`):
   * the session row's own provider and model, or the one a turn reported when
   * the privacy barrier had to repair the binding mid-turn.
   *
   * ⚠ It is the composer's model chip and context gauge that this fixes. Both
   * stated the app's GLOBAL selection, while `restore_provider_from_session`
   * binds the row's — so a private chat running on Versa showed
   * `claude-opus-5` and measured its usage against Claude's 1M window instead
   * of the 400k one really in play. Preferring this binding is a no-op whenever
   * it names what was already selected, which is the common case.
   */
  effectiveModel?: PinnedModelView;
  /**
   * Send the composed message. Resolving FALSE means the submit was REFUSED
   * silently and the composer still owns the text (see
   * `ChatStreamController.handleSubmit`); the composer must then put the text
   * back, or keep the message queued, instead of clearing it. A handler that
   * resolves `undefined` predates the contract and counts as accepted.
   */
  handleSubmit: (e: React.FormEvent) => void | Promise<boolean | void>;
  chatState: ChatState;
  setChatState?: (state: ChatState) => void;
  onStop?: (continuationPending?: boolean) => boolean | void | Promise<boolean | void>;
  onAbandonContinuation?: () => void | Promise<void>;
  /** Keep composed text editable but prevent submission until an ownership gate resolves. */
  submissionBlocked?: boolean;
  /** BR-61 soft interrupt: inject text into the turn that is already running
   * (no cancel, no lost work). Resolves false when there was nothing to steer,
   * in which case the caller must send/queue the text normally. */
  onSteer?: (text: string) => Promise<boolean>;
  commandHistory?: string[];
  initialValue?: string;
  /**
   * The owning tab's draft address: a sessionless tab key, a tab+session key
   * for an existing chat, or Home's key. The owner retains it while the tab exists.
   *
   * With a key, what the composer holds is not this instance's alone: it is
   * seeded from, and saved on every change to, the draft in
   * `utils/composerDrafts.ts`, and a message a send did not take is handed back
   * THROUGH the key — even after this instance is gone. That is what lets an
   * unsent new chat survive the rebuilds a person never sees — a failed start
   * remounting the composer, a tab switch, a split, leaving /pair — and it is
   * the composer's only addressable identity: no other composer can be reached
   * under it. See that module for the lifetime and why it is bounded.
   */
  draftKey?: string;
  /** Shared with the session creator so the first reply inherits this draft’s choice. */
  reasoningDraftKey?: string;
  droppedFiles?: DroppedFile[];
  onFilesProcessed?: () => void;
  setView: (view: View, options?: ViewOptions) => void;
  totalTokens?: number;
  accumulatedInputTokens?: number;
  accumulatedOutputTokens?: number;
  /**
   * #22 — the transcript LENGTH, not the array. The composer only ever asks
   * "is the conversation empty?", and taking the whole array as a prop made
   * this 1900-line component re-render on every streamed token (the array
   * identity changes per event, the length almost never does).
   */
  messagesLength?: number;
  /**
   * #44 — authoritative working-dir lock, derived by the owner of the session
   * metadata (BaseChat, via `deriveWorkingDirLocked`). `messagesLength` alone
   * misleads while a resumed transcript hydrates (0 for a non-empty session)
   * and after a failed optimistic first submit (>0 for a server-empty
   * session), so when this prop is provided it wins; the `messagesLength > 0`
   * fallback keeps callers that do not track session metadata working.
   */
  workingDirLocked?: boolean;
  sessionCosts?: {
    [key: string]: {
      inputTokens: number;
      outputTokens: number;
      totalCost: number;
    };
  };
  /** Real per-model usage rows from the token ledger (Issue #1 breakdown). */
  modelCostRows?: ModelCostRow[];
  disableAnimation?: boolean;
  workflow?: Workflow | null;
  workflowAccepted?: boolean;
  initialPrompt?: string;
  toolCount: number;
  append?: (message: Message) => void;
  onWorkingDirChange?: (newDir: string) => void;
  /** Optional override for vision capability. When the chat is bound to a
   * specific session whose model differs from the user's global default
   * (notably tabs and split panes), the override reflects the session's actual
   * model. Falls back to the global ModelAndProviderContext flag when
   * undefined. */
  supportsVisionOverride?: boolean;
  supportedInputMimeTypesOverride?: string[] | null;
  /**
   * Take the keyboard focus when this composer MOUNTS (default true). Read at
   * mount only; later changes do nothing.
   *
   * False for a split pane that is not the focused one. Every pane's composer
   * mounts at once when /pair is rebuilt (coming back from Settings or Home), a
   * focus inside a pane makes that pane the focused one, and the last composer
   * to mount won: measured in the dev app, after resuming the draft showing in
   * the left pane, the caret and the focused pane were the right pane's chat.
   */
  autoFocus?: boolean;
}

export default function ChatInput({
  sessionId,
  sessionRowPrivacyTier,
  effectiveModel,
  handleSubmit,
  chatState = ChatState.Idle,
  setChatState,
  onStop,
  onAbandonContinuation,
  submissionBlocked = false,
  onSteer,
  commandHistory = [],
  initialValue = '',
  draftKey,
  reasoningDraftKey,
  droppedFiles = [],
  onFilesProcessed,
  setView,
  totalTokens,
  accumulatedInputTokens,
  accumulatedOutputTokens,
  messagesLength = 0,
  workingDirLocked,
  disableAnimation = false,
  sessionCosts,
  modelCostRows,
  workflowAccepted,
  initialPrompt,
  append: _append,
  onWorkingDirChange,
  supportsVisionOverride,
  supportedInputMimeTypesOverride,
  autoFocus = true,
}: ChatInputProps) {
  const [anonymousReasoningDraftKey] = useState(() => crypto.randomUUID());
  // A new chat's unsent message, as its tab last held it. Read ONCE, in the
  // first render, and used to seed state rather than applied by an effect: an
  // effect would run after a first render showing an empty box, and after the
  // `[initialValue]` effect below. The stamp read with it is how the draft
  // effects below tell whether anyone wrote under this key after this render.
  const [seed] = useState<{ draft: ComposerDraft | undefined; version: number } | undefined>(() =>
    draftKey
      ? { draft: readComposerDraft(draftKey), version: composerDraftVersion(draftKey) }
      : undefined
  );
  const seedDraft = seed?.draft;
  // The stamp of the store write this box reflects: the seed's, then each save
  // this composer makes.
  const draftVersionRef = useRef(seed?.version ?? 0);
  const [_value, setValue] = useState(seedDraft?.text ?? initialValue);
  const [displayValue, setDisplayValue] = useState(seedDraft?.text ?? initialValue); // For immediate visual feedback
  // (`isFocused` used to live here, mirroring the textarea's focus into React
  // purely so the card could paint a ring. The card now asks CSS directly with
  // `has-[textarea:focus]`, which is one source of truth instead of two and
  // cannot fall out of sync with the DOM the way a mirrored flag can.)
  const [pastedImages, setPastedImages] = useState<PastedImage[]>(() =>
    pastedImagesOf(seedDraft?.images ?? [])
  );
  const pastedImagesRef = useRef(pastedImages);
  pastedImagesRef.current = pastedImages;
  // What the box holds, as of the last render OR the last synchronous change
  // this component made without waiting for one (a send clearing it, a give-back
  // filling it). The draft is saved from these on the way out, and an unmount
  // can land between a state update and its render — so a ref that only
  // followed renders would save the box as it was BEFORE the send cleared it.
  const displayValueRef = useRef(displayValue);
  displayValueRef.current = displayValue;

  // Loading gates composer operations; live work drives the visual activity
  // indicator. LoadingConversation is intentionally only in the former.
  const isLoading = chatState !== ChatState.Idle;
  const isWorking = isRunningState(chatState);
  const wasLoadingRef = useRef(isLoading);
  const isLoadingNowRef = useRef(isLoading);
  isLoadingNowRef.current = isLoading;

  // The queue — renderer memory only, and the CHAT's rather than this
  // instance's: it is parked under the chat's key when this composer unmounts
  // and claimed by the next composer for the same chat. See
  // `utils/composerQueues.ts` for the two states a message can be in and why.
  const queueKey = composerQueueKey(sessionId);
  const queueKeyRef = useRef(queueKey);
  queueKeyRef.current = queueKey;
  const queueDraftKeyRef = useRef(draftKey);
  queueDraftKeyRef.current = draftKey;
  const [queuedMessages, setQueuedMessages] = useState<QueuedMessage[]>([]);
  const queuedMessagesRef = useRef(queuedMessages);
  queuedMessagesRef.current = queuedMessages;
  // Offers refused once and waiting for their next attempt's timer: out of the
  // queue, and not handed to any submit right now.
  const retryingQueuedOffersRef = useRef(new Map<string, QueuedMessage>());
  // Offers handed to a submit that has not answered. They belong to that submit.
  const pendingQueuedOfferIdsRef = useRef(new Set<string>());
  // A claimed queue whose turn ended while no composer was mounted: drain it at
  // the first idle render, as that turn's end would have.
  const drainWhenIdleRef = useRef(false);
  const queueDisposedRef = useRef(false);
  const queuePausedRef = useRef(false);
  // What `queuePausedRef` goes back to when a Stop & send that paused it settles.
  const pausedBeforeStopAndSendRef = useRef<boolean | null>(null);
  const editingMessageIdRef = useRef<string | null>(null);
  const continuationQueuedMessageIdRef = useRef<string | null>(null);
  const [lastInterruption, setLastInterruption] = useState<string | null>(null);
  const lastInterruptionRef = useRef(lastInterruption);
  lastInterruptionRef.current = lastInterruption;

  const { getProviders, read } = useConfig();
  const {
    getCurrentModelAndProvider,
    currentModel,
    currentProvider,
    modelConfigStatus,
    currentModelSupportsVision: globalSupportsVision,
    currentModelSupportedInputMimeTypes: globalSupportedInputMimeTypes,
  } = useModelAndProvider();
  // Prefer the session-scoped flag when provided. This matters when several
  // chats are open at once (each may be bound to a different model than the
  // user's global default) and after per-session model switches.
  const currentModelSupportsVision =
    supportsVisionOverride !== undefined ? supportsVisionOverride : globalSupportsVision;
  const currentModelSupportedInputMimeTypes =
    supportedInputMimeTypesOverride !== undefined
      ? supportedInputMimeTypesOverride
      : globalSupportedInputMimeTypes;
  const [tokenLimit, setTokenLimit] = useState<number>(TOKEN_LIMIT_DEFAULT);
  const [isTokenLimitLoaded, setIsTokenLimitLoaded] = useState(false);
  const [sessionWorkingDir, setSessionWorkingDir] = useState<string | null>(null);

  // Branch-the-conversation action shared with the message-level Diverge button.
  const { diverge } = useDiverge();

  useEffect(() => {
    if (!sessionId) {
      return;
    }

    const fetchSessionWorkingDir = async () => {
      try {
        const response = await getSession({
          path: { session_id: sessionId },
          // The row, not the transcript: `working_dir` is all this reads.
          query: { metadata_only: true },
          // Issue #56 Task 58: reading a private chat needs the proof-of-user.
          headers: await userActionHeaders(),
        });
        if (response.data?.working_dir) {
          setSessionWorkingDir(response.data.working_dir);
        }
      } catch (error) {
        console.error('[ChatInput] Failed to fetch session working dir:', error);
      }
    };

    fetchSessionWorkingDir();
  }, [sessionId]);

  /**
   * The chat's privacy tier, for the two composer surfaces that need it
   * (issue #56, §14.2 / §14.5): the model chip's dot and the extension
   * selector's pairing state.
   *
   * Its own effect rather than a second read inside the working-directory one
   * above, because it must re-read when a turn ENDS — the classification
   * ratchets on the provider bind, so a chat that becomes private mid-session
   * would otherwise keep showing the tier it had when the composer mounted.
   * Folding it into the working-directory fetch would also re-apply a
   * server-side `working_dir` over a change the user had just made locally.
   *
   * Left `undefined` on any failure. That is not "public": both consumers treat
   * an unresolved tier as "judge nothing", because walling a working tool on a
   * failed read is the same defect as hiding it.
   */
  const [ownPrivacyTierRead, setOwnPrivacyTierRead] = useState<SessionClassification | undefined>(
    undefined
  );

  /**
   * The chat's tier, with the STREAM's answer preferred over this component's
   * own read.
   *
   * ⚠ This is what retires the F-12 probe's premise. That probe exists because
   * "the composer cannot see the ratchet directly — no event announces it"; the
   * reply stream now does, in its first frames, so `sessionRowPrivacyTier` is
   * current from turn START while the probe cannot fire until the transcript
   * has grown. The probe is KEPT as the fallback: an observer surface, a
   * transcript view, or any caller that does not thread the row still needs an
   * answer, and its own read is the only one they have.
   *
   * Both sources are the daemon's own answer, so preferring the fresher one
   * cannot make the composer less restrictive than the truth — the property the
   * probe's ⚠ turns on.
   */
  const sessionPrivacyTier = sessionRowPrivacyTier ?? ownPrivacyTierRead;
  // Which chat `sessionPrivacyTier` is a statement about, and the ordering of the
  // reads that produced it. Refs rather than effect-locals because the reads are
  // now issued from two effects (the bind below and the turn watcher after it)
  // and both must share one generation counter — "last to land" is not "newest",
  // and a slow read from before a rebind must not answer for the chat after it.
  const tierSessionRef = useRef<string | null>(null);
  const tierGenerationRef = useRef(0);

  const readSessionPrivacyTier = useCallback(async () => {
    if (!sessionId) return;
    const issued = ++tierGenerationRef.current;
    try {
      const response = await getSession({
        path: { session_id: sessionId },
        // The row, not the transcript: `privacy_tier` is all this reads, and the
        // same query as the working-directory read above, so a mount's two reads
        // share one request (`utils/sessionReadCoalescing.ts`).
        query: { metadata_only: true },
        // Issue #56 Task 58: and this read is *about* the tier, so it is
        // exactly the read the gate refuses without the header.
        headers: await userActionHeaders(),
      });
      if (issued !== tierGenerationRef.current) return;
      if (tierSessionRef.current !== sessionId) return;
      if (response.data?.privacy_tier) {
        setOwnPrivacyTierRead(response.data.privacy_tier);
      }
    } catch (error) {
      console.error('[ChatInput] Failed to read the session privacy tier:', error);
    }
  }, [sessionId]);

  useEffect(() => {
    // Clear FIRST, on every rebind and not only on the no-session case.
    // `BaseChat` is keyed by tab rather than by session, so this component
    // survives a move from one chat to another; keeping the old value until the
    // new read lands (or forever, if it throws) makes both consumers assert the
    // previous chat's tier about this one. In the private -> public direction
    // that greys out every model the new chat may legitimately run, and in the
    // reverse it paints a Private dot on a chat with no such guarantee.
    tierSessionRef.current = sessionId;
    tierGenerationRef.current += 1;
    setOwnPrivacyTierRead(undefined);

    if (!sessionId) {
      return;
    }

    void readSessionPrivacyTier();
    window.addEventListener('message-stream-finished', readSessionPrivacyTier);
    return () => {
      window.removeEventListener('message-stream-finished', readSessionPrivacyTier);
    };
  }, [sessionId, readSessionPrivacyTier]);

  /**
   * v1.89.0 F-12: re-read once the running turn has proved the daemon is past
   * the point where it RAISES the tier.
   *
   * A session is created `public` (`privacy_tier TEXT NOT NULL DEFAULT 'public'`
   * in `session_manager.rs`) and the daemon ratchets it as it starts a turn —
   * the stored `privacy_reason` reads `turn:<provider>`. The bind read above
   * fires between those two moments, and on the paths that create the chat
   * (Home's composer submitting, or a blank tab's first message, which keeps its
   * tab id and so does not remount this component) it reliably loses that race.
   * Nothing then re-read until the turn ENDED, so for the whole of a turn — 12 s
   * for a small local model, minutes for a real one — the extension menu labelled
   * every private extension "Unavailable in this chat (public model)" on a chat
   * the daemon had already classified private, and depressed its own
   * "Enable all (N)" to match. That is F-12, and a reload cleared it because a
   * reload re-reads.
   *
   * The composer cannot see the ratchet directly — no event announces it, which
   * is the same missing signal already written up as a KNOWN GAP over the tab
   * dot in `ChatGroupsShell.useSessionPrivacyTiers`. What it can see is the
   * transcript growing: the first message the daemon streams back is emitted
   * strictly after the turn started, hence strictly after the raise was written.
   * So: arm on the turn, fire the single re-read when the transcript first grows
   * under it.
   *
   * ⚠ **This only ever adopts the daemon's own answer, so it cannot make the
   * label less restrictive than the daemon is.** It closes the window in which
   * the composer was MORE restrictive than the truth; `extensionPairingRefused`
   * still treats an unresolved tier as "judge nothing", and enforcement was never
   * here at all (Gates C/E/F, `crates/biorouter/src/privacy/`).
   *
   * ⚠ **Once per turn, and never once the answer is `private`.** Within a chat
   * the ratchet only ever raises — so a second read during the same turn cannot
   * change the answer and would only ask the daemon again. (The read is
   * `metadata_only` now; it used to re-fetch the whole conversation too.)
   * A declassification lowers the tier by explicit user action and is still
   * picked up by the turn-end refresh and the next bind, exactly as before;
   * "still says private" is the safe direction to be wrong in meanwhile.
   */
  const turnTierProbeRef = useRef<{ armed: boolean; fired: boolean; messages: number }>({
    armed: false,
    fired: false,
    messages: 0,
  });
  useEffect(() => {
    const probe = turnTierProbeRef.current;
    if (chatState === ChatState.Idle) {
      probe.armed = false;
      probe.fired = false;
      return;
    }
    if (!probe.armed) {
      probe.armed = true;
      probe.messages = messagesLength;
      return;
    }
    if (probe.fired || messagesLength === probe.messages) return;
    probe.fired = true;
    if (!sessionId || sessionPrivacyTier === 'private') return;
    void readSessionPrivacyTier();
  }, [chatState, messagesLength, sessionId, sessionPrivacyTier, readSessionPrivacyTier]);

  // Save queue state (paused/interrupted) to storage
  useEffect(() => {
    try {
      window.sessionStorage.setItem(
        'biorouter-queue-paused',
        JSON.stringify(queuePausedRef.current)
      );
    } catch (error) {
      console.error('Error saving queue pause state:', error);
    }
  }, [queuedMessages]); // Save when queue changes

  useEffect(() => {
    try {
      window.sessionStorage.setItem(
        'biorouter-queue-interruption',
        JSON.stringify(lastInterruption)
      );
    } catch (error) {
      console.error('Error saving queue interruption state:', error);
    }
  }, [lastInterruption]);

  // Cleanup effect - save final state on component unmount
  useEffect(() => {
    return () => {
      // Save final queue state when component unmounts
      try {
        window.sessionStorage.setItem(
          'biorouter-queue-paused',
          JSON.stringify(queuePausedRef.current)
        );
        window.sessionStorage.setItem(
          'biorouter-queue-interruption',
          JSON.stringify(lastInterruption)
        );
      } catch (error) {
        console.error('Error saving queue state on unmount:', error);
      }
    };
  }, [lastInterruption]); // Include lastInterruption in dependency array

  // Timers for the bounded re-offer below. Tracked so unmounting cannot leave a
  // submit firing out of a composer that is gone.
  const queueRetryTimersRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set());

  // The queue's lifetime across composers: claim the chat's parked queue on
  // mount, park it on unmount. LAYOUT effects, and on mount/unmount only (the
  // key is read from its ref, so a new chat's first message binding a session
  // is not mistaken for an unmount): a pane rebuild renders the replacement
  // BEFORE this instance unmounts, so a claim made while rendering would read
  // the store one park too early. React runs every unmount's layout cleanup
  // before any mount's layout effect in the same commit, so the claim below
  // sees what the composer it replaces parked, and before anything is painted.
  useLayoutEffect(() => {
    queueDisposedRef.current = false;
    const key = queueKeyRef.current;
    const claimed = key ? claimComposerQueue(key) : undefined;
    if (key && claimed) {
      const current = queuedMessagesRef.current;
      const adopted = [
        ...claimed.messages.filter((message) => !current.some((held) => held.id === message.id)),
        ...current,
      ];
      queuedMessagesRef.current = adopted;
      setQueuedMessages(adopted);
      queuePausedRef.current = claimed.paused;
      if (claimed.interruption) setLastInterruption(claimed.interruption);
      // Not while a message of this chat is still with a submit: it answers into
      // a turn this composer will see end, and draining now would only race it.
      if (claimed.sendWhenIdle && !hasQueuedOfferInFlight(key)) drainWhenIdleRef.current = true;
    }
    const timers = queueRetryTimersRef.current;
    const retrying = retryingQueuedOffersRef.current;
    const pending = pendingQueuedOfferIdsRef.current;
    return () => {
      queueDisposedRef.current = true;
      timers.forEach((timer) => clearTimeout(timer));
      timers.clear();

      // Everything this composer holds that no submit has: offers between
      // attempts (they were being sent, so they go first), then the queue. An
      // offer a submit still has is left to that submit's answer.
      const waiting = [...retrying.values()];
      retrying.clear();
      const keep = [
        ...waiting,
        ...queuedMessagesRef.current.filter(
          (message) => !pending.has(message.id) && !waiting.some((w) => w.id === message.id)
        ),
      ];
      if (keep.length === 0) return;
      const keyNow = queueKeyRef.current;
      if (keyNow) {
        parkComposerQueue(keyNow, {
          messages: keep,
          paused: pausedBeforeStopAndSendRef.current ?? queuePausedRef.current,
          interruption: lastInterruptionRef.current,
          sendWhenIdle: waiting.length > 0 || isLoadingNowRef.current,
        });
      } else if (queueDraftKeyRef.current) {
        // No chat to send it to later: back to the draft it was typed under.
        giveBackToComposer(queueDraftKeyRef.current, draftOfQueuedMessages(keep));
      }
    };
  }, []);

  // A message of this chat that a gone composer's submit did not take.
  useLayoutEffect(() => {
    if (!queueKey) return;
    return subscribeQueuedOfferReturns(queueKey, (message) => {
      queuedMessagesRef.current = [
        message,
        ...queuedMessagesRef.current.filter((queued) => queued.id !== message.id),
      ];
      setQueuedMessages((prev) =>
        prev.some((queued) => queued.id === message.id) ? prev : [message, ...prev]
      );
      drainWhenIdleRef.current = true;
    });
  }, [queueKey]);

  useEffect(
    () => () => {
      if (continuationQueuedMessageIdRef.current) {
        continuationQueuedMessageIdRef.current = null;
        void onAbandonContinuation?.();
      }
    },
    [onAbandonContinuation]
  );

  /**
   * Send a message that came out of the queue, and KEEP IT if the submit
   * refuses it.
   *
   * Callers take the message out of the queue BEFORE calling: the accepted
   * path's promise does not resolve until the whole turn is over, so holding
   * the message until then would leave the "Next:" chip up for the length of
   * the turn and hand the next drain edge the same message to send again.
   * Refusal is therefore what this reacts to. It re-offers the message up to
   * `QUEUE_DRAIN_ATTEMPTS` times, and the message stays OUT of the queue while
   * it does, so no other control (Stop and send, steer, a second drain edge)
   * can pick up the same message and send it twice. Only a message that is
   * still refused at the bound goes back into the queue, at the head, with a
   * toast: visible and re-sendable, never silently dropped.
   *
   * The composer can unmount while an offer is out. Between attempts, the
   * unmount parks the message for the chat (the timer is cancelled with it). With
   * a submit, it stays that submit's: taken, it is done; not taken, it goes back
   * to the chat's queue through `utils/composerQueues.ts` — never both, which is
   * what used to show a running message as "Next" and send it again.
   */
  const offerQueuedMessage = useCallback(
    (message: QueuedMessage) => {
      // Captured at the offer: an answer arriving after this composer is gone
      // still belongs to the chat (or draft) the message was queued in.
      const key = queueKeyRef.current;
      const draftKeyAtOffer = queueDraftKeyRef.current;
      if (queueDisposedRef.current) {
        // A late caller on a composer that is gone (a refused steer, a resume
        // timer). The message is not in any queue any more, so hand it on.
        handBackQueuedMessage(key, draftKeyAtOffer, message);
        return;
      }
      LocalMessageStorage.addMessage(message.content);
      const offer = (attempt: number) => {
        if (queueDisposedRef.current) return;
        retryingQueuedOffersRef.current.delete(message.id);
        pendingQueuedOfferIdsRef.current.add(message.id);
        if (key) beginQueuedOffer(key, message.id);
        const submitted = handleSubmit(
          new CustomEvent('submit', {
            detail: { value: message.content, attachments: message.attachments ?? [] },
          }) as unknown as React.FormEvent
        );
        void Promise.resolve(submitted).then((accepted) => {
          pendingQueuedOfferIdsRef.current.delete(message.id);
          if (key) endQueuedOffer(key, message.id);
          // Only an explicit `false` is a refusal: a handler predating the
          // contract resolves `undefined` and must not be read as one.
          if (accepted !== false) {
            if (continuationQueuedMessageIdRef.current === message.id) {
              continuationQueuedMessageIdRef.current = null;
            }
            return;
          }
          // Not taken, and the composer that offered it is gone: no timer, no
          // state on a component that no longer exists. The message goes back
          // to whatever holds this chat's queue now.
          if (queueDisposedRef.current) {
            handBackQueuedMessage(key, draftKeyAtOffer, message);
            return;
          }
          if (attempt < QUEUE_DRAIN_ATTEMPTS) {
            // Next macrotask, which is all the in-flight submit's promise chain
            // needs to unwind and release its latch. Deliberately not a poll:
            // the attempt count is the whole retry budget.
            retryingQueuedOffersRef.current.set(message.id, message);
            const timer = setTimeout(() => {
              queueRetryTimersRef.current.delete(timer);
              offer(attempt + 1);
            }, 0);
            queueRetryTimersRef.current.add(timer);
            return;
          }
          setQueuedMessages((prev) =>
            prev.some((queued) => queued.id === message.id) ? prev : [message, ...prev]
          );
          toastWarning({
            title: 'Message still queued',
            msg: 'Biorouter could not send it while the chat was busy. It is still in the queue, ready to send.',
          });
        });
      };
      offer(1);
    },
    [handleSubmit]
  );

  // Queue processing
  useEffect(() => {
    // A turn running now ends with its own edge, so a claimed queue's pending
    // drain is that edge's.
    if (isLoading) drainWhenIdleRef.current = false;
    const turnEnded = wasLoadingRef.current && !isLoading;
    const missedTurnEnd = drainWhenIdleRef.current && !isLoading;
    if ((turnEnded || missedTurnEnd) && queuedMessages.length > 0) {
      drainWhenIdleRef.current = false;
      // After an interruption, we should process the interruption message immediately
      // The queue is only truly paused if there was an interruption AND we want to keep it paused
      const shouldProcessQueue = !queuePausedRef.current || lastInterruption;

      if (shouldProcessQueue) {
        const nextMessage = queuedMessages[0];
        setQueuedMessages((prev) => {
          const newQueue = prev.filter((queued) => queued.id !== nextMessage.id);
          // If queue becomes empty after processing, clear the paused state
          if (newQueue.length === 0) {
            queuePausedRef.current = false;
            setLastInterruption(null);
          }
          return newQueue;
        });
        offerQueuedMessage(nextMessage);

        // Clear the interruption flag after processing the interruption message
        if (lastInterruption) {
          setLastInterruption(null);
          // Keep the queue paused after sending the interruption message
          // User can manually resume if they want to continue with queued messages
          queuePausedRef.current = true;
        }
      }
    }
    wasLoadingRef.current = isLoading;
  }, [isLoading, queuedMessages, offerQueuedMessage, lastInterruption]);
  const [mentionPopover, setMentionPopover] = useState<{
    isOpen: boolean;
    position: { x: number; y: number };
    query: string;
    mentionStart: number;
    selectedIndex: number;
    isSlashCommand: boolean;
  }>({
    isOpen: false,
    position: { x: 0, y: 0 },
    query: '',
    mentionStart: -1,
    selectedIndex: 0,
    isSlashCommand: false,
  });
  const mentionPopoverRef = useRef<{
    getDisplayFiles: () => DisplayItemWithMatch[];
    selectFile: (index: number) => void;
  }>(null);

  // Update internal value when initialValue changes.
  //
  // CHANGES only, not the mount. On mount the state was just initialised, so
  // this was a no-op there — except for a composer seeded with a new chat's
  // unsent message, which it would blank, deleting that message's staged
  // images from disk on the way. StrictMode runs it twice on mount; both runs
  // see an unchanged value.
  const appliedInitialValueRef = useRef(initialValue);
  useEffect(() => {
    if (appliedInitialValueRef.current === initialValue) return;
    appliedInitialValueRef.current = initialValue;
    setValue(initialValue);
    setDisplayValue(initialValue);

    // Use a functional update to get the current pastedImages
    // and perform cleanup. This avoids needing pastedImages in the deps.
    setPastedImages((currentPastedImages) => {
      currentPastedImages.forEach((img) => {
        if (img.filePath) {
          window.electron.deleteTempFile(img.filePath);
        }
      });
      return []; // Return a new empty array
    });

    // Reset history index when input is cleared
    setHistoryIndex(-1);
    setIsInGlobalHistory(false);
    setHasUserTyped(false);
  }, [initialValue]); // Keep only initialValue as a dependency

  // Handle workflow prompt updates
  useEffect(() => {
    // If workflow is accepted and we have an initial prompt, and no messages yet, and we haven't set it before
    if (workflowAccepted && initialPrompt && messagesLength === 0) {
      setDisplayValue(initialPrompt);
      setValue(initialPrompt);
      setTimeout(() => {
        textAreaRef.current?.focus();
      }, 0);
    }
  }, [workflowAccepted, initialPrompt, messagesLength]);

  // State to track if the IME is composing (i.e., in the middle of Japanese IME input)
  const [isComposing, setIsComposing] = useState(false);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [savedInput, setSavedInput] = useState('');
  const [isInGlobalHistory, setIsInGlobalHistory] = useState(false);
  const [hasUserTyped, setHasUserTyped] = useState(() => Boolean(seedDraft?.text));
  const textAreaRef = useRef<HTMLTextAreaElement>(null);
  const timeoutRefsRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set());

  // A region the user selected in the preview panel arrives here as an already
  // written PNG. It joins `pastedImages` rather than getting a channel of its
  // own, so it inherits the whole staged-attachment contract for free — the
  // thumbnail strip, the hover-remove, the per-message cap, the vision-model
  // gate, and the path→base64 conversion at submit. A parallel mechanism would
  // have had to re-earn every one of those.
  useEffect(() => {
    return onArtifactAnnotation(sessionId, (annotation) => {
      // Deliberately NOT gated on vision support the way `handlePaste` is.
      // A paste can be incidental; dragging a rectangle cannot, and silently
      // dropping it would look like a broken feature. Staging it instead makes
      // the existing `visionMismatch` bar appear, which says the model cannot
      // read images and lets the user remove the chip or switch models.
      const id = `annotation-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      if (pastedImagesRef.current.length >= MAX_IMAGES_PER_MESSAGE) {
        window.electron.deleteTempFile(annotation.imagePath);
        toastWarning({
          title: 'Region not attached',
          msg: `A message can contain at most ${MAX_IMAGES_PER_MESSAGE} images.`,
        });
        return;
      }
      const staged = {
        id,
        dataUrl: '',
        filePath: annotation.imagePath,
        isLoading: true,
      };
      pastedImagesRef.current = [...pastedImagesRef.current, staged];
      setPastedImages(pastedImagesRef.current);
      // The thumbnail is read back from the file the main process wrote; the
      // panel never had the bytes in the first place.
      void window.electron
        ?.readTempImageAsBase64(annotation.imagePath)
        .then(({ data, mimeType }) => {
          setPastedImages((current) =>
            current.map((image) =>
              image.id === id
                ? { ...image, dataUrl: `data:${mimeType};base64,${data}`, isLoading: false }
                : image
            )
          );
          setDisplayValue((current) => {
            const context = annotationContextText(annotation);
            return current.trim() ? `${current.trimEnd()}\n\n${context}\n` : `${context}\n`;
          });
        })
        .catch(() => {
          window.electron.deleteTempFile(annotation.imagePath);
          setPastedImages((current) => current.filter((image) => image.id !== id));
          toastWarning({
            title: 'Region not attached',
            msg: 'The selected region could not be read.',
          });
        });
      textAreaRef.current?.focus();
    });
  }, [sessionId]);

  // (The `insert-chat-input` channel used to live here. Its only dispatcher was
  // the landing state's suggestion chips, which are gone, so the listener went
  // with them rather than being left as a receiver nobody calls. The `restore-`
  // channel above is a different mechanism and is still live.)

  // Use shared file drop hook for ChatInput
  const {
    droppedFiles: localDroppedFiles,
    setDroppedFiles: setLocalDroppedFiles,
    isDraggingOver,
    handleDrop: handleLocalDrop,
    handleDragEnter: handleLocalDragEnter,
    handleDragOver: handleLocalDragOver,
    handleDragLeave: handleLocalDragLeave,
  } = useFileDrop(() =>
    // Files the parent still holds are shown from there, not twice.
    (seedDraft?.files ?? []).filter((file) => !droppedFiles.some((held) => held.id === file.id))
  );

  // Merge local dropped files with parent dropped files. Keep every dropped
  // item visible: model capability determines whether an image is uploaded as
  // content or sent as a filesystem path, never whether the drop disappears.
  const allDroppedFiles = useMemo(() => {
    return [...droppedFiles, ...localDroppedFiles];
  }, [droppedFiles, localDroppedFiles]);

  // ---- What the box holds, and handing a message back to it -----------------
  //
  // "Your message was kept" promises the MESSAGE: its text, its staged images
  // and its dropped files. It used to keep the text alone — the image was gone
  // from a failed start while the toast said otherwise, measured on 1.90.4.
  const localDroppedFilesRef = useRef(localDroppedFiles);
  localDroppedFilesRef.current = localDroppedFiles;
  const parentDroppedFilesRef = useRef(droppedFiles);
  parentDroppedFilesRef.current = droppedFiles;
  const draftKeyRef = useRef(draftKey);
  draftKeyRef.current = draftKey;

  /** Everything the person would lose if this composer vanished right now. */
  const heldDraft = useCallback(
    (): ComposerDraft => ({
      text: displayValueRef.current,
      images: draftImagesOf(pastedImagesRef.current),
      files: draftFilesOf([...parentDroppedFilesRef.current, ...localDroppedFilesRef.current]),
    }),
    []
  );

  /**
   * Show `next`, replacing what the box holds. Callers pass what the box holds
   * MERGED with what came back (`takeBack`), never a bare give-back — so this
   * cannot replace anything the person typed. The refs move with the state,
   * synchronously, so an unmount before the next render saves what is shown.
   */
  const showDraft = useCallback(
    (next: ComposerDraft, { focus = true }: { focus?: boolean } = {}) => {
      displayValueRef.current = next.text;
      setDisplayValue(next.text);
      setValue(next.text);
      const images = pastedImagesOf(next.images);
      pastedImagesRef.current = images;
      setPastedImages(images);
      const parentIds = new Set(parentDroppedFilesRef.current.map((file) => file.id));
      const local = next.files.filter((file) => !parentIds.has(file.id));
      localDroppedFilesRef.current = local;
      setLocalDroppedFiles(local);
      if (next.text.trim()) setHasUserTyped(true);
      if (focus) textAreaRef.current?.focus();
      // A preview that was never read — an image handed back by path — is read
      // back from the file it names, as an annotation's is.
      for (const image of next.images) {
        if (image.dataUrl) continue;
        void window.electron
          ?.readTempImageAsBase64(image.filePath)
          .then(({ data, mimeType }) => {
            setPastedImages((current) =>
              current.map((candidate) =>
                candidate.id === image.id
                  ? { ...candidate, dataUrl: `data:${mimeType};base64,${data}`, isLoading: false }
                  : candidate
              )
            );
          })
          .catch(() => {
            setPastedImages((current) => current.filter((candidate) => candidate.id !== image.id));
          });
      }
    },
    [setLocalDroppedFiles]
  );

  /** Merge a message this composer did not keep back into the box. */
  const takeBack = useCallback(
    (returned: ComposerDraft): ComposerDraft => {
      const next = mergeComposerDraft(heldDraft(), returned);
      showDraft(next);
      return next;
    },
    [heldDraft, showDraft]
  );

  // A NEW chat's composer (and Home's): the draft under its key is where the box
  // lives between composers. The seed above is the way in. These are the way out
  // and the addressed way back.
  //
  // ⚠ SAVED ON EVERY CHANGE, never only on the way out. The composer that
  // replaces this one reads its seed while it RENDERS, and React renders the
  // replacement before it runs this one's unmount — splitting a pane, closing
  // the other half of a split, dragging a tab into a new pane all rebuild the
  // composer in one commit. A save made only at unmount therefore reached the
  // replacement one save late: measured in the production bundle, "PROD A PROD
  // B" came back as "PROD A", and a tab dragged before its first save arrived
  // empty. Layout effects, so the store is current before anything else can
  // render.
  //
  // Mounting under a key: anything written there since this composer read its
  // seed (a give-back landing between its render and its commit, or the composer
  // it replaced saving on its way out) is newer than the seed, so the box adopts
  // it instead of the save below writing the stale seed over it. Then listen for
  // give-backs, which land merged into whatever the box holds by then.
  useLayoutEffect(() => {
    if (!draftKey) return;
    if (composerDraftVersion(draftKey) !== draftVersionRef.current) {
      showDraft(readComposerDraft(draftKey) ?? EMPTY_COMPOSER_DRAFT, { focus: false });
      draftVersionRef.current = composerDraftVersion(draftKey);
    }
    const unsubscribe = subscribeComposerGiveBack(draftKey, (returned) => {
      draftVersionRef.current = saveComposerDraft(draftKey, takeBack(returned));
    });
    return () => {
      unsubscribe();
      // On the way out, only if nobody else has written here since this
      // composer last did: a composer must never put its older copy back over
      // a newer one. Every change was saved as it happened, so this matters only
      // for a change made and unmounted before it could commit.
      if (composerDraftVersion(draftKey) === draftVersionRef.current) {
        draftVersionRef.current = saveComposerDraft(draftKey, heldDraft());
      }
    };
  }, [draftKey, heldDraft, showDraft, takeBack]);

  useLayoutEffect(() => {
    if (!draftKey) return;
    // Declared after the mount effect above so a newer store copy is adopted
    // before this saves; `heldDraft` reads the refs that adoption just moved.
    if (composerDraftVersion(draftKey) !== draftVersionRef.current) return;
    draftVersionRef.current = saveComposerDraft(draftKey, heldDraft());
  }, [draftKey, heldDraft, displayValue, pastedImages, localDroppedFiles, droppedFiles]);

  // A message handed back to an EXISTING chat's composer by that chat's id
  // (`BaseChat.returnInitialMessageToComposer`). `composerRestoreIsFor` refuses
  // any restore that does not name this composer's own chat, so a composer with
  // no chat — a new tab's, Home's — can never be reached this way; and it is
  // merged, so text the person is typing here is never replaced by it. Its
  // attachments come back too: the detail names their files.
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<unknown>).detail;
      if (!composerRestoreIsFor(detail, sessionId)) return;
      const stamp = Date.now();
      takeBack({
        text: detail.value ?? '',
        images: (detail.attachments ?? [])
          .filter((attachment) => attachment.kind === 'image' && attachment.path)
          .map((attachment, index) => ({
            id: `restored-${stamp}-${index}`,
            filePath: attachment.path,
            dataUrl: '',
          })),
        files: [],
      });
    };
    window.addEventListener(RESTORE_CHAT_INPUT_EVENT, handler);
    return () => window.removeEventListener(RESTORE_CHAT_INPUT_EVENT, handler);
  }, [sessionId, takeBack]);

  const currentModelAcceptsMimeType = useCallback(
    (mimeType: string) =>
      currentModelSupportsVision &&
      Boolean(mimeType) &&
      mimeTypeAllowed(currentModelSupportedInputMimeTypes, mimeType),
    [currentModelSupportedInputMimeTypes, currentModelSupportsVision]
  );

  const canUploadDroppedImage = useCallback(
    (file: DroppedFile) =>
      currentModelSupportsVision &&
      file.isImage &&
      currentModelAcceptsMimeType(file.type) &&
      file.canUploadAsImage === true &&
      !file.error &&
      !file.isLoading &&
      Boolean(file.stagedPath),
    [currentModelAcceptsMimeType, currentModelSupportsVision]
  );

  const canSendDroppedFileAsPath = useCallback(
    (file: DroppedFile) =>
      Boolean(file.sourcePath || file.path) &&
      !file.isLoading &&
      (!file.isImage || !canUploadDroppedImage(file)),
    [canUploadDroppedImage]
  );

  const canSendDroppedFile = useCallback(
    (file: DroppedFile) => canUploadDroppedImage(file) || canSendDroppedFileAsPath(file),
    [canSendDroppedFileAsPath, canUploadDroppedImage]
  );

  // Stable identities: these are pure functions of their argument, so memoising
  // with an empty dep list keeps the useCallback at the bottom of this component
  // from re-creating on every render.
  const droppedFilePath = useCallback((file: DroppedFile) => file.sourcePath || file.path, []);
  const droppedImageAttachmentPath = useCallback(
    (file: DroppedFile) => file.stagedPath || file.path,
    []
  );

  const handleRemoveDroppedFile = (idToRemove: string) => {
    // Remove from local dropped files
    setLocalDroppedFiles((prev) => prev.filter((file) => file.id !== idToRemove));

    // If it's from parent, call the parent's callback
    if (onFilesProcessed && droppedFiles.some((file) => file.id === idToRemove)) {
      onFilesProcessed();
    }
  };

  const handleRemovePastedImage = (idToRemove: string) => {
    const imageToRemove = pastedImages.find((img) => img.id === idToRemove);
    if (imageToRemove?.filePath) {
      window.electron.deleteTempFile(imageToRemove.filePath);
    }
    setPastedImages((currentImages) => currentImages.filter((img) => img.id !== idToRemove));
  };

  const handleRetryImageSave = async (imageId: string) => {
    const imageToRetry = pastedImages.find((img) => img.id === imageId);
    if (!imageToRetry || !imageToRetry.dataUrl) return;

    // Set the image to loading state
    setPastedImages((prev) =>
      prev.map((img) => (img.id === imageId ? { ...img, isLoading: true, error: undefined } : img))
    );

    try {
      const result = await window.electron.saveDataUrlToTemp(imageToRetry.dataUrl, imageId);
      setPastedImages((prev) =>
        prev.map((img) =>
          img.id === result.id
            ? { ...img, filePath: result.filePath, error: result.error, isLoading: false }
            : img
        )
      );
    } catch (err) {
      console.error('Error retrying image save:', err);
      setPastedImages((prev) =>
        prev.map((img) =>
          img.id === imageId
            ? { ...img, error: 'Failed to save image via Electron.', isLoading: false }
            : img
        )
      );
    }
  };

  const autoFocusAtMountRef = useRef(autoFocus);
  useEffect(() => {
    if (autoFocusAtMountRef.current && textAreaRef.current) {
      textAreaRef.current.focus();
    }
  }, []);

  // Load model limits from the API
  const getModelLimits = async () => {
    try {
      const response = await read('model-limits', false);
      if (response) {
        // The response is already parsed, no need for JSON.parse
        return response as ModelLimit[];
      }
    } catch (err) {
      console.error('Error fetching model limits:', err);
    }
    return [];
  };

  // Helper function to find model limit using pattern matching
  const findModelLimit = (modelName: string, modelLimits: ModelLimit[]): number | null => {
    if (!modelName) return null;
    const matchingLimit = modelLimits.find((limit) =>
      modelName.toLowerCase().includes(limit.pattern.toLowerCase())
    );
    return matchingLimit ? matchingLimit.context_limit : null;
  };

  // Load providers and get current model's token limit
  const loadProviderDetails = async () => {
    try {
      // Reset token limit loaded state
      setIsTokenLimitLoaded(false);

      // Get current model and provider first to avoid unnecessary provider fetches
      //
      // Issue #56 Gate B: THIS CHAT's binding wins over the app's global
      // selection. A private chat pinned to Versa was measuring its usage
      // against whatever public model the app happened to be pointed at —
      // measured at "969.9k of 1M" for a turn that ran on a different model
      // entirely. When nothing is pinned, or the pin names what is already
      // selected, this resolves to exactly what it always did.
      const selected = await getCurrentModelAndProvider();
      const model = effectiveModel?.model ?? selected.model;
      const provider = effectiveModel?.provider ?? selected.provider;
      if (!model || !provider) {
        // No model is bound, so there is no context window to report. Leaving
        // the 128k default in place while announcing it as LOADED is what made
        // the onboarding gauge read "128k of 128k tokens remaining" beside a
        // chip correctly saying no model was chosen — a precise figure for a
        // model that does not exist. Zero is what the gauge renders its empty
        // state from.
        setTokenLimit(0);
        setIsTokenLimitLoaded(false);
        return;
      }

      // Llama Server (local models): the real context window is a live property
      // of the loaded model, read from the running server's /props. Prefer it
      // over any static catalog/default so the gauge matches the CLI/backend
      // (e.g. a 262k model instead of the 128k fallback). Only when the sidecar
      // has reported a window; otherwise fall through to the static logic.
      if (provider === 'llamacpp') {
        try {
          const status = await llamacppStatus();
          const ctx = status.data?.sidecar?.context_size;
          if (typeof ctx === 'number' && ctx > 0) {
            setTokenLimit(ctx);
            setIsTokenLimitLoaded(true);
            return;
          }
        } catch (e) {
          console.warn('Failed to read llama-server context window, using fallback:', e);
        }
      }

      // First, check predefined models from environment (highest priority)
      const predefinedModels = getPredefinedModelsFromEnv();
      const predefinedModel = predefinedModels.find((m) => m.name === model);
      if (predefinedModel?.context_limit) {
        setTokenLimit(predefinedModel.context_limit);
        setIsTokenLimitLoaded(true);
        return;
      }

      const providers = await getProviders(true);

      // Find the provider details for the current provider
      const currentProvider = providers.find((p) => p.name === provider);
      if (currentProvider?.metadata?.known_models) {
        // Find the model's token limit from the backend response
        const modelConfig = currentProvider.metadata.known_models.find((m) => m.name === model);
        if (modelConfig?.context_limit) {
          setTokenLimit(modelConfig.context_limit);
          setIsTokenLimitLoaded(true);
          return;
        }
      }

      // Fallback: Use pattern matching logic if no exact model match was found
      const modelLimit = await getModelLimits();
      const fallbackLimit = findModelLimit(model as string, modelLimit);
      if (fallbackLimit !== null) {
        setTokenLimit(fallbackLimit);
        setIsTokenLimitLoaded(true);
        return;
      }

      // If no match found, use the default model limit
      setTokenLimit(TOKEN_LIMIT_DEFAULT);
      setIsTokenLimitLoaded(true);
    } catch (err) {
      console.error('Error loading providers or token limit:', err);
      // Set default limit on error
      setTokenLimit(TOKEN_LIMIT_DEFAULT);
      setIsTokenLimitLoaded(true);
    }
  };

  // Initial load and refresh when model changes — including when a turn reports
  // that this chat is pinned to a different one (issue #56 Gate B).
  useEffect(() => {
    loadProviderDetails();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentModel, currentProvider, effectiveModel?.provider, effectiveModel?.model]);

  // Cleanup effect for component unmount - prevent memory leaks
  useEffect(() => {
    return () => {
      // Clear any pending timeouts from image processing
      //
      // A composer with a draft key does NOT delete its staged images on the
      // way out: they are in its draft (saved as they were staged) and the
      // composer that shows that key next needs the files. The draft's owner
      // deletes them when the tab is gone (`retainTabComposerDrafts`).
      if (!draftKeyRef.current) {
        setPastedImages((currentImages) => {
          currentImages.forEach((img) => {
            if (img.filePath) {
              try {
                window.electron.deleteTempFile(img.filePath);
              } catch (error) {
                console.error('Error deleting temp file:', error);
              }
            }
          });
          return [];
        });
      }

      // Clear all tracked timeouts
      // eslint-disable-next-line react-hooks/exhaustive-deps
      const timeouts = timeoutRefsRef.current;
      timeouts.forEach((timeoutId) => {
        window.clearTimeout(timeoutId);
      });
      timeouts.clear();
    };
  }, []);

  // Ten lines, counted in the line box the composer actually renders:
  // `COMPOSER_INPUT_TYPE_CLASS` is `--text-body`, which is 14 on a 20px line.
  // This tracked a 24px line while the input was briefly 16px; left at 24 it
  // would now mean twelve lines, which is how this figure was wrong before.
  // The textarea's own vertical padding is inside the measurement
  // (`scrollHeight` includes padding), so the true ceiling is a shade under ten
  // — the right way to be wrong for a scroll cap.
  const maxHeight = 10 * 20;

  // Immediate function to update actual value - no debounce for better responsiveness
  const updateValue = React.useCallback((value: string) => {
    setValue(value);
  }, []);

  const debouncedAutosize = useMemo(
    () =>
      debounce((element: HTMLTextAreaElement) => {
        element.style.height = '0px'; // Reset height
        const scrollHeight = element.scrollHeight;
        element.style.height = Math.min(scrollHeight, maxHeight) + 'px';
      }, 50),
    [maxHeight]
  );

  useEffect(() => {
    if (textAreaRef.current) {
      debouncedAutosize(textAreaRef.current);
    }
  }, [debouncedAutosize, displayValue]);

  // Issue #65 — the composer's two views of one string.
  //
  // `displayValue` stays the whole message, reference tags included, because
  // every other seam in this component already carries it: draft save/restore,
  // the `?prompt=` deep link, history navigation, the queue, steering, submit.
  // Holding references in their own state would mean teaching each of those
  // about them, and each one missed is a reference the user attached and the
  // agent never sees.
  //
  // What the *textarea* binds to is the body — the message with the tags taken
  // out — so the user never sees ~45 characters of XML where they typed a
  // sentence. The tags come back as chips in the rail below. `composerRefs` is
  // the parse, so a chip on screen is always a reference the agent resolves.
  const { body: composerBody, refs: composerRefs } = useMemo(
    () => splitComposerText(displayValue),
    [displayValue]
  );

  const setComposerText = useCallback(
    (next: string) => {
      setDisplayValue(next);
      updateValue(next);
    },
    [updateValue]
  );

  useEffect(
    () =>
      onQuotedText(
        sessionId,
        (quote) => {
          const current = splitComposerText(displayValueRef.current);
          const next = joinComposerText(current.body, [...current.refs, quoteReference(quote)]);
          displayValueRef.current = next;
          setComposerText(next);
          requestAnimationFrame(() => textAreaRef.current?.focus());
        },
        () => textAreaRef.current
      ),
    [sessionId, setComposerText]
  );

  /** Replace the prose, keeping whatever references are attached. */
  const setComposerBody = useCallback(
    (body: string) => setComposerText(joinComposerText(body, composerRefs)),
    [composerRefs, setComposerText]
  );

  const handleRemoveReference = useCallback(
    (index: number) => {
      setComposerText(removeComposerRefAt(displayValue, index));
      textAreaRef.current?.focus();
    },
    [displayValue, setComposerText]
  );

  // Reset textarea height when the prose is empty. Keyed off the body, not the
  // whole message: a message that is nothing but a chip shows an empty box.
  useEffect(() => {
    if (textAreaRef.current && composerBody === '') {
      textAreaRef.current.style.height = 'auto';
    }
  }, [composerBody]);

  const handleChange = (evt: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = evt.target.value;
    const cursorPosition = evt.target.selectionStart;

    setComposerBody(val);
    setHasUserTyped(true);
    // The textarea's offsets are body offsets, and so is everything the mention
    // popover computes from them.
    checkForMentionOrSlash(val, cursorPosition, evt.target);
  };

  const checkForMentionOrSlash = (
    text: string,
    cursorPosition: number,
    textArea: HTMLTextAreaElement
  ) => {
    const beforeCursor = text.slice(0, cursorPosition);
    const lastAtIndex = beforeCursor.lastIndexOf('@');
    let lastSlashIndex = -1;
    for (
      let index = beforeCursor.lastIndexOf('/');
      index >= 0;
      index = beforeCursor.lastIndexOf('/', index - 1)
    ) {
      if (index === 0 || /\s/.test(beforeCursor[index - 1])) {
        lastSlashIndex = index;
        break;
      }
    }
    const triggerIndex = Math.max(lastAtIndex, lastSlashIndex);

    if (triggerIndex === -1) {
      setMentionPopover((prev) => ({ ...prev, isOpen: false }));
      return;
    }

    const trigger = beforeCursor[triggerIndex];
    const query = beforeCursor.slice(triggerIndex + 1);
    if (query.includes(' ') || query.includes('\n')) {
      setMentionPopover((prev) => ({ ...prev, isOpen: false }));
      return;
    }

    // Calculate position for the popover - position it above the chat input
    const textAreaRect = textArea.getBoundingClientRect();

    setMentionPopover((prev) => ({
      ...prev,
      isOpen: true,
      position: {
        x: textAreaRect.left,
        y: textAreaRect.top, // Position at the top of the textarea
      },
      query,
      mentionStart: triggerIndex,
      selectedIndex: 0, // Reset selection when query changes
      isSlashCommand: trigger === '/',
      // filteredFiles will be populated by the MentionPopover component
    }));
  };

  const handlePaste = async (evt: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(evt.clipboardData.files || []);
    const clipboardImages = files.filter((file) => file.type.startsWith('image/'));
    const imageFiles = clipboardImages.filter((file) => currentModelAcceptsMimeType(file.type));
    const unsupportedImages = clipboardImages.filter(
      (file) => !currentModelAcceptsMimeType(file.type)
    );

    if (clipboardImages.length === 0) return;

    // If the active model does not support vision, ignore image pastes and
    // let the browser handle any plain-text content in the clipboard.
    if (!currentModelSupportsVision) return;

    if (unsupportedImages.length > 0) {
      evt.preventDefault();
      setPastedImages((prev) => [
        ...prev,
        {
          id: `error-${Date.now()}`,
          dataUrl: '',
          isLoading: false,
          error: `This model cannot accept ${unsupportedImages
            .map((file) => file.type || 'this image type')
            .join(', ')} as image input.`,
        },
      ]);

      const timeoutId = setTimeout(() => {
        setPastedImages((prev) => prev.filter((img) => !img.id.startsWith('error-')));
        timeoutRefsRef.current.delete(timeoutId);
      }, 5000);
      timeoutRefsRef.current.add(timeoutId);

      if (imageFiles.length === 0) return;
    }

    // Check if adding these images would exceed the limit
    if (pastedImages.length + imageFiles.length > MAX_IMAGES_PER_MESSAGE) {
      // Show error message to user
      setPastedImages((prev) => [
        ...prev,
        {
          id: `error-${Date.now()}`,
          dataUrl: '',
          isLoading: false,
          error: `Cannot paste ${imageFiles.length} image(s). Maximum ${MAX_IMAGES_PER_MESSAGE} images per message allowed. Currently have ${pastedImages.length}.`,
        },
      ]);

      // Remove the error message after 5 seconds with cleanup tracking
      const timeoutId = setTimeout(() => {
        setPastedImages((prev) => prev.filter((img) => !img.id.startsWith('error-')));
        timeoutRefsRef.current.delete(timeoutId);
      }, 5000);
      timeoutRefsRef.current.add(timeoutId);

      return;
    }

    evt.preventDefault();

    // Process each image file
    const newImages: PastedImage[] = [];

    for (const file of imageFiles) {
      // Check individual file size before processing
      if (file.size > MAX_IMAGE_SIZE_MB * 1024 * 1024) {
        const errorId = `error-${Date.now()}-${Math.random().toString(36).substring(2, 9)}`;
        newImages.push({
          id: errorId,
          dataUrl: '',
          isLoading: false,
          error: `Image too large (${Math.round(file.size / (1024 * 1024))}MB). Maximum ${MAX_IMAGE_SIZE_MB}MB allowed.`,
        });

        // Remove the error message after 5 seconds with cleanup tracking
        const timeoutId = setTimeout(() => {
          setPastedImages((prev) => prev.filter((img) => img.id !== errorId));
          timeoutRefsRef.current.delete(timeoutId);
        }, 5000);
        timeoutRefsRef.current.add(timeoutId);

        continue;
      }

      const imageId = `img-${Date.now()}-${Math.random().toString(36).substring(2, 9)}`;

      // Add the image with loading state
      newImages.push({
        id: imageId,
        dataUrl: '',
        isLoading: true,
      });

      // Process the image asynchronously
      const reader = new FileReader();
      reader.onload = async (e) => {
        const dataUrl = e.target?.result as string;
        if (dataUrl) {
          try {
            await validateImageDataUrl(dataUrl);
          } catch {
            setPastedImages((prev) =>
              prev.map((img) =>
                img.id === imageId
                  ? {
                      ...img,
                      dataUrl: '',
                      error: 'Image preview could not be decoded.',
                      isLoading: false,
                    }
                  : img
              )
            );
            return;
          }

          setPastedImages((prev) =>
            prev.map((img) => (img.id === imageId ? { ...img, dataUrl, isLoading: true } : img))
          );

          try {
            const result = await window.electron.saveDataUrlToTemp(dataUrl, imageId);
            setPastedImages((prev) =>
              prev.map((img) =>
                img.id === result.id
                  ? { ...img, filePath: result.filePath, error: result.error, isLoading: false }
                  : img
              )
            );
          } catch (err) {
            console.error('Error saving pasted image:', err);
            setPastedImages((prev) =>
              prev.map((img) =>
                img.id === imageId
                  ? { ...img, error: 'Failed to save image via Electron.', isLoading: false }
                  : img
              )
            );
          }
        }
      };
      reader.onerror = () => {
        console.error('Failed to read image file:', file.name);
        setPastedImages((prev) =>
          prev.map((img) =>
            img.id === imageId
              ? { ...img, error: 'Failed to read image file.', isLoading: false }
              : img
          )
        );
      };
      reader.readAsDataURL(file);
    }

    // Add all new images to the existing list
    setPastedImages((prev) => [...prev, ...newImages]);
  };

  // Cleanup debounced functions on unmount
  useEffect(() => {
    return () => {
      debouncedAutosize.cancel?.();
    };
  }, [debouncedAutosize]);

  // Handlers for composition events, which are crucial for proper IME behavior
  const handleCompositionStart = () => {
    setIsComposing(true);
  };

  const handleCompositionEnd = () => {
    setIsComposing(false);
  };

  const handleHistoryNavigation = (evt: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const isUp = evt.key === 'ArrowUp';
    const isDown = evt.key === 'ArrowDown';

    // Only handle up/down keys with Cmd/Ctrl modifier
    if ((!isUp && !isDown) || !(evt.metaKey || evt.ctrlKey) || evt.altKey || evt.shiftKey) {
      return;
    }

    // Only prevent history navigation if the user has actively typed something
    // This allows history navigation when text is populated from history or other sources
    // but prevents it when the user is actively editing text
    if (hasUserTyped && displayValue.trim() !== '') {
      return;
    }

    evt.preventDefault();

    // Get global history once to avoid multiple calls
    const globalHistory = LocalMessageStorage.getRecentMessages() || [];

    // Save current input if we're just starting to navigate history
    if (historyIndex === -1) {
      setSavedInput(displayValue || '');
      setIsInGlobalHistory(commandHistory.length === 0);
    }

    // Determine which history we're using
    const currentHistory = isInGlobalHistory ? globalHistory : commandHistory;
    let newIndex = historyIndex;
    let newValue = '';

    // Handle navigation
    if (isUp) {
      // Moving up through history
      if (newIndex < currentHistory.length - 1) {
        // Still have items in current history
        newIndex = historyIndex + 1;
        newValue = currentHistory[newIndex];
      } else if (!isInGlobalHistory && globalHistory.length > 0) {
        // Switch to global history
        setIsInGlobalHistory(true);
        newIndex = 0;
        newValue = globalHistory[newIndex];
      }
    } else {
      // Moving down through history
      if (newIndex > 0) {
        // Still have items in current history
        newIndex = historyIndex - 1;
        newValue = currentHistory[newIndex];
      } else if (isInGlobalHistory && commandHistory.length > 0) {
        // Switch to chat history
        setIsInGlobalHistory(false);
        newIndex = commandHistory.length - 1;
        newValue = commandHistory[newIndex];
      } else {
        // Return to original input
        newIndex = -1;
        newValue = savedInput;
      }
    }

    // Update display if we have a new value
    if (newIndex !== historyIndex) {
      setHistoryIndex(newIndex);
      if (newIndex === -1) {
        setDisplayValue(savedInput || '');
        setValue(savedInput || '');
      } else {
        setDisplayValue(newValue || '');
        setValue(newValue || '');
      }
      // Reset hasUserTyped when we populate from history
      setHasUserTyped(false);
    }
  };

  // Helper function to handle interruption and queue logic when loading
  // Every path that stops a running turn goes through `stopAck.trigger` rather
  // than calling `onStop` directly, so the hard interrupt is confirmed the same
  // way no matter which control fired it (the Stop button, "Stop and Send" on a
  // queued message, or a typed interruption phrase).
  const stopAck = useStopAcknowledgement(onStop);

  const handleInterruptionAndQueue = () => {
    if (!isLoading || !hasSubmittableContent) {
      return false;
    }

    const imageAttachments: UserAttachment[] = currentModelSupportsVision
      ? [
          ...pastedImages
            .filter((img) => img.filePath && !img.error && !img.isLoading)
            .map((img) => ({ path: img.filePath as string, kind: 'image' as const })),
          ...allDroppedFiles
            .filter(canUploadDroppedImage)
            .map((file) => ({ path: droppedImageAttachmentPath(file), kind: 'image' as const })),
        ]
      : [];
    const ownedTempAttachmentPaths = currentModelSupportsVision
      ? [
          ...pastedImages
            .filter((img) => img.filePath && !img.error && !img.isLoading)
            .map((img) => img.filePath as string),
          ...allDroppedFiles
            .filter(canUploadDroppedImage)
            .flatMap((file) => (file.stagedPath ? [file.stagedPath] : [])),
        ]
      : [];
    const droppedFilePaths = allDroppedFiles.filter(canSendDroppedFileAsPath).map(droppedFilePath);

    let contentToQueue = displayValue.trim();
    if (droppedFilePaths.length > 0) {
      const pathsString = droppedFilePaths.join(' ');
      contentToQueue = contentToQueue ? `${contentToQueue} ${pathsString}` : pathsString;
    }

    // The prose again, for the same reason as the /diverge check: "stop" is an
    // interruption whether or not the user also left a skill attached, and the
    // detector's short-input branch would not see it past 45 characters of tag.
    const interruptionMatch = detectInterruption(composerBody.trim());

    if (interruptionMatch && interruptionMatch.shouldInterrupt) {
      setLastInterruption(interruptionMatch.matchedText);
      void stopAck.trigger(true);
      queuePausedRef.current = true;

      // For interruptions, we need to queue the message to be sent after the stop completes
      // rather than trying to send it immediately while the system is still loading
      const interruptionMessage = {
        id: Date.now().toString() + Math.random().toString(36).substr(2, 9),
        content: contentToQueue,
        attachments: imageAttachments,
        ownedTempAttachmentPaths,
        timestamp: Date.now(),
      };

      // Add the interruption message to the front of the queue so it gets sent first
      setQueuedMessages((prev) => [interruptionMessage, ...prev]);

      setDisplayValue('');
      setValue('');
      setPastedImages([]);
      if (onFilesProcessed && droppedFiles.length > 0) {
        onFilesProcessed();
      }
      if (localDroppedFiles.length > 0) {
        setLocalDroppedFiles([]);
      }
      return true;
    }

    const newMessage = {
      id: Date.now().toString() + Math.random().toString(36).substr(2, 9),
      content: contentToQueue,
      attachments: imageAttachments,
      ownedTempAttachmentPaths,
      timestamp: Date.now(),
    };
    setQueuedMessages((prev) => {
      const newQueue = [...prev, newMessage];
      // If adding to an empty queue, reset the paused state
      if (prev.length === 0) {
        queuePausedRef.current = false;
        setLastInterruption(null);
      }
      return newQueue;
    });
    setDisplayValue('');
    setValue('');
    setPastedImages([]);
    if (onFilesProcessed && droppedFiles.length > 0) {
      onFilesProcessed();
    }
    if (localDroppedFiles.length > 0) {
      setLocalDroppedFiles([]);
    }
    return true;
  };

  // --- BR-61: soft interrupt ("steer") ---------------------------------------
  // Hand a message to the turn that is *already running* instead of queueing it
  // until the turn ends (the default) or stopping the agent outright: the server
  // queues it on the agent, which injects it at its next loop boundary, so no
  // in-flight tool work is thrown away. Text only — a soft interrupt has no
  // attachment channel, so anything with images/files takes the normal path.

  // A fresh view of `isLoading` for callbacks that resolve after an await (the
  // value closed over at click time is stale by the time the POST answers).
  const isLoadingRef = useRef(isLoading);
  useEffect(() => {
    isLoadingRef.current = isLoading;
  }, [isLoading]);

  /**
   * Is a steer on the table at all — is a turn running with a steer path bound?
   *
   * Kept separate from {@link canSteer} so `MessageQueue` still learns that a
   * turn IS running on a surface that cannot steer it: it needs that to know
   * whether its SD-8 note has anything to explain. An undefined
   * `onSteerMessage` means both "no turn" and "cannot steer" otherwise, and the
   * note would appear over an idle agent.
   */
  const steerApplies = Boolean(onSteer) && isLoading;

  /**
   * May THIS surface actually steer? (SD-8.)
   *
   * `steerUnavailableReason()` is non-null exactly on a browser-served session,
   * where `userActionHeaders()` has no `X-User-Action` to send and the daemon's
   * `steer_refusal` admits nothing else — so the POST is refused on every
   * daemon, every time. Measured on a real `biorouter serve` 2026-09-12: both
   * clicks of "Add now" posted `/interrupt` and both took a 403 whose body
   * nothing rendered.
   *
   * Gating here rather than on the refusal is what keeps the Cmd/Ctrl+Enter
   * chord and its tooltip line honest too: a shortcut advertised as "adds it to
   * the running turn" that instead queues the message is the same silent lie in
   * a second place.
   */
  const canSteer = steerApplies && !steerUnavailableReason();

  // Never drop the user's words: if the steer was refused (the turn ended in the
  // meantime) send the text now, or re-queue it if a turn is somehow running.
  const sendOrQueueText = useCallback(
    (content: string) => {
      const message = {
        id: Date.now().toString() + Math.random().toString(36).substr(2, 9),
        content,
        attachments: [],
        timestamp: Date.now(),
      };
      if (queueDisposedRef.current) {
        // The steer was answered after this composer went away.
        handBackQueuedMessage(queueKeyRef.current, queueDraftKeyRef.current, message);
        return;
      }
      if (isLoadingRef.current) {
        setQueuedMessages((prev) => [...prev, message]);
        return;
      }
      // Via the queue-aware send, so that "never drop the user's words" also
      // survives a submit that REFUSES the text: it lands back in the queue
      // rather than nowhere.
      offerQueuedMessage(message);
    },
    [offerQueuedMessage]
  );

  const steerText = useCallback(
    (content: string, putBack?: () => boolean) => {
      if (!onSteer) return;
      void onSteer(content).then((accepted) => {
        if (accepted) {
          // The agent echoes the steer back as a user message on the live stream
          // once it consumes it, so nothing is appended to the transcript here.
          LocalMessageStorage.addMessage(content);
        } else if (!putBack?.()) {
          sendOrQueueText(content);
        }
      });
    },
    [onSteer, sendOrQueueText]
  );

  /** Cmd/Ctrl+Enter while a turn runs: steer with whatever is in the composer. */
  const handleSteerFromComposer = useCallback((): boolean => {
    if (!canSteer) return false;
    const text = displayValue.trim();
    if (!text || pastedImages.length > 0 || allDroppedFiles.length > 0) {
      return false;
    }
    steerText(text);
    setDisplayValue('');
    setValue('');
    return true;
  }, [canSteer, displayValue, pastedImages.length, allDroppedFiles.length, steerText]);

  /** BR-61: send a queued message into the running turn without stopping it. */
  const handleSteerMessage = (messageId: string) => {
    const index = queuedMessages.findIndex((msg) => msg.id === messageId);
    const messageToSteer = queuedMessages[index];
    if (!messageToSteer || !canSteer) return;

    setQueuedMessages((prev) => prev.filter((msg) => msg.id !== messageId));
    // D5: a steer the daemon refuses while a turn is still running goes back
    // where it was in the queue, not to the back of it — "Add now" on the
    // first of three rows must not quietly make it the last.
    steerText(messageToSteer.content, () => {
      if (!isLoadingRef.current) return false;
      setQueuedMessages((prev) => {
        if (prev.some((msg) => msg.id === messageToSteer.id)) return prev;
        const next = [...prev];
        next.splice(Math.min(index, next.length), 0, messageToSteer);
        return next;
      });
      return true;
    });
  };

  /**
   * Cmd/Ctrl+Enter with an EMPTY composer: steer the front of the queue — the
   * keyboard equivalent of its "Add now" button.
   *
   * The composer must be empty of everything, not merely of text. A composer
   * holding images or dropped files cannot steer (a soft interrupt has no
   * attachment channel), and quietly sending a QUEUED message instead would be
   * the chord doing something the user did not ask for. It does nothing there.
   *
   * Eligibility is the button's own: `canSteer`, `canSteerMessage`, and not
   * mid-edit — the row's button is disabled while its editor is open, and the
   * user can click into the composer with that editor still open, so without
   * this the chord would steer the row out from under the edit in progress.
   */
  const handleSteerNextQueuedMessage = (): boolean => {
    if (!canSteer) return false;
    if (displayValue.trim() || pastedImages.length > 0 || allDroppedFiles.length > 0) {
      return false;
    }
    const next = queuedMessages[0];
    if (!next || !canSteerMessage(next) || editingMessageIdRef.current === next.id) {
      return false;
    }
    handleSteerMessage(next.id);
    return true;
  };

  /**
   * Nothing to send TO. Reachable since "Explore Biorouter first →" — the app
   * renders with no provider bound, and the composer owes the user the reason
   * at the moment they try to send rather than the daemon's own error toast a
   * round trip later.
   */
  const noModelConfigured = hasNoModelConfigured(
    modelConfigStatus,
    effectiveModel?.provider ?? currentProvider
  );

  // Why a chat whose Crew access lapsed cannot send, for Enter to say instead of doing nothing.
  const crewHold = useCrewComposerHold(sessionId);

  const hasCrewCommandExtras =
    composerRefs.length > 0 || pastedImages.length > 0 || allDroppedFiles.length > 0;
  const isCrewNavigationCommand = splitComposerText(displayValue).body.trim() === '/crew';
  const openCrew = useCallback(() => {
    if (hasCrewCommandExtras) {
      toastWarning({
        title: 'Draft kept',
        msg: 'Remove the attached files, images, and reference chips before using /crew, or open Crew from the sidebar. Nothing was sent.',
      });
      return;
    }
    // A chat with no session yet (Home's composer, a new chat before its first send) has nothing
    // Crew could connect: navigating anyway dropped the chat and showed no connect offer, while
    // the Access tab's own instruction says to type /crew in the chat. Say what to do instead and
    // keep the draft, as /diverge does.
    if (!sessionId) {
      toastWarning({
        title: 'Start the chat first',
        msg: 'Send this chat a message, then type /crew to connect it to a Crew channel. To just open Crew, use the sidebar.',
      });
      setMentionPopover((prev) => ({ ...prev, isOpen: false }));
      return;
    }
    displayValueRef.current = '';
    setView('crew', { resumeSessionId: sessionId });
    setDisplayValue('');
    setValue('');
    setHasUserTyped(false);
    setMentionPopover((prev) => ({ ...prev, isOpen: false }));
  }, [hasCrewCommandExtras, sessionId, setView, setValue]);

  const canSubmit =
    !isLoading &&
    !noModelConfigured &&
    (displayValue.trim() ||
      (currentModelSupportsVision &&
        pastedImages.some((img) => img.filePath && !img.error && !img.isLoading)) ||
      allDroppedFiles.some(canSendDroppedFile));

  const performSubmit = useCallback(
    (text?: string) => {
      const validPastedImages = pastedImages.filter(
        (img) => img.filePath && !img.error && !img.isLoading
      );
      const validDroppedImages = allDroppedFiles.filter(canUploadDroppedImage);
      const validDroppedFiles = allDroppedFiles.filter(canSendDroppedFileAsPath);

      // Build structured image attachments (sent as content blocks, not path tokens)
      const imageAttachments: UserAttachment[] = [
        ...(currentModelSupportsVision
          ? validPastedImages.map((img) => ({
              path: img.filePath as string,
              kind: 'image' as const,
            }))
          : []),
        ...validDroppedImages.map((file) => ({
          path: droppedImageAttachmentPath(file),
          kind: 'image' as const,
        })),
      ];

      // Files that cannot or should not be uploaded still go into the text as paths.
      const nonImageFilePaths = validDroppedFiles.map(droppedFilePath);

      // Intercept the client-side /diverge command before it becomes a message.
      // It branches the current conversation instead of being sent to the agent.
      //
      // Matched against the prose, not the whole message: a reference is drawn
      // as a chip and is invisible in the textarea, so comparing the raw text
      // would let an attached chip silently defeat a command that is — to the
      // user, correctly — the only thing in the box.
      const trimmedCandidate = splitComposerText(text ?? displayValue).body.trim();
      if (trimmedCandidate === '/crew') {
        openCrew();
        return;
      }
      if (trimmedCandidate === DIVERGE_TRIGGER) {
        if (!sessionId) {
          toastWarning({
            title: 'Start a chat first',
            msg: '/diverge continues an existing conversation in a new chat. Send a message before using it.',
          });
          return;
        }
        void diverge(sessionId);
        setDisplayValue('');
        setValue('');
        setHasUserTyped(false);
        return;
      }

      let textToSend = text ?? displayValue.trim();

      // Append non-image file paths to the text prompt
      if (nonImageFilePaths.length > 0) {
        const pathsString = nonImageFilePaths.join(' ');
        textToSend = textToSend ? `${textToSend} ${pathsString}` : pathsString;
      }

      if (textToSend || imageAttachments.length > 0) {
        if (displayValue.trim()) {
          LocalMessageStorage.addMessage(displayValue);
        } else if (nonImageFilePaths.length > 0) {
          LocalMessageStorage.addMessage(nonImageFilePaths.join(' '));
        }

        const submitted = handleSubmit(
          new CustomEvent('submit', {
            detail: { value: textToSend, attachments: imageAttachments },
          }) as unknown as React.FormEvent
        );

        // The composer wipes itself a few lines below, synchronously, before the
        // submit has said whether it took the message. When it did NOT — a
        // re-entrant send, a chat start that failed or was refused, a controller
        // with no session — the message is handed back. What goes back is the
        // box the person was looking at: `displayValue` (reference chips
        // included, dropped-file paths appended for the send not), the staged
        // images and the dropped files.
        //
        // HOW it goes back is the part that was wrong. It was a window-wide
        // `restore-chat-input` addressed by chat id, and a new chat has none, so
        // every new tab's composer in every pane took it — replacing what they
        // held (1.90.4, measured). Now a new chat's composer hands it to its own
        // TAB's draft, which reaches this composer or the one that replaces it
        // and nothing else; any other composer hands it back to itself.
        const unsent: ComposerDraft = {
          text: displayValue,
          images: draftImagesOf(pastedImages),
          files: draftFilesOf(allDroppedFiles),
        };
        // Captured at the send: the message belongs to the key it was typed
        // under, whatever this composer has become when the answer arrives —
        // even unmounted, because the person went to Settings in the meantime.
        // The key is marked sending until then, which keeps its tab.
        const inFlight = draftKey ? beginComposerSend(draftKey) : null;
        void Promise.resolve(submitted).then(
          (accepted) => {
            if (accepted !== false) inFlight?.settle();
            else if (inFlight) inFlight.giveBack(unsent);
            else takeBack(unsent);
          },
          (error: unknown) => {
            inFlight?.settle();
            throw error;
          }
        );

        // Auto-resume queue after sending a NON-interruption message (if it was paused due to interruption)
        if (
          queuePausedRef.current &&
          lastInterruption &&
          textToSend &&
          !detectInterruption(textToSend)
        ) {
          queuePausedRef.current = false;
          setLastInterruption(null);
        }

        setDisplayValue('');
        setValue('');
        setPastedImages([]);
        setHistoryIndex(-1);
        setSavedInput('');
        setIsInGlobalHistory(false);
        setHasUserTyped(false);

        // Clear both parent and local dropped files after processing
        if (onFilesProcessed && droppedFiles.length > 0) {
          onFilesProcessed();
        }
        if (localDroppedFiles.length > 0) {
          setLocalDroppedFiles([]);
        }

        // The message now belongs to the send, and `beginComposerSend` above
        // has already emptied the draft. The refs are cleared with the state,
        // not at the next render, because this composer can be unmounted before
        // that render and saves from them.
        displayValueRef.current = '';
        pastedImagesRef.current = [];
        localDroppedFilesRef.current = [];
        parentDroppedFilesRef.current = [];
        if (draftKey) draftVersionRef.current = composerDraftVersion(draftKey);
      }
    },
    [
      allDroppedFiles,
      canSendDroppedFileAsPath,
      canUploadDroppedImage,
      currentModelSupportsVision,
      displayValue,
      diverge,
      draftKey,
      droppedFilePath,
      droppedImageAttachmentPath,
      droppedFiles.length,
      handleSubmit,
      lastInterruption,
      localDroppedFiles.length,
      onFilesProcessed,
      pastedImages,
      sessionId,
      openCrew,
      setLocalDroppedFiles,
      takeBack,
    ]
  );

  const handleKeyDown = (evt: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Navigation does not submit content, start a model, or grant access.
    if (
      evt.key === 'Enter' &&
      !evt.shiftKey &&
      !evt.altKey &&
      !isComposing &&
      isCrewNavigationCommand
    ) {
      evt.preventDefault();
      openCrew();
      return;
    }
    // If mention popover is open, handle arrow keys and enter
    if (mentionPopover.isOpen && mentionPopoverRef.current) {
      if (evt.key === 'ArrowDown') {
        evt.preventDefault();
        const displayFiles = mentionPopoverRef.current.getDisplayFiles();
        const maxIndex = Math.max(0, displayFiles.length - 1);
        setMentionPopover((prev) => ({
          ...prev,
          selectedIndex: Math.min(prev.selectedIndex + 1, maxIndex),
        }));
        return;
      }
      if (evt.key === 'ArrowUp') {
        evt.preventDefault();
        setMentionPopover((prev) => ({
          ...prev,
          selectedIndex: Math.max(prev.selectedIndex - 1, 0),
        }));
        return;
      }
      if (evt.key === 'Enter') {
        evt.preventDefault();
        mentionPopoverRef.current.selectFile(mentionPopover.selectedIndex);
        return;
      }
      if (evt.key === 'Tab') {
        const displayFiles = mentionPopoverRef.current.getDisplayFiles();
        if (displayFiles.length > 0) {
          evt.preventDefault();
          mentionPopoverRef.current.selectFile(
            displayFiles.length === 1 ? 0 : mentionPopover.selectedIndex
          );
          return;
        }
      }
      if (evt.key === 'Escape') {
        evt.preventDefault();
        setMentionPopover((prev) => ({ ...prev, isOpen: false }));
        return;
      }
    }

    // Handle history navigation first
    handleHistoryNavigation(evt);

    if (evt.key === 'Enter') {
      // should not trigger submit on Enter if it's composing (IME input in progress) or shift/alt(option) is pressed
      if (evt.shiftKey || isComposing) {
        // Allow line break for Shift+Enter, or during IME composition
        return;
      }

      if (evt.altKey) {
        // The newline belongs to the prose, not after the reference block.
        setComposerBody(composerBody + '\n');
        return;
      }

      evt.preventDefault();

      // BR-61: Cmd/Ctrl+Enter while a turn is running steers it — the message
      // reaches the model on its next step instead of waiting for the turn to end.
      if (evt.metaKey || evt.ctrlKey) {
        if (handleSteerFromComposer()) {
          return;
        }
        // ...and with an EMPTY composer it steers the front of the queue: the
        // keyboard equivalent of the queue's own "Add now" button. One meaning
        // for one key — "send what I have into the running turn" — rather than
        // a second meaning that would depend on invisible state.
        if (handleSteerNextQueuedMessage()) {
          return;
        }
      }

      // Handle interruption and queue logic
      if (handleInterruptionAndQueue()) {
        return;
      }

      if (canSubmit && !submissionBlocked) {
        performSubmit();
      } else if (canSubmit && submissionBlocked && crewHold) {
        toastWarning({ title: crewHold.title, msg: crewHold.message });
      }
    }
  };

  // THE FORM SEND SUBMITS, named for THIS composer. Send sits outside the form
  // (it is the input line's sibling, not its child) and reaches it through the
  // `form` attribute, which the document resolves with an id lookup — and a
  // lookup for an id two elements share returns the FIRST. The id used to be the
  // literal `bior-chat-form`, so in a split every pane's Send submitted the LEFT
  // pane's form: measured on 1.90.4, the right-hand Send sent the left pane's
  // draft and never its own message. `useId` is unique within the window's one
  // React root, which is every composer a Send could be confused with.
  // `ChatInput.splitPaneSend.test.tsx`.
  const composerFormId = React.useId();

  const onFormSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (isCrewNavigationCommand) {
      openCrew();
      return;
    }
    if (isLoading && hasSubmittableContent) {
      handleInterruptionAndQueue();
      return;
    }
    const canSubmit =
      !isLoading &&
      !submissionBlocked &&
      (displayValue.trim() ||
        (currentModelSupportsVision &&
          pastedImages.some((img) => img.filePath && !img.error && !img.isLoading)) ||
        allDroppedFiles.some(canSendDroppedFile));
    if (canSubmit) {
      performSubmit();
    }
  };

  const handleMentionItemSelect = (itemText: string) => {
    const beforeMention = composerBody.slice(0, mentionPopover.mentionStart);
    const afterMention = composerBody.slice(
      mentionPopover.mentionStart + 1 + mentionPopover.query.length
    );

    if (`${beforeMention}${itemText}${afterMention}`.trim() === '/crew') {
      openCrew();
      return;
    }

    // A picked resource is a reference, not prose: it goes to the chip rail and
    // the `@query` it replaced just disappears. Detected by running the inserted
    // text back through the real parser rather than by asking the popover what
    // kind of item it was — the parser is the same one the agent uses, so the
    // composer cannot draw a chip for something the agent would ignore.
    const inserted = findRefTags(itemText);
    const isReference = inserted.length === 1 && inserted[0].raw === itemText.trim();

    const nextBody = `${beforeMention}${isReference ? '' : itemText}${afterMention}`;
    const nextText = joinComposerText(nextBody, composerRefs);
    setComposerText(
      isReference
        ? appendComposerRef(nextText, inserted[0].kind, inserted[0].value, inserted[0].label)
        : nextText
    );
    setMentionPopover((prev) => ({ ...prev, isOpen: false }));
    textAreaRef.current?.focus();

    // Set cursor position after the inserted file path
    setTimeout(() => {
      if (textAreaRef.current) {
        const newCursorPosition = beforeMention.length + (isReference ? 0 : itemText.length);
        textAreaRef.current.setSelectionRange(newCursorPosition, newCursorPosition);
      }
    }, 0);
  };

  const hasSubmittableContent =
    displayValue.trim() ||
    (currentModelSupportsVision &&
      pastedImages.some((img) => img.filePath && !img.error && !img.isLoading)) ||
    allDroppedFiles.some(canSendDroppedFile);
  const isAnyImageLoading = pastedImages.some((img) => img.isLoading);
  const isAnyDroppedFileLoading = allDroppedFiles.some((file) => file.isLoading);

  const hasPastedImageAttachments = pastedImages.some(
    (img) => img.filePath && !img.error && !img.isLoading
  );

  const visionMismatch = !currentModelSupportsVision && hasPastedImageAttachments;

  const isSubmitButtonDisabled =
    !isCrewNavigationCommand &&
    (noModelConfigured ||
      !hasSubmittableContent ||
      isAnyImageLoading ||
      isAnyDroppedFileLoading ||
      chatState === ChatState.RestartingAgent ||
      submissionBlocked ||
      visionMismatch);

  // Queue management functions - no storage persistence, only in-memory
  const handleRemoveQueuedMessage = (messageId: string) => {
    if (continuationQueuedMessageIdRef.current === messageId) {
      continuationQueuedMessageIdRef.current = null;
      void onAbandonContinuation?.();
    }
    const removed = queuedMessagesRef.current.find((message) => message.id === messageId);
    if (removed) deleteOwnedTempAttachments([removed]);
    setQueuedMessages((prev) => prev.filter((msg) => msg.id !== messageId));
  };

  const handleClearQueue = () => {
    if (continuationQueuedMessageIdRef.current) {
      continuationQueuedMessageIdRef.current = null;
      void onAbandonContinuation?.();
    }
    deleteOwnedTempAttachments(queuedMessagesRef.current);
    setQueuedMessages([]);
    queuePausedRef.current = false;
    setLastInterruption(null);
  };

  const handleReorderMessages = (reorderedMessages: QueuedMessage[]) => {
    setQueuedMessages(reorderedMessages);
  };

  const handleEditMessage = (messageId: string, newContent: string) => {
    setQueuedMessages((prev) =>
      prev.map((msg) => (msg.id === messageId ? { ...msg, content: newContent } : msg))
    );
  };

  const stopAndSendPendingRef = useRef(new Set<string>());
  const handleStopAndSend = (messageId: string) => {
    const messageToSend = queuedMessages.find((msg) => msg.id === messageId);
    if (!messageToSend || stopAndSendPendingRef.current.has(messageId)) return;

    // A paused queue can outlive the turn that created it. In that idle state
    // there is no exact generation to cancel (and guessing one would violate
    // the Stop barrier), so this action is simply an immediate, ordinary send.
    if (!isLoading) {
      stopAndSendPendingRef.current.add(messageId);
      setQueuedMessages((prev) => prev.filter((msg) => msg.id !== messageId));
      offerQueuedMessage(messageToSend);
      stopAndSendPendingRef.current.delete(messageId);
      return;
    }

    // Keep the row in the queue until the server confirms the previous turn's
    // slot is free. The queue remains the visible owner of the user's words.
    stopAndSendPendingRef.current.add(messageId);
    const wasPaused = queuePausedRef.current;
    queuePausedRef.current = true;
    // The pause is this control's, not the person's: a queue parked while it is
    // held goes back to the chat as it was before.
    pausedBeforeStopAndSendRef.current = wasPaused;
    // Own the future lease before asking for it. Removal, Clear, and unmount
    // can now revoke this exact queued replacement while the cancel barrier is
    // still on the wire; the delayed acknowledgement below observes that
    // revocation and abandons the lease instead of orphaning it Live.
    continuationQueuedMessageIdRef.current = messageId;

    void stopAck.trigger(true).then((stopped) => {
      stopAndSendPendingRef.current.delete(messageId);
      pausedBeforeStopAndSendRef.current = null;
      if (!stopped) {
        if (continuationQueuedMessageIdRef.current === messageId) {
          continuationQueuedMessageIdRef.current = null;
        }
        queuePausedRef.current = wasPaused;
        toastWarning({
          title: 'Message still queued',
          msg: 'Biorouter could not confirm that the current turn stopped. Your message is still in the queue.',
        });
        return;
      }

      if (continuationQueuedMessageIdRef.current !== messageId) {
        void onAbandonContinuation?.();
        queuePausedRef.current = wasPaused;
        return;
      }
      setQueuedMessages((prev) => prev.filter((msg) => msg.id !== messageId));
      offerQueuedMessage(messageToSend);
      queuePausedRef.current = wasPaused;
    });
  };

  const handleResumeQueue = () => {
    queuePausedRef.current = false;
    setLastInterruption(null);
    if (!isLoading && queuedMessages.length > 0) {
      const nextMessage = queuedMessages[0];
      offerQueuedMessage(nextMessage);
      setQueuedMessages((prev) => {
        const newQueue = prev.filter((msg) => msg.id !== nextMessage.id);
        // If queue becomes empty after processing, clear the paused state
        if (newQueue.length === 0) {
          queuePausedRef.current = false;
          setLastInterruption(null);
        }
        return newQueue;
      });
    }
  };

  // #44: the working dir is choosable only while the chat is completely empty
  // (pre-session #39 path included); the first message locks it for the
  // session's lifetime. Prefer the authoritative lock from BaseChat (hydration-
  // and failed-submit-aware); fall back to the transcript length for callers
  // that do not track session metadata.
  const workingDirIsLocked = workingDirLocked ?? messagesLength > 0;

  const compactNow = () => {
    handleSubmit(
      new CustomEvent('submit', {
        detail: { value: MANUAL_COMPACT_TRIGGER },
      }) as unknown as React.FormEvent
    );
  };

  /** A file picked from the `+` menu goes down exactly the path a drop does. */
  const attachPickedFiles = (files: FileList) => {
    void handleLocalDrop({
      preventDefault: () => {},
      stopPropagation: () => {},
      dataTransfer: { files, getData: () => '' },
    } as unknown as React.DragEvent<HTMLDivElement>);
    textAreaRef.current?.focus();
  };

  /** `@` or `/` at the caret, with the menu it opens, as if typed. */
  const insertTrigger = (trigger: ComposerTrigger) => {
    const textArea = textAreaRef.current;
    const start = textArea?.selectionStart ?? composerBody.length;
    const end = textArea?.selectionEnd ?? start;
    const before = composerBody.slice(0, start);
    // A command only opens at the start of a word.
    const insert = `${before && !/\s$/.test(before) ? ' ' : ''}${trigger}`;
    const nextBody = `${before}${insert}${composerBody.slice(end)}`;
    const caret = before.length + insert.length;
    setComposerBody(nextBody);
    setHasUserTyped(true);
    setTimeout(() => {
      const current = textAreaRef.current;
      if (!current) return;
      current.focus();
      current.setSelectionRange(caret, caret);
      checkForMentionOrSlash(nextBody, caret, current);
    }, 0);
  };

  /**
   * "Switch model" in the vision notice opens the model chip's own menu, so the
   * fix is one choice away and nothing navigates.
   */
  const modelSlotRef = useRef<HTMLDivElement>(null);
  const openModelMenu = () => {
    const trigger = modelSlotRef.current?.querySelector<HTMLButtonElement>('button[aria-haspopup]');
    if (!trigger) return;
    trigger.focus();
    trigger.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  };

  // The one place Send and Stop swap. Stop shows only for a running turn; a
  // chat that is still being created or loaded shows Send with a spinner.
  const isStarting = chatState === ChatState.LoadingConversation && !hasSubmittableContent;
  const showStop = isLoading && !hasSubmittableContent && !isStarting;
  const sendTooltip = isStarting
    ? sessionId
      ? COMPOSER_COPY.send.loading
      : COMPOSER_COPY.send.starting
    : isAnyImageLoading
      ? COMPOSER_COPY.send.waitingForImages
      : isAnyDroppedFileLoading
        ? COMPOSER_COPY.send.processingFiles
        : chatState === ChatState.RestartingAgent
          ? COMPOSER_COPY.send.restarting
          : COMPOSER_COPY.send.tooltip;

  const sendOrStop = showStop ? (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          onClick={() => void stopAck.trigger(false)}
          // 32×32, matching Send exactly: the two swap in place as a turn
          // starts and ends.
          size="default"
          shape="round"
          className={cn('flex-shrink-0', stopAck.acknowledged && 'scale-90')}
          data-testid="chat-stop-button"
          data-stop-acknowledged={stopAck.acknowledged}
          aria-label={
            stopAck.acknowledged ? COMPOSER_COPY.stop.labelAcknowledged : COMPOSER_COPY.stop.label
          }
        >
          <Stop size={16} />
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {stopAck.acknowledged ? COMPOSER_COPY.stop.tooltipAcknowledged : COMPOSER_COPY.stop.tooltip}
      </TooltipContent>
    </Tooltip>
  ) : (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="flex-shrink-0">
          <Button
            type="submit"
            // Its OWN composer's form, see `composerFormId`.
            form={composerFormId}
            size="default"
            shape="round"
            // Quiet at rest, accent when armed: the same 32×32 box, so the
            // swap moves no pixels, only the fill and the ink.
            variant={isSubmitButtonDisabled ? 'secondary' : 'default'}
            disabled={isSubmitButtonDisabled || isStarting}
            aria-label={COMPOSER_COPY.send.label}
            aria-busy={isStarting || undefined}
            className={cn(
              (isSubmitButtonDisabled || isStarting) && 'cursor-not-allowed disabled:opacity-100'
            )}
          >
            {isStarting ? <Spinner size={16} /> : <ArrowUp className="size-4" />}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>
        <p>{sendTooltip}</p>
        {/* The steer chord, named only while a turn is running and plain Enter
            would queue this message instead. */}
        {canSteer && !isAnyImageLoading && !isAnyDroppedFileLoading && (
          <p className="text-text-muted">{COMPOSER_COPY.send.steerHint(getSteerShortcutText())}</p>
        )}
      </TooltipContent>
    </Tooltip>
  );

  return (
    // ONE CARD AND ONE LINE (spec 3.7, Crew's composer recipe plus Codex's
    // footer). The card holds what is being written and every control that
    // shapes it: the queue ahead of it, its notices, its chips, the text, and a
    // controls row ending in Send. Under it, one muted footer line says where
    // the agent works and what the chat has used.
    //
    // The drop target is the WRAPPER, so a file dropped on the footer still
    // attaches.
    <div
      className={cn('relative flex w-full flex-col gap-1', !disableAnimation && 'page-transition')}
      data-drop-zone="true"
      data-drag-active={isDraggingOver ? 'true' : 'false'}
      onDrop={handleLocalDrop}
      onDragEnter={handleLocalDragEnter}
      onDragOver={handleLocalDragOver}
      onDragLeave={handleLocalDragLeave}
    >
      <div
        className={cn(
          'relative flex min-w-0 flex-col rounded-container py-2.5 pr-3 pl-4',
          // One edge, a hairline at 60% of the token: the field's shape, not an
          // outline drawn around it.
          'bg-background-default border border-border-subtle/60',
          'transition-[box-shadow,background-color,border-color]',
          // The focus edge (`:has(textarea:focus)`, full accent) and the working
          // edge (`[data-working]`) are authored CSS in `main.css`, shared with
          // Crew's composer.
          'biorouter-composer-card',
          // Drag-over is a transient state worth showing; inset, so it costs no
          // layout.
          isDraggingOver && 'inset-shadow-accent',
          isDraggingOver && 'bg-background-medium/80'
        )}
        // THE WORKING EDGE: an attribute, not a class, so the hook is something
        // Tailwind never had to generate. Absent when idle.
        data-working={isWorking ? 'true' : undefined}
      >
        {queuedMessages.length > 0 && (
          <MessageQueue
            queuedMessages={queuedMessages}
            onRemoveMessage={handleRemoveQueuedMessage}
            onClearQueue={handleClearQueue}
            onStopAndSend={handleStopAndSend}
            // `steerApplies`, not `canSteer`: the queue decides for itself
            // whether to offer the button or the SD-8 note, and it can only
            // tell the two apart while it can still see that a turn is running.
            // `handleSteerMessage` re-checks `canSteer` before it posts, so the
            // callback being bound here grants nothing.
            onSteerMessage={steerApplies ? handleSteerMessage : undefined}
            onReorderMessages={handleReorderMessages}
            onEditMessage={handleEditMessage}
            onTriggerQueueProcessing={handleResumeQueue}
            editingMessageIdRef={editingMessageIdRef}
            isPaused={queuePausedRef.current}
          />
        )}

        {/* Why Send is off, said in the composer with the way out, instead of
            letting the person press it and meet a daemon error. */}
        {noModelConfigured && (
          <div data-testid="composer-no-model-hint" className="br-composer-notice">
            <span className="text-supporting">{NO_MODEL_COMPOSER_HINT}</span>
            <Button
              type="button"
              variant="link"
              size="xs"
              className="h-auto px-0 text-supporting"
              onClick={() => setView?.('ConfigureProviders')}
              data-testid="composer-no-model-action"
            >
              {NO_MODEL_COMPOSER_ACTION}
            </Button>
          </div>
        )}

        {/* Images attached for a model that cannot read them. Blocks Send
            until resolved. */}
        {visionMismatch && (
          <div data-testid="composer-vision-notice" className="br-composer-notice">
            <EyeOff className="br-composer-notice__icon" aria-hidden />
            <span className="text-supporting">{COMPOSER_COPY.vision.banner}</span>
            <InfoTip label={COMPOSER_COPY.vision.banner} help={COMPOSER_COPY.vision.help} />
            <Button
              type="button"
              variant="link"
              size="xs"
              className="h-auto px-0 text-supporting"
              onClick={openModelMenu}
            >
              {COMPOSER_COPY.vision.action}
            </Button>
          </div>
        )}

        {/* What goes with the message, in one row above the text: references
            (issue #65), images and files. */}
        <ComposerChips
          refs={composerRefs}
          images={pastedImages}
          files={allDroppedFiles}
          onRemoveRef={handleRemoveReference}
          onRemoveImage={handleRemovePastedImage}
          onRetryImage={(id) => void handleRetryImageSave(id)}
          onRemoveFile={handleRemoveDroppedFile}
        />

        <form id={composerFormId} onSubmit={onFormSubmit} className="relative flex min-w-0">
          {/* `py-1.5` makes the textarea's box 32px around a 20px line, so the
              text sits centred however it grows; `px-0` keeps the text on the
              card's 16px edge. */}
          <textarea
            data-testid="chat-input"
            autoFocus={autoFocusAtMountRef.current}
            id="dynamic-textarea"
            // An invitation, never an instruction: "Ask a follow-up" once the
            // chat has messages, the open question before that.
            placeholder={
              crewHold?.placeholder ??
              ((messagesLength ?? 0) > 0
                ? COMPOSER_COPY.placeholderFollowUp
                : COMPOSER_COPY.placeholderEmpty)
            }
            value={composerBody}
            onChange={handleChange}
            onCompositionStart={handleCompositionStart}
            onCompositionEnd={handleCompositionEnd}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            ref={textAreaRef}
            rows={1}
            style={{
              maxHeight: `${maxHeight}px`,
              overflowY: 'auto',
            }}
            className={cn(
              'block w-full resize-none border-none bg-transparent px-0 py-1.5',
              COMPOSER_INPUT_TYPE_CLASS,
              'text-text-default placeholder:text-text-muted'
            )}
          />
        </form>

        {/* THE CONTROLS ROW: what to add, what the agent can reach, which model
            answers, and Send at the end, Crew's `.crew-compose-controls`. */}
        <div data-testid="chat-input-toolbar" className="br-composer-controls">
          <ComposerPlusMenu onAttachFiles={attachPickedFiles} onInsertTrigger={insertTrigger} />
          <ToolsChip sessionId={sessionId} privacyTier={sessionPrivacyTier} />
          <span className="br-composer-controls__spacer" />
          <div ref={modelSlotRef} className="br-composer-controls__model">
            <ModelEffortChip
              sessionId={sessionId}
              reasoningScope={
                sessionId
                  ? sessionReasoningScope(sessionId)
                  : draftReasoningScope(reasoningDraftKey ?? draftKey ?? anonymousReasoningDraftKey)
              }
              effectiveModel={effectiveModel}
              privacyTier={sessionPrivacyTier}
              setView={setView}
            />
          </div>
          {sendOrStop}
        </div>
      </div>

      {/* THE FOOTER LINE: the folder on the left, the context ring and the cost
          on the right, 12px muted, on the canvas under the card. The ring and
          the cost show only inside a chat. */}
      <ComposerFooter
        className="pl-3 pr-2.5"
        sessionId={sessionId}
        workingDir={sessionWorkingDir ?? getInitialWorkingDir()}
        workingDirLocked={workingDirIsLocked}
        onWorkingDirChange={(newDir) => {
          setSessionWorkingDir(newDir);
          onWorkingDirChange?.(newDir);
        }}
        onRestartStart={() => setChatState?.(ChatState.RestartingAgent)}
        onRestartEnd={() => setChatState?.(ChatState.Idle)}
        totalTokens={totalTokens}
        tokenLimit={tokenLimit}
        isTokenLimitLoaded={isTokenLimitLoaded}
        onCompact={compactNow}
        inputTokens={accumulatedInputTokens}
        outputTokens={accumulatedOutputTokens}
        sessionCosts={sessionCosts}
        modelCostRows={modelCostRows}
      />

      <MentionPopover
        ref={mentionPopoverRef}
        isOpen={mentionPopover.isOpen}
        isSlashCommand={mentionPopover.isSlashCommand}
        onClose={() => setMentionPopover((prev) => ({ ...prev, isOpen: false }))}
        onSelect={handleMentionItemSelect}
        position={mentionPopover.position}
        query={mentionPopover.query}
        selectedIndex={mentionPopover.selectedIndex}
        onSelectedIndexChange={(index) =>
          setMentionPopover((prev) => ({ ...prev, selectedIndex: index }))
        }
        workingDir={sessionWorkingDir ?? getInitialWorkingDir()}
        sessionId={sessionId}
      />
    </div>
  );
}
