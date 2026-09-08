import React, { useRef, useState, useEffect, useMemo, useCallback } from 'react';
import { annotationContextText, onArtifactAnnotation } from '../utils/annotationChannel';
import { ArrowUp, ChevronsDownUp, Plus, X } from './icons/app-icons';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { useComposerToolbarCollapsed } from './bottom_menu/useComposerToolbarCollapsed';
import { ContextWindowIndicator } from './ContextWindowIndicator';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/Tooltip';
import { Button } from './ui/button';
import type { View } from '../utils/navigationUtils';
import Stop from './ui/Stop';
import { ChatState } from '../types/chatState';
import debounce from 'lodash/debounce';
import { LocalMessageStorage } from '../utils/localMessageStorage';
import { DirSwitcher } from './bottom_menu/DirSwitcher';
import ModelsBottomBar from './settings/models/bottom_bar/ModelsBottomBar';
import { BottomMenuExtensionSelection } from './bottom_menu/BottomMenuExtensionSelection';
import { BottomMenuSkillSelection } from './bottom_menu/BottomMenuSkillSelection';
import { BottomMenuKnowledgeSelection } from './bottom_menu/BottomMenuKnowledgeSelection';
import { BottomMenuReasoningEffort } from './bottom_menu/BottomMenuReasoningEffort';
import { AlertType, useAlerts } from './alerts';
import { toolCountWarning } from './alerts/toolCountWarning';
import { useConfig } from './ConfigContext';
import { useModelAndProvider } from './ModelAndProviderContext';
import {
  NO_MODEL_COMPOSER_ACTION,
  NO_MODEL_COMPOSER_HINT,
  hasNoModelConfigured,
} from './composerNoProvider';
import MentionPopover, { DisplayItemWithMatch } from './MentionPopover';
import { COST_TRACKING_ENABLED } from '../updates';
import { CostTracker } from './bottom_menu/CostTracker';
import type { ModelCostRow } from '../hooks/useCostTracking';
import { DroppedFile, useFileDrop } from '../hooks/useFileDrop';
import { useDiverge } from '../hooks/useDiverge';
import { Workflow } from '../workflow';
import MessageQueue, { canSteerMessage } from './MessageQueue';
import { detectInterruption } from '../utils/interruptionDetector';
import { getSession, llamacppStatus, Message } from '../api';
import { userActionHeaders } from '../utils/userAction';
import type { SessionClassification } from '../api/types.gen';
import { getInitialWorkingDir } from '../utils/workingDir';
import { getPredefinedModelsFromEnv } from './settings/models/predefinedModelsUtils';
import { getNavigationShortcutText, getSteerShortcutText } from '../utils/keyboardShortcuts';
import type { UserAttachment } from '../types/message';
import { useStopAcknowledgement } from '../hooks/useStopAcknowledgement';
import { isRunningState } from '../hooks/chatStreamStore';
import { toastWarning } from '../toasts';
import { cn } from '../utils';
import {
  appendComposerRef,
  joinComposerText,
  removeComposerRefAt,
  splitComposerText,
} from '../utils/composerRefs';
import { findRefTags } from '../utils/resourceRefs';
import { ResourceRefChip } from './ResourceRefChip';

interface QueuedMessage {
  id: string;
  content: string;
  attachments?: UserAttachment[];
  /** Renderer-owned temp images to unlink if the queue discards this message.
   * Kept separate from `attachments`: that array may also contain a user's
   * original file path, which this component must never delete. */
  ownedTempAttachmentPaths?: string[];
  timestamp: number;
}

const orphanedQueuedOffers = new Map<string, Map<string, QueuedMessage>>();
const orphanedQueuedOfferListeners = new Map<string, Set<(messageId: string) => void>>();

function queuedOfferOwner(sessionId: string | null): string {
  return sessionId ?? '__new_session__';
}

function settleOrphanedQueuedOffer(owner: string, messageId: string): void {
  const offers = orphanedQueuedOffers.get(owner);
  offers?.delete(messageId);
  if (offers?.size === 0) orphanedQueuedOffers.delete(owner);
  for (const listener of orphanedQueuedOfferListeners.get(owner) ?? []) listener(messageId);
}

function deleteQueuedOwnedTempAttachments(messages: readonly QueuedMessage[]): void {
  const ownedPaths = new Set(messages.flatMap((message) => message.ownedTempAttachmentPaths ?? []));
  for (const path of ownedPaths) window.electron.deleteTempFile(path);
}

interface PastedImage {
  id: string;
  dataUrl: string; // For immediate preview
  filePath?: string; // Path on filesystem after saving
  isLoading: boolean;
  error?: string;
}

// Constants for image handling
const MAX_IMAGES_PER_MESSAGE = 5;
const MAX_IMAGE_SIZE_MB = 3;

// Constants for token and tool alerts
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
 * The composer's controls are grouped by ROW, and the rows are not boxes.
 *
 * Three arrangements have been tried. A single card holding everything read as
 * a settings panel you could also type into. A recessed banner tucked behind a
 * card grouped the controls correctly but paid two outlines and an 8px overlap
 * for it. One card of three rows fixed that, and then made the card so tall and
 * so full that "crowded" was the note two rounds running.
 *
 * What ships now inverts it: the CARD IS THE INPUT, and nothing else. The
 * context row floats above it and the control row below it, both directly on
 * the canvas, so the only boxed thing on screen is the one thing the user acts
 * on. The grouping that needed a rule, then a tuck, then a hairline is now done
 * by the canvas gaps alone.
 *
 * Chips inside one cluster sit at 8px. They were at 2px when the strip was its
 * own recessed surface and 4px when it was a card row; on bare canvas there is
 * no fill or edge left to hold them together, so the spacing has to do it, and
 * a chip cluster that is merely NOT touching reads as a cramped toolbar.
 */
const TOOLBAR_GROUP_CLASS = 'flex flex-shrink-0 items-center gap-2';

/*
 * ═══════════════════════════════════════════════════════════════════════════
 * THE RAILS' TYPE IS `text-supporting` (12/16), AND THAT OVERRIDES A RULE.
 * ═══════════════════════════════════════════════════════════════════════════
 *
 * This governs the controls in row 1 and row 3 — the directory, the extension /
 * skill / knowledge counts, the reasoning knob, the model, the context gauge's
 * figure, the cost. They live in their own files (`bottom_menu/*`,
 * `ContextWindowIndicator`), every one of which is composer-only, so the change
 * is scoped even though it is spread. Each carries a pointer back here.
 *
 * ⚠ THE RULE THIS SUBORDINATES, so nobody "fixes" it back: `main.css`'s type
 * scale states the sanctioned dense-control exception is `--text-secondary`
 * (13px) and says in terms — "It does NOT drop to `text-supporting`; a control
 * that renders at metadata size stops looking like something you can press."
 * These controls were raised 12 -> 13 earlier for exactly that reason. They are
 * now back at 12 at the USER'S EXPLICIT DIRECTION, with a stated goal: the rails
 * should "read as annotations rather than competing UI" against the message
 * text. That is a call about this surface's hierarchy, and it outranks a
 * default.
 *
 * The ratio is the part to preserve if the body size ever moves. The spec asked
 * for rails one clear step under a 15px body; our body is `--text-body` 14px
 * (and the composer's input deliberately shares that role, so what you type and
 * what it becomes are one size). One step under 14 on our scale is 12 —
 * `text-supporting` — which also lands inside the spec's own 11.5-12.5 band.
 * Do not import 15px, and do not invent a half-pixel role to hit it exactly.
 *
 * What answers the rule's actual objection — "stops looking like something you
 * can press" — is that pressability here is not carried by type size at all.
 * Every one of these keeps a 28px (`h-7`) hit area, a hover fill, and an icon or
 * caret. The type shrank; the affordance did not.
 */

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

/**
 * The control rail's ONE separator, between the reasoning knob and the model.
 *
 * This is not the old toolbar vocabulary returning. That row carried several
 * rules across eight controls, which is what made a strip of chips read as a
 * control panel; this one separates the two settings that answer the same
 * question — "how does the next message get answered" — from each other, and it
 * is the only separator inside any row of the composer.
 *
 * A 3px DOT, not a 14px rule, and the change is not cosmetic. A vertical rule
 * is a piece of RULING: it runs the height of its neighbours, so it reads as
 * the beginning of a column and invites more of itself — which is exactly how
 * the row got to eight controls and several rules the first time. A dot has no
 * height to align to. It says "these are two groups" and nothing else, which is
 * all this seam ever needed, and it cannot accrete into a grid.
 *
 * IT IS NOT A BORDER TOKEN, and that is the part that had to be measured rather
 * than assumed. Contrast is area-dependent: the old rule laid 14 px of ink on
 * the canvas, a 3px dot lays about 7, so a token that read as a hairline reads
 * as dirt as a dot. Against `--background-canvas`, across all six scopes:
 *
 *   --border-subtle        1.27 light / 1.39 dark   invisible as a dot
 *   --border-strong        1.52       / 1.71        still washed out
 *   text-muted/55          2.37-2.48  / 3.01-3.62   present, clearly subordinate
 *   --text-muted (full)    6.11-6.89  / 7.37-9.41   darker than what it divides
 *
 * The rail's own labels measure 5.71-6.19 light / 8.08-8.55 dark, so `/55` puts
 * the dot at roughly 40% of the ink it separates: enough to be seen on purpose,
 * never enough to be read as content. A separator that outweighs its neighbours
 * — which the full token does — is just punctuation shouting.
 */
const TOOLBAR_DIVIDER_CLASS = 'size-[3px] flex-shrink-0 rounded-full bg-text-muted/55';

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
  droppedFiles?: DroppedFile[];
  onFilesProcessed?: () => void;
  setView: (view: View) => void;
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
}

export default function ChatInput({
  sessionId,
  handleSubmit,
  chatState = ChatState.Idle,
  setChatState,
  onStop,
  onAbandonContinuation,
  submissionBlocked = false,
  onSteer,
  commandHistory = [],
  initialValue = '',
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
  toolCount,
  append: _append,
  onWorkingDirChange,
  supportsVisionOverride,
  supportedInputMimeTypesOverride,
}: ChatInputProps) {
  const [_value, setValue] = useState(initialValue);
  const [displayValue, setDisplayValue] = useState(initialValue); // For immediate visual feedback
  // (`isFocused` used to live here, mirroring the textarea's focus into React
  // purely so the card could paint a ring. The card now asks CSS directly with
  // `has-[textarea:focus]`, which is one source of truth instead of two and
  // cannot fall out of sync with the DOM the way a mirrored flag can.)
  const [pastedImages, setPastedImages] = useState<PastedImage[]>([]);
  const pastedImagesRef = useRef(pastedImages);
  pastedImagesRef.current = pastedImages;

  // Loading gates composer operations; live work drives the visual activity
  // indicator. LoadingConversation is intentionally only in the former.
  const isLoading = chatState !== ChatState.Idle;
  const isWorking = isRunningState(chatState);
  const wasLoadingRef = useRef(isLoading);

  // Queue functionality - renderer-memory only, scoped to this chat. An offer
  // crossing an unmount is handed to the next composer instance below.
  const queueOwner = queuedOfferOwner(sessionId);
  const recoveredQueuedOffersRef = useRef<QueuedMessage[] | null>(null);
  if (recoveredQueuedOffersRef.current === null) {
    recoveredQueuedOffersRef.current = [...(orphanedQueuedOffers.get(queueOwner)?.values() ?? [])];
    orphanedQueuedOffers.delete(queueOwner);
  }
  const recoveredQueuedOffers = recoveredQueuedOffersRef.current ?? [];
  const recoveredQueuedMessageIdsRef = useRef(
    new Set(recoveredQueuedOffers.map((message) => message.id))
  );
  const [queuedMessages, setQueuedMessages] = useState<QueuedMessage[]>(recoveredQueuedOffers);
  const queuedMessagesRef = useRef(queuedMessages);
  queuedMessagesRef.current = queuedMessages;
  const activeQueuedOffersRef = useRef(new Map<string, QueuedMessage>());
  const queueDisposedRef = useRef(false);
  const queuePausedRef = useRef(false);
  const editingMessageIdRef = useRef<string | null>(null);
  const continuationQueuedMessageIdRef = useRef<string | null>(null);
  const [lastInterruption, setLastInterruption] = useState<string | null>(null);

  const { alerts, addAlert, clearAlerts } = useAlerts();
  const dropdownRef: React.RefObject<HTMLDivElement> = useRef<HTMLDivElement>(
    null
  ) as React.RefObject<HTMLDivElement>;
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
  const [sessionPrivacyTier, setSessionPrivacyTier] = useState<SessionClassification | undefined>(
    undefined
  );
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
        // Issue #56 Task 58: and this read is *about* the tier, so it is
        // exactly the read the gate refuses without the header.
        headers: await userActionHeaders(),
      });
      if (issued !== tierGenerationRef.current) return;
      if (tierSessionRef.current !== sessionId) return;
      if (response.data?.privacy_tier) {
        setSessionPrivacyTier(response.data.privacy_tier);
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
    setSessionPrivacyTier(undefined);

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
   * ⚠ **Once per turn, and never once the answer is `private`.** `GET
   * /sessions/{id}` carries the whole transcript (`get_session(id, true)`), and
   * within a chat the ratchet only ever raises — so a second read during the
   * same turn cannot change the answer and would only re-fetch the conversation.
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
  useEffect(() => {
    const listener = (messageId: string) => {
      recoveredQueuedMessageIdsRef.current.delete(messageId);
      activeQueuedOffersRef.current.delete(messageId);
      setQueuedMessages((prev) => prev.filter((message) => message.id !== messageId));
    };
    const listeners = orphanedQueuedOfferListeners.get(queueOwner) ?? new Set();
    listeners.add(listener);
    orphanedQueuedOfferListeners.set(queueOwner, listeners);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) orphanedQueuedOfferListeners.delete(queueOwner);
    };
  }, [queueOwner]);

  useEffect(() => {
    const timers = queueRetryTimersRef.current;
    const activeQueuedOffers = activeQueuedOffersRef.current;
    const recoveredQueuedMessageIds = recoveredQueuedMessageIdsRef.current;
    queueDisposedRef.current = false;
    return () => {
      queueDisposedRef.current = true;
      timers.forEach((timer) => clearTimeout(timer));
      timers.clear();

      const recoverable = new Map(orphanedQueuedOffers.get(queueOwner));
      for (const [id, message] of activeQueuedOffers) {
        recoverable.set(id, message);
      }
      for (const message of queuedMessagesRef.current) {
        if (recoveredQueuedMessageIds.has(message.id)) {
          recoverable.set(message.id, message);
        }
      }
      if (recoverable.size > 0) orphanedQueuedOffers.set(queueOwner, recoverable);
    };
  }, [queueOwner]);

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
   */
  const offerQueuedMessage = useCallback(
    (message: QueuedMessage) => {
      LocalMessageStorage.addMessage(message.content);
      activeQueuedOffersRef.current.set(message.id, message);
      const offer = (attempt: number) => {
        if (queueDisposedRef.current) return;
        const submitted = handleSubmit(
          new CustomEvent('submit', {
            detail: { value: message.content, attachments: message.attachments ?? [] },
          }) as unknown as React.FormEvent
        );
        void Promise.resolve(submitted).then((accepted) => {
          // Only an explicit `false` is a refusal: a handler predating the
          // contract resolves `undefined` and must not be read as one.
          if (accepted !== false) {
            activeQueuedOffersRef.current.delete(message.id);
            recoveredQueuedMessageIdsRef.current.delete(message.id);
            settleOrphanedQueuedOffer(queueOwner, message.id);
            if (continuationQueuedMessageIdRef.current === message.id) {
              continuationQueuedMessageIdRef.current = null;
            }
            return;
          }
          // Cleanup already copied this in-flight offer to the renderer-level
          // recovery map. A late refusal must leave it there and must not arm a
          // timer or set state on a composer that no longer exists.
          if (queueDisposedRef.current) return;
          if (attempt < QUEUE_DRAIN_ATTEMPTS) {
            // Next macrotask, which is all the in-flight submit's promise chain
            // needs to unwind and release its latch. Deliberately not a poll:
            // the attempt count is the whole retry budget.
            const timer = setTimeout(() => {
              queueRetryTimersRef.current.delete(timer);
              offer(attempt + 1);
            }, 0);
            queueRetryTimersRef.current.add(timer);
            return;
          }
          activeQueuedOffersRef.current.delete(message.id);
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
    [handleSubmit, queueOwner]
  );

  // Queue processing
  useEffect(() => {
    if (wasLoadingRef.current && !isLoading && queuedMessages.length > 0) {
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

  // Update internal value when initialValue changes
  useEffect(() => {
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
  const [hasUserTyped, setHasUserTyped] = useState(false);
  const textAreaRef = useRef<HTMLTextAreaElement>(null);
  const timeoutRefsRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set());
  // Rung 3b of the yield ladder: when the composer's control row is too narrow
  // to lay every picker out, they collapse behind a single "+" at the lower-left
  // instead of overlapping. Measured on the toolbar's own box (see the hook).
  const toolbarRef = useRef<HTMLDivElement>(null);
  const composerToolbarCollapsed = useComposerToolbarCollapsed(toolbarRef);

  // Re-populate the composer when a submit failed before the backend accepted it
  // (e.g. backend unreachable): BaseChat.handleCreateSessionError dispatches
  // 'restore-chat-input' with the text that performSubmit had already cleared, so
  // the user does not silently lose what they typed. Match by sessionId — with
  // `null === null` for the pre-session Home composer — so a broadcast can only
  // restore the input that actually submitted, never a sibling.
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ sessionId?: string | null; value?: string }>).detail;
      if ((detail?.sessionId ?? null) !== (sessionId ?? null)) return;
      if (typeof detail?.value !== 'string' || !detail.value) return;
      setDisplayValue(detail.value);
      setValue(detail.value);
      setHasUserTyped(true);
      textAreaRef.current?.focus();
    };
    window.addEventListener('restore-chat-input', handler);
    return () => window.removeEventListener('restore-chat-input', handler);
  }, [sessionId]);

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
  } = useFileDrop();

  // Merge local dropped files with parent dropped files. Keep every dropped
  // item visible: model capability determines whether an image is uploaded as
  // content or sent as a filesystem path, never whether the drop disappears.
  const allDroppedFiles = useMemo(() => {
    return [...droppedFiles, ...localDroppedFiles];
  }, [droppedFiles, localDroppedFiles]);

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

  useEffect(() => {
    if (textAreaRef.current) {
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
      const { model, provider } = await getCurrentModelAndProvider();
      if (!model || !provider) {
        console.log('No model or provider found');
        setIsTokenLimitLoaded(true);
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

  // Initial load and refresh when model changes
  useEffect(() => {
    loadProviderDetails();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentModel, currentProvider]);

  // Handle tool count alerts and token usage
  useEffect(() => {
    clearAlerts();

    // Show alert when either there is registered token usage, or we know the limit
    if ((totalTokens && totalTokens > 0) || (isTokenLimitLoaded && tokenLimit)) {
      addAlert({
        type: AlertType.Info,
        message: 'Context window',
        progress: {
          current: totalTokens || 0,
          total: tokenLimit,
        },
        showCompactButton: true,
        compactButtonDisabled: !totalTokens,
        onCompact: () => {
          window.dispatchEvent(new CustomEvent('hide-alert-popover'));

          const customEvent = new CustomEvent('submit', {
            detail: { value: MANUAL_COMPACT_TRIGGER },
          }) as unknown as React.FormEvent;

          handleSubmit(customEvent);
        },
        compactIcon: <ChevronsDownUp size={12} />,
      });
    }

    // Add tool count alert if we have the data
    const countWarning = toolCountWarning(toolCount, sessionId);
    if (countWarning) addAlert(countWarning);
    // `handleSubmit` changes with composer state; alerts should refresh from
    // their metrics, not every time the submit callback is rebuilt.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [totalTokens, toolCount, tokenLimit, isTokenLimitLoaded, sessionId, addAlert, clearAlerts]);

  // Cleanup effect for component unmount - prevent memory leaks
  useEffect(() => {
    return () => {
      // Clear any pending timeouts from image processing
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

      // Clear all tracked timeouts
      // eslint-disable-next-line react-hooks/exhaustive-deps
      const timeouts = timeoutRefsRef.current;
      timeouts.forEach((timeoutId) => {
        window.clearTimeout(timeoutId);
      });
      timeouts.clear();

      // Clear alerts to prevent memory leaks
      clearAlerts();
    };
  }, [clearAlerts]);

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

  const canSteer = Boolean(onSteer) && isLoading;

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
    (content: string) => {
      if (!onSteer) return;
      void onSteer(content).then((accepted) => {
        if (accepted) {
          // The agent echoes the steer back as a user message on the live stream
          // once it consumes it, so nothing is appended to the transcript here.
          LocalMessageStorage.addMessage(content);
        } else {
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
    const messageToSteer = queuedMessages.find((msg) => msg.id === messageId);
    if (!messageToSteer || !canSteer) return;

    setQueuedMessages((prev) => prev.filter((msg) => msg.id !== messageId));
    steerText(messageToSteer.content);
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
  const noModelConfigured = hasNoModelConfigured(modelConfigStatus, currentProvider);

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
      if (trimmedCandidate === DIVERGE_TRIGGER) {
        if (sessionId) {
          void diverge(sessionId);
        }
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
        // submit has said whether it took the message. When it did NOT (a
        // re-entrant send, or a controller with no session, which is the fresh
        // tab that accepts text, clears it and creates nothing), the text was
        // gone with no error and no message. Put it back through the same
        // channel a failed `createSession` uses. What goes back is
        // `displayValue`, not `textToSend`: it must be the box the user was
        // looking at, reference chips included, appended dropped-file paths not.
        const restoredText = displayValue.trim() ? displayValue : textToSend;
        const restoredImages = pastedImages;
        void Promise.resolve(submitted).then((accepted) => {
          if (accepted !== false) return;
          window.dispatchEvent(
            new CustomEvent('restore-chat-input', {
              detail: { sessionId: sessionId ?? null, value: restoredText },
            })
          );
          // Attachments ride a different channel than the restore event, so they
          // are handed straight back to the state they were cleared from.
          if (restoredImages.length > 0) {
            setPastedImages(restoredImages);
          }
        });

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
      }
    },
    [
      allDroppedFiles,
      canSendDroppedFileAsPath,
      canUploadDroppedImage,
      currentModelSupportsVision,
      displayValue,
      diverge,
      droppedFilePath,
      droppedImageAttachmentPath,
      droppedFiles.length,
      handleSubmit,
      lastInterruption,
      localDroppedFiles.length,
      onFilesProcessed,
      pastedImages,
      sessionId,
      setLocalDroppedFiles,
    ]
  );

  const handleKeyDown = (evt: React.KeyboardEvent<HTMLTextAreaElement>) => {
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
      }
    }
  };

  const onFormSubmit = (e: React.FormEvent) => {
    e.preventDefault();
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
    noModelConfigured ||
    !hasSubmittableContent ||
    isAnyImageLoading ||
    isAnyDroppedFileLoading ||
    chatState === ChatState.RestartingAgent ||
    submissionBlocked ||
    visionMismatch;

  // Queue management functions - no storage persistence, only in-memory
  const handleRemoveQueuedMessage = (messageId: string) => {
    if (continuationQueuedMessageIdRef.current === messageId) {
      continuationQueuedMessageIdRef.current = null;
      void onAbandonContinuation?.();
    }
    recoveredQueuedMessageIdsRef.current.delete(messageId);
    activeQueuedOffersRef.current.delete(messageId);
    orphanedQueuedOffers.get(queueOwner)?.delete(messageId);
    const removed = queuedMessagesRef.current.find((message) => message.id === messageId);
    if (removed) deleteQueuedOwnedTempAttachments([removed]);
    setQueuedMessages((prev) => prev.filter((msg) => msg.id !== messageId));
  };

  const handleClearQueue = () => {
    if (continuationQueuedMessageIdRef.current) {
      continuationQueuedMessageIdRef.current = null;
      void onAbandonContinuation?.();
    }
    recoveredQueuedMessageIdsRef.current.clear();
    activeQueuedOffersRef.current.clear();
    orphanedQueuedOffers.delete(queueOwner);
    deleteQueuedOwnedTempAttachments(queuedMessagesRef.current);
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
    // Own the future lease before asking for it. Removal, Clear, and unmount
    // can now revoke this exact queued replacement while the cancel barrier is
    // still on the wire; the delayed acknowledgement below observes that
    // revocation and abandons the lease instead of orphaning it Live.
    continuationQueuedMessageIdRef.current = messageId;

    void stopAck.trigger(true).then((stopped) => {
      stopAndSendPendingRef.current.delete(messageId);
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

  // The composer's controls, defined ONCE and arranged in three places: the
  // card's context strip (row 1), its control bar (row 3), and the collapsed
  // "+" popover. They used to be declared inside the toolbar's IIFE, which is
  // why the context strip could not exist — a control cannot be lifted out of
  // the row that defines it. Hoisting them here is the whole enabling change.
  //
  // #44: the working dir is choosable only while the chat is completely empty
  // (pre-session #39 path included); the first message locks it for the
  // session's lifetime. Prefer the authoritative lock from BaseChat (hydration-
  // and failed-submit-aware); fall back to the transcript length for callers
  // that do not track session metadata.
  const workingDirIsLocked = workingDirLocked ?? messagesLength > 0;
  const dirSwitcher = (
    <DirSwitcher
      // NO RESTING FILL, and the reason is a token collision worth recording.
      //
      // This chip carried `bg-background-muted` while the context strip was a
      // row INSIDE the card: muted is one step off `background-default`, so the
      // fill read as a raised pill and distinguished the working directory from
      // the three bare count chips beside it. The strip now sits on the CANVAS,
      // and `--background-canvas` is byte-identical to `--background-muted` in
      // both Parchment dark (#282217) and Alma Mater dark (#0d2a50) — so the
      // pill did not merely lose contrast in two of the six combinations, it
      // disappeared completely, taking the chip's only visible edge with it.
      //
      // Nothing replaces it, because nothing needs to: the design has no pill
      // here either, and `justify-between` already says what the fill was
      // saying — the directory is the thing on the left, the counts are the
      // tally on the right. The chip keeps its hover fill, which is a response
      // to the pointer rather than a permanent surface and so is allowed to be
      // one step off whatever it happens to be standing on.
      className="mr-0"
      sessionId={sessionId ?? undefined}
      locked={workingDirIsLocked}
      workingDir={sessionWorkingDir ?? getInitialWorkingDir()}
      onWorkingDirChange={(newDir) => {
        setSessionWorkingDir(newDir);
        if (onWorkingDirChange) {
          onWorkingDirChange(newDir);
        }
      }}
      onRestartStart={() => setChatState?.(ChatState.RestartingAgent)}
      onRestartEnd={() => setChatState?.(ChatState.Idle)}
    />
  );
  const extensionsSkillsKnowledge = (
    <>
      <BottomMenuExtensionSelection sessionId={sessionId} privacyTier={sessionPrivacyTier} />
      <BottomMenuSkillSelection sessionId={sessionId} />
      <BottomMenuKnowledgeSelection />
    </>
  );
  const reasoning = <BottomMenuReasoningEffort />;
  const model = (
    <div className="min-w-0">
      <ModelsBottomBar
        sessionId={sessionId}
        privacyTier={sessionPrivacyTier}
        dropdownRef={dropdownRef}
        setView={setView}
        alerts={alerts}
        hideAlertPopover
      />
    </div>
  );
  const contextGauge = (
    <ContextWindowIndicator
      totalTokens={totalTokens}
      tokenLimit={tokenLimit}
      isTokenLimitLoaded={isTokenLimitLoaded}
      // The control bar has the width to print the headroom, and a bare ring
      // states only "some of it is gone" — the figure is the reason anyone
      // looks. (Remaining, matching the arc; see the prop's own note.)
      showRemainingPercent
      onCompact={() => {
        handleSubmit(
          new CustomEvent('submit', {
            detail: { value: MANUAL_COMPACT_TRIGGER },
          }) as unknown as React.FormEvent
        );
      }}
    />
  );
  const cost = COST_TRACKING_ENABLED ? (
    <CostTracker
      inputTokens={accumulatedInputTokens}
      outputTokens={accumulatedOutputTokens}
      sessionCosts={sessionCosts}
      modelCostRows={modelCostRows}
    />
  ) : null;

  return (
    // THE CARD IS THE INPUT. Nothing else on this surface is boxed.
    //
    // Three arrangements came before (see `TOOLBAR_GROUP_CLASS`), and each one
    // put more inside the card than the card was about. The last of them — one
    // box, three rows, a hairline under the first — grouped correctly and still
    // drew the note "crowded", because a card that contains the directory, the
    // extension counts, the prose, the model, the gauge, the cost and Send is a
    // panel no matter how its rows are spaced.
    //
    // So the box is now drawn around the ONE thing the user acts on:
    //
    //   ROW 1  context   — where am I, what can I reach.   (on the canvas)
    //   ROW 2  the card  — the prose, and Send.            (the only surface)
    //   ROW 3  controls  — how it answers, what it costs.  (on the canvas)
    //
    // Nothing separates the three but canvas: no rule, no fill, no tuck. That
    // is the whole point — the chrome reads as annotation ON the input rather
    // than as more contents OF it, and the input gets to be the only object
    // with an edge.
    //
    // `gap-1.5` IS 6px, AND THE SMALLNESS IS THE DESIGN. It was 10px, and
    // before that 12px, on the theory that a wide gap made the card "read as
    // one object and its two neighbours as satellites". That theory was wrong
    // in a way worth recording, because it is the note this round came in on:
    // ~10px is also roughly what the app puts between UNRELATED blocks, so the
    // rails did not read as satellites of the input — they read as three
    // separate rows that happened to be stacked, "three floating strips".
    //
    // Proximity is the only grouping signal available on a surface with no
    // fills and no rules, and it is RELATIVE, not absolute: what binds the
    // group is that its internal gaps are visibly smaller than the gap to
    // everything around it. So the two numbers are one decision, and neither
    // can be read alone —
    //
    //   6px  here                              the three rows are one object
    //   28px `pt-7` on the composer bar        that object is not the transcript
    //
    // (see `BaseChat.tsx`, which carries the same note from the other side).
    // Widening one without tightening the other just moves the looseness.
    //
    // The drop target stays the WRAPPER rather than the card, so a file dropped
    // on the context row or the control row still attaches.
    <div
      className={cn(
        'relative flex w-full flex-col gap-1.5',
        !disableAnimation && 'page-transition'
      )}
      data-drop-zone="true"
      data-drag-active={isDraggingOver ? 'true' : 'false'}
      onDrop={handleLocalDrop}
      onDragEnter={handleLocalDragEnter}
      onDragOver={handleLocalDragOver}
      onDragLeave={handleLocalDragLeave}
    >
      {/* ROW 1 — CONTEXT, ON THE CANVAS. No fill, no outline, no rule under it.
          It keeps its `data-testid` because it is still the same thing to
          anything looking for it — "where the agent works · what it can reach"
          — even though it is no longer inside anything.

          The directory goes LEFT and the counts go RIGHT (`justify-between`)
          rather than all four hugging the left edge. Four chips in a row read
          as a toolbar; one chip against three across a span reads as a label
          and a tally, which is what they are.

          ⚠ `pl-3` IS NOT A FREE NUMBER, AND A FLAT INSET WILL BREAK IT. A spec
          round asked for both rails at a flat 4px inset, with the stated goal
          "folder icon, placeholder text and the effort bars share one left
          edge". That is the same goal this padding already serves, by the other
          mechanism, and the flat version was measured before it was declined:

            left ink from the shell's edge     row 1 / card / row 3    spread
            solved optically (`pl-3`/`pl-4`/`pl-3.5`)   16 / 17 / 16     1px
            flat 4px on both rails (`pl-1`)              8 / 17 /  6    11px

          A flat inset only lands one column if every chip carries the same
          internal padding, and ours do not — the folder chip has 4px, the
          reasoning chip 2px, and the card contributes a 1px border the rails do
          not have. So the literal 4px would have moved the rails 8-11px LEFT of
          the placeholder and lost the alignment it was asked for. The result was
          kept; the mechanism was not adopted.

          `pl-3` is therefore solved for, not a copy of the design's number. The
          invariant is that the folder glyph here, the placeholder in the card
          and the reasoning glyph below all start on ONE column; each of the
          three sits behind a different amount of its own padding (this chip
          4px, the card's border 1px + inset, the reasoning chip 2px), so the
          row paddings differ precisely so the ink does not. See the card. */}
      <div
        data-testid="composer-context-banner"
        className="flex min-w-0 items-center justify-between gap-4 pl-3 pr-2.5"
      >
        <div className="flex min-w-0 items-center">{dirSwitcher}</div>
        <div className={TOOLBAR_GROUP_CLASS}>{extensionsSkillsKnowledge}</div>
      </div>

      {/* ROW 2 — THE CARD. The prose and Send, and everything attached to the
          message being written: the queue ahead of it, a vision warning about
          it, the reference chips on it, the files under it. Nothing that is a
          SETTING lives in here any more.

          `pl-4 pr-3 py-2.5` — 16px to the text, 12px to Send, 10px top and
          bottom. The horizontal insets stayed where they were: the note two
          rounds running was "the edge distance seems a bit too small", and
          nothing about returning the type to 14/20 makes the SIDES tighter. The
          vertical came in a step, because it is what "less spaced out" reaches
          first — a 20px line inside 12px of card padding read as a small
          sentence in a large box.

          THIS CARD AND THE USER'S BUBBLE ARE ALREADY THE SAME BOX on the two
          axes that are comparable, and the numbers are worth having here so the
          next "make it match the chat bubble" round does not re-derive them.
          `UserMessage.tsx` is `rounded-container bg-background-medium px-3.5
          py-2.5`; measured against this card: radius 12 == 12, vertical padding
          10 == 10. Only the horizontal differs (16/12 here, 14/14 there), and
          that difference is the "edge distance too small" note above — the
          bubble has never drawn it, this card has, twice.

          What does NOT transfer is height: the bubble is 40px around a 20px line
          and this card is 54px, because the card also has to hold a 32px Send.
          Closing that gap means cutting the card's vertical padding BELOW the
          bubble's — trading a padding the two boxes agree on for a height they
          still would not. Measured and left alone.

          The asymmetry is Send's: a 32px button needs less inset than a text
          baseline does to look equally clear of the corner.

          `items-end` on the input line is what keeps Send pinned to the last
          line as the prose grows upward, instead of drifting to the middle of
          a six-line message. */}
      <div
        className={cn(
          'relative flex min-w-0 flex-col rounded-container py-2.5 pr-3 pl-4',
          // ONE EDGE, AND IT IS A HAIRLINE. This card previously carried BOTH
          // `--shadow-composer` and a 1px inset ring. Two edge statements on one
          // box is what reads as "a rim of coloured pixels": in the warm
          // families the elevation tints the pixels just outside the box while
          // the inset ring tints the ones just inside it, so the border appears
          // two or three pixels thick and slightly haloed rather than crisp.
          // A single 1px border states the edge once, which is also the house
          // rule everywhere else in the app — surfaces over elevation.
          //
          // The design does pair a border with a soft drop shadow, and now that
          // the card stands alone on the canvas that is a more defensible thing
          // to want than it was when it had a banner behind it. It is still not
          // taken here: the user's own words about that exact pairing were "a
          // rim of coloured pixels". `shadow-composer` is the one-line change if
          // they ask for it.
          //
          // AND THE ONE EDGE IS A FRACTION OF THE TOKEN — `/60`, not the whole
          // hairline. At full strength the note was "a very thick rim around the
          // chat box": same 1px, but enough contrast that the eye reads an
          // OUTLINE DRAWN AROUND the field rather than the shape OF the field.
          //
          // 60% is measured, not chosen by eye (Chrome, real compiled tokens,
          // composited over the card's own fill):
          //
          //            border vs card fill      border vs canvas
          //   light      1.275 -> 1.152          (canvas == fill, #ffffff)
          //   dark       1.289 -> 1.141          1.389 -> 1.184
          //
          // All three families share one neutral set, so those six scopes are
          // really two. LIGHT IS THE BINDING ONE: `--background-canvas` and
          // `--background-default` are both #ffffff there, so the border is the
          // only thing separating the field from the page and it may not go much
          // lower — 44% (the `.biorouter-modal-surface` idiom) lands at 1.111 and
          // starts to disappear, and a modal can afford that because it sits on a
          // blurred scrim that reinforces its edge. Dark is the safer scope, not
          // the riskier one: the card fill is already a step off the canvas
          // (1.078:1) so the field keeps its shape there even as the hairline
          // fades.
          //
          // Note this is deliberately BELOW `check-contrast.mjs`'s 1.25 floor for
          // `--border-subtle` vs the app ground. That floor governs the TOKEN, at
          // full strength, where a hairline is the whole statement. This call
          // site wants one step lighter than that on purpose.
          'bg-background-default border border-border-subtle/60',
          'transition-[box-shadow,background-color,border-color]',
          // NO FOCUS RING. THE EDGE ITSELF CARRIES THE STATE.
          //
          // This is what "the rim is still too thick" was actually about. The
          // ring was 2px and INSET, so stacked on the 1px border it painted a
          // 3px band — and because the composer AUTOFOCUSES on mount, it was on
          // from the first frame of every session. It was never a focus state;
          // it was the composer's permanent resting appearance. No amount of
          // lightening the 1px border underneath could fix that, because the
          // border was never the thick part.
          //
          // THE ORANGE RING IS BACK, BY EXPLICIT REQUEST, AND IT IS THE FOCUS
          // STATE. It was removed once — on the reasoning below — and the user
          // asked for it back after seeing the composer without it. Their call
          // stands: read the rest of this note as the constraint the ring has to
          // survive, not as an argument against it.
          //
          // The reasoning that removed it, which is still TRUE and still worth
          // knowing: the composer autofocuses, so `:focus` is on from the first
          // frame of every session, which makes anything hung on it the resting
          // appearance rather than a state. That is why the ring must stay
          // CHEAP — it is on nearly always, so it has to be something the eye
          // can live beside, not an alert. What made it read as "a very thick
          // rim" the first time was the ring at 2px stacked on a border that was
          // then at full strength: two edges, ~3px, in two different hues. The
          // border has since dropped to 60%, and everything around the composer
          // has tightened, so the same ring now reads as one accented edge
          // rather than a band.
          //
          // ⚠ DO NOT hang this on `:focus-visible` to make it "keyboard only".
          // It was tried and measured: `:focus-visible` ALWAYS matches a focused
          // text control, however focus arrived, because the element accepts
          // keyboard input. On a textarea it is not a narrower `:focus`, it is
          // the same selector spelled longer — so it changes nothing except how
          // long the next person takes to work that out.
          //
          // CSS, not a mirrored React flag. This used to ride an `isFocused`
          // state kept in sync by the textarea's own `onFocus`/`onBlur`. Asking
          // the DOM directly is one source of truth instead of two, and cannot
          // desync from it.
          // THE RING IS THE BORDER, IN FULL ACCENT — one crisp orange pixel,
          // not a pale band.
          //
          // The previous spelling was `inset-shadow-accent`, which is
          // `--inset-accent` = `inset 0 0 0 2px var(--accent-muted)`, and
          // `--accent-muted` is the accent at **8%**. Two pixels of 8% coral on
          // a white card does not read as orange at all — it reads as a thick,
          // slightly warm grey border, which is exactly how it was reported
          // ("not orange, a thick border"). The alpha was the bug, and it is
          // also why the same ring earlier read as "a very thick rim": a 2px
          // near-transparent band beside a 1px real one is two edges, and
          // neither of them looks deliberate.
          //
          // `--border-accent` is the full accent (coral-600 light, coral-400
          // dark, teal in Alma Mater), so the composer's own 1px edge simply
          // turns orange. One pixel, one edge, unmistakably the accent —
          // and it stays a border, so nothing about the box changes.
          'biorouter-composer-card',
          // Drag-over keeps the ring: THAT is a transient state worth shouting,
          // it is nothing like a resting appearance, and being inset it still
          // costs no layout.
          isDraggingOver && 'inset-shadow-accent',
          isDraggingOver && 'bg-background-medium/80'
        )}
        // THE WORKING EDGE. While a turn runs, the card's own border carries a
        // travelling segment of the accent (`main.css`, next to the focus rule)
        // — the replacement for the breathing dot that used to sit in a row
        // above this card, duplicating the transcript's indicator 8px out of
        // line with it.
        //
        // An ATTRIBUTE, not a class, and deliberately: the styling is authored
        // CSS rather than a Tailwind utility (a newly written arbitrary class
        // can silently fail to generate under `BIOROUTER_NO_HMR`), so the hook
        // it keys off has to be something Tailwind never had to produce.
        //
        // `undefined` rather than `'false'` when idle, so the attribute is
        // absent from the DOM and `[data-working]` is the whole test.
        data-working={isWorking ? 'true' : undefined}
      >
        {/* Message Queue Display */}
        {queuedMessages.length > 0 && (
          <MessageQueue
            queuedMessages={queuedMessages}
            onRemoveMessage={handleRemoveQueuedMessage}
            onClearQueue={handleClearQueue}
            onStopAndSend={handleStopAndSend}
            onSteerMessage={canSteer ? handleSteerMessage : undefined}
            onReorderMessages={handleReorderMessages}
            onEditMessage={handleEditMessage}
            onTriggerQueueProcessing={handleResumeQueue}
            editingMessageIdRef={editingMessageIdRef}
            isPaused={queuePausedRef.current}
            className="border-b border-border-subtle"
          />
        )}
        {/* No-model hint. Same shape as the vision-mismatch banner below and for
 the same reason: the composer says why Send is off, in the composer, instead
 of letting the user press it and meet a daemon error written for a developer.
 It carries the way out, because "choose a provider" is only actionable if the
 catalog is one click away. */}
        {noModelConfigured && (
          <div
            data-testid="composer-no-model-hint"
            className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-2 mb-2 bg-background-medium/60 border border-border-subtle rounded-element text-supporting text-text-muted"
          >
            <span className="leading-snug">{NO_MODEL_COMPOSER_HINT}</span>
            <button
              type="button"
              onClick={() => setView?.('ConfigureProviders')}
              data-testid="composer-no-model-action"
              className="underline underline-offset-2 transition-colors duration-150 hover:text-text-default"
            >
              {NO_MODEL_COMPOSER_ACTION}
            </button>
          </div>
        )}
        {/* Vision-mismatch banner: shown when the user has images attached but the
 current model does not support vision. Blocks Send until resolved. */}
        {visionMismatch && (
          <div className="flex items-start gap-2 px-3 py-2 mb-2 bg-background-medium/60 border border-border-subtle rounded-element text-supporting text-text-muted">
            <svg
              className="w-3.5 h-3.5 flex-shrink-0 mt-0.5 opacity-60"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth={1.5}
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              <path d="M9.88 9.88a3 3 0 1 0 4.24 4.24" />
              <path d="M10.73 5.08A10.43 10.43 0 0 1 12 5c7 0 10 7 10 7a13.16 13.16 0 0 1-1.67 2.68" />
              <path d="M6.61 6.61A13.526 13.526 0 0 0 2 12s3 7 10 7a9.74 9.74 0 0 0 5.39-1.61" />
              <line x1="2" y1="2" x2="22" y2="22" />
            </svg>
            <span className="leading-snug">
              The active model can&apos;t read images. Switch to a vision-capable model, or remove
              attached images to send.
            </span>
          </div>
        )}
        {/* Attached resources (issue #65). The canonical form of a reference is a
 `<biorouter-ref …>` tag, which is the only form that survives a name with a
 space or a quote in it — and is far too much markup to leave sitting in the
 user's sentence. It lives in the message text; this rail is where the user
 sees and manages it. Above the prose because a reference qualifies the whole
 message rather than a point in it, and because the row below the textarea is
 already the attachments area for images and files. */}
        {composerRefs.length > 0 && (
          <div
            data-testid="composer-reference-rail"
            className="mb-1.5 flex flex-wrap items-center gap-1.5 px-1"
          >
            {composerRefs.map((ref, index) => (
              <ResourceRefChip
                key={`${ref.kind}:${ref.value}`}
                refSpan={ref}
                onRemove={() => handleRemoveReference(index)}
              />
            ))}
          </div>
        )}
        {/* THE INPUT LINE: the prose, and the one action on it. Send lives
            HERE now, not in the control row — it acts on what is in this box,
            so it belongs in this box, and putting it here is also what lets the
            control row below be settings and readouts only.

            `items-end` rather than `items-center`: on one line the two children
            are the same 32px so the two agree, but on a six-line message only
            `end` keeps Send beside the last line the user typed.

            `gap-2` (8px), down from 10. The shell above claims its `gap-2.5` is
            "deliberately wider than any gap inside the card" — that is the whole
            reason the card reads as one object with two satellites rather than
            as three bands — and while this was also 10px the claim was simply
            not true, the two were equal. Tightening the one gap INSIDE the card
            is what makes it true, and it is the "squeeze the elements in a
            little" the note asked for, spent where it costs nothing: 8px between
            the prose and Send, still inside 12px to the card's edge. */}
        <div className="flex min-w-0 items-end gap-2">
          <form
            id="bior-chat-form"
            onSubmit={onFormSubmit}
            className="relative flex min-w-0 flex-1"
          >
            {/* THE TYPING REGION, and the third attempt at making the text sit
              plainly in the middle of its box. The first two put the padding in
              the wrong place; this one puts it symmetrically on the control
              itself, which is the only arrangement that survives the textarea
              growing.

              The physics, stated once so it stops being rediscovered: a
              textarea's text begins at the top of its CONTENT box. So
              bottom-only padding cannot centre anything — it grows the box
              underneath the text and leaves the line where it was (that was the
              original `p-0 pb-1.5` bug, measured at 3px high). SYMMETRIC
              padding is different: it insets the content box equally top and
              bottom, and a single line box then fills that content box exactly,
              so the line is centred in the border box by construction.

              `py-1.5` is therefore load-bearing twice over. It centres the line
              — and it makes the textarea's border box exactly 32px, the same as
              Send, so `items-end` above lands the two on the same baseline
              instead of dropping the text below the button. Remove it and the
              text does not merely lose its padding, it goes visibly low.

              ⚠ THIS NUMBER IS TIED TO THE TYPE SIZE, and that coupling is what
              made it 6px rather than 4px. The invariant is `line + 2×padding ==
              32px`, so the input's box matches Send's: at a 24px line that was
              `py-1`, and at `--text-body`'s 20px line it is `py-1.5`. When the
              input was moved back onto the shared 14/20 role, leaving `py-1`
              would have made the box 28px, and `items-end` would have hung the
              text 4px low — reintroducing the exact bias this note exists to
              prevent, while every padding number still looked correct.

              Air, measured rather than eyeballed: the 20px line box sits inside
              6px of the control's own padding inside 10px of the card's, so it
              carries 16px above and 16px below — the same number, which is what
              "not biased towards anywhere" has to mean.

              `block` is the last piece. A textarea is `inline-block` by default
              and sits on its parent's BASELINE, so a wrapping line box reserves
              descender room under it and the row the user sees is taller than
              the control — invisible to any test that measures the textarea
              rather than its wrapper. It is a flex item here, which blockifies
              it anyway, but the class states the requirement rather than
              relying on the parent keeping `display:flex` forever. */}
            <textarea
              data-testid="chat-input"
              autoFocus
              id="dynamic-textarea"
              // The navigation hint is only true once there is something to
              // navigate. On Home and in a brand-new session the app's primary
              // input used to invite nothing and explain a shortcut that did
              // nothing yet; `messagesLength` is already a prop here (#22), so the
              // placeholder can simply tell the truth in both states.
              placeholder={
                (messagesLength ?? 0) > 0 ? getNavigationShortcutText() : 'Ask Biorouter anything…'
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
              // `px-0` is deliberate and is the horizontal half of the
              // alignment invariant: the CARD's 16px inset is the composer's
              // left edge, and the placeholder has to start on it so the
              // folder glyph above and the reasoning glyph below line up with
              // the text. A horizontal inset here would be a second, private
              // declaration of the same edge, and the three would drift the
              // first time one of them changed.
              //
              // `py-1.5` is the vertical half, and the long note above the form
              // is why it is not `p-0` — and why it moves with the type size.
              className={cn(
                'block w-full resize-none border-none bg-transparent px-0 py-1.5',
                COMPOSER_INPUT_TYPE_CLASS,
                'text-text-default placeholder:text-text-muted'
              )}
            />
          </form>

          {/* SEND / STOP — inside the card, at the end of the input line.
              The row's one primary action, and never collapsed: the control row
              below can fold its pickers behind a "+" at narrow widths, but the
              way to send a message may not depend on how wide the window is.

              Both were `variant="outline"` repainted into an accent fill by a
              className override, which is the system's own primary variant
              spelled out longhand: the override could (and did) drift from
              `--background-accent-hover`, and a reader had to diff two class
              strings to learn the button was primary. `variant="default"` IS
              that fill.

              SEND IS ONLY ACCENT WHEN THERE IS SOMETHING TO SEND. At rest it is
              `secondary` — `--background-medium`, which is the user bubble's own
              fill, so the square you press is already the colour of the thing it
              is about to make.

              The note was that the composer should be "more minimalistic", and
              on an EMPTY composer a saturated accent square was the loudest
              thing on the chat surface while meaning nothing: there was no
              message, and the button was disabled anyway. Now the accent is
              information — it arrives with the first character and says "this
              will go". Nothing is removed to get it: same 32×32 box in the same
              place, same `aria-label`, same tooltip, same Enter path, and the
              button still looks like a button at rest, which is the part that
              could not be given up.

              `disabled:opacity-100` is deliberate and pairs with the above. The
              base variant fades a disabled control to 50%, which was the right
              call while disabled meant "an accent button you may not press"; a
              50% grey square on white is close to nothing, and the quiet FILL
              already carries "not yet". Cursor and tooltip still say why. */}
          {isLoading && !hasSubmittableContent ? (
            <Button
              type="button"
              onClick={() => void stopAck.trigger(false)}
              // 32×32, matching Send exactly: the two swap in place as a turn
              // starts and ends, and a 28px Stop would twitch the card's
              // height and the text's centring on each swap.
              size="default"
              shape="round"
              className={cn('relative flex-shrink-0', stopAck.acknowledged && 'scale-90')}
              data-testid="chat-stop-button"
              data-stop-acknowledged={stopAck.acknowledged}
              aria-label={stopAck.acknowledged ? 'Stopping response' : 'Stop response'}
              title={stopAck.acknowledged ? 'Stopping…' : 'Stop response'}
            >
              <Stop size={16} />
              {/* The press recoils the button and sends one ring outward from
                  it — motion that reads as "received", distinct from the
                  looping pulses that mean "still working". */}
              {stopAck.acknowledged && (
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0 rounded-element ring-2 ring-current animate-ping"
                />
              )}
            </Button>
          ) : (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex-shrink-0">
                  <Button
                    type="submit"
                    form="bior-chat-form"
                    // A 32×32 ROUNDED SQUARE, not a circle and not the 28px
                    // rung. `shape="round"` is already the square icon button
                    // (the name is historical — see button.tsx), so `default`
                    // is the whole change.
                    size="default"
                    shape="round"
                    // Quiet at rest, accent when armed. Both variants are the
                    // same 32×32 `p-0 rounded-element` box, so the swap moves no
                    // pixels — only `background-color` and `color`, both of which
                    // are already in the button's own transition list, so it
                    // eases in as you type instead of snapping.
                    variant={isSubmitButtonDisabled ? 'secondary' : 'default'}
                    disabled={isSubmitButtonDisabled}
                    aria-label="Send message"
                    className={cn(
                      isSubmitButtonDisabled && 'cursor-not-allowed disabled:opacity-100'
                    )}
                  >
                    {/* An UP arrow, not a paper plane. The gesture is "send
                        this upward into the transcript", which is where the
                        message literally goes, and the arrow says it without
                        the plane's diagonal — the only diagonal glyph in the
                        composer. */}
                    <ArrowUp className="size-4" />
                  </Button>
                </span>
              </TooltipTrigger>
              <TooltipContent>
                <p>
                  {isAnyImageLoading
                    ? 'Waiting for images to save...'
                    : isAnyDroppedFileLoading
                      ? 'Processing dropped files...'
                      : chatState === ChatState.RestartingAgent
                        ? 'Restarting chat...'
                        : 'Send'}
                </p>
                {/* BR-61's steer chord has been bound since BR-61 and was
                    advertised nowhere, so it was reachable only by someone who
                    had read the Enter handler. This is the one moment it
                    applies — a turn is in flight, so plain Enter QUEUES this
                    message — which makes the tooltip the honest place to say
                    what the other chord does, rather than a permanent hint that
                    would be wrong whenever nothing is running. */}
                {canSteer && !isAnyImageLoading && !isAnyDroppedFileLoading && (
                  <p className="text-text-muted">
                    {getSteerShortcutText()} adds it to the running turn
                  </p>
                )}
              </TooltipContent>
            </Tooltip>
          )}
        </div>

        {/* Combined files and images preview */}
        {(pastedImages.length > 0 || allDroppedFiles.length > 0) && (
          <div className="flex flex-wrap gap-2 pt-3 mt-2 border-t border-border-subtle">
            {/* Render pasted images first */}
            {pastedImages.map((img) => (
              <div key={img.id} className="relative group w-20 h-20">
                {img.dataUrl && (
                  <img
                    src={img.dataUrl}
                    alt={`Pasted image ${img.id}`}
                    className={`w-full h-full object-cover rounded-element border ${img.error ? 'border-border-danger' : 'border-border-subtle'}`}
                  />
                )}
                {img.isLoading && (
                  <div className="absolute inset-0 flex items-center justify-center bg-black bg-opacity-50 rounded-element">
                    <div className="animate-spin rounded-full h-6 w-6 border-t-2 border-b-2 border-white"></div>
                  </div>
                )}
                {img.error && !img.isLoading && (
                  <div className="absolute inset-0 flex flex-col items-center justify-center bg-black bg-opacity-75 rounded-element p-1 text-center">
                    <p className="text-text-danger text-supporting leading-tight break-all mb-1">
                      {img.error.substring(0, 50)}
                    </p>
                    {img.dataUrl && (
                      <Button
                        type="button"
                        onClick={() => handleRetryImageSave(img.id)}
                        title="Retry saving image"
                        variant="outline"
                        size="xs"
                      >
                        Retry
                      </Button>
                    )}
                  </div>
                )}
                {!img.isLoading && (
                  <Button
                    type="button"
                    shape="round"
                    onClick={() => handleRemovePastedImage(img.id)}
                    className="absolute -top-1 -right-1 opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity z-10"
                    aria-label="Remove image"
                    variant="outline"
                    size="xs"
                  >
                    <X />
                  </Button>
                )}
              </div>
            ))}

            {/* Render dropped files after pasted images */}
            {allDroppedFiles.map((file) => (
              <div key={file.id} className="relative group">
                {file.canUploadAsImage ? (
                  // Image preview
                  <div className="w-20 h-20">
                    {file.dataUrl && (
                      <img
                        src={file.dataUrl}
                        alt={file.name}
                        className={`w-full h-full object-cover rounded-element border ${file.error ? 'border-border-danger' : 'border-border-subtle'}`}
                      />
                    )}
                    {file.isLoading && (
                      <div className="absolute inset-0 flex items-center justify-center bg-black bg-opacity-50 rounded-element">
                        <div className="animate-spin rounded-full h-6 w-6 border-t-2 border-b-2 border-white"></div>
                      </div>
                    )}
                    {file.error && !file.isLoading && (
                      <div className="absolute inset-0 flex flex-col items-center justify-center bg-black bg-opacity-75 rounded-element p-1 text-center">
                        <p className="text-text-danger text-supporting leading-tight break-all">
                          {file.error.substring(0, 30)}
                        </p>
                      </div>
                    )}
                  </div>
                ) : (
                  // File box preview.
                  //
                  // The error branch is not decoration. A file that could not be
                  // located is still attached and still looks ordinary, so
                  // without it the user sends a message believing a file went
                  // with it. Only the image branch rendered `error` before, so a
                  // dropped document failed completely silently.
                  <div
                    className={`flex items-center gap-2 px-3 py-2 bg-background-medium border rounded-element min-w-[120px] ${
                      // An error needs room to be read. Clamping it to the
                      // normal chip width hid all but two lines behind a
                      // `title` tooltip, which is not a place a person looks
                      // when they think their file attached fine.
                      file.error
                        ? 'border-border-danger max-w-[420px]'
                        : 'border-border-subtle max-w-[200px]'
                    }`}
                  >
                    <div className="flex-shrink-0 w-8 h-8 bg-background-default border border-border-subtle rounded-inner flex items-center justify-center text-supporting font-mono text-text-muted">
                      {file.name.split('.').pop()?.toUpperCase() || 'FILE'}
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-secondary text-text-default truncate" title={file.name}>
                        {file.name}
                      </p>
                      {file.error ? (
                        <p className="text-supporting text-text-danger">{file.error}</p>
                      ) : (
                        <p className="text-supporting text-text-muted">
                          {file.type || 'Unknown type'}
                        </p>
                      )}
                    </div>
                  </div>
                )}
                {!file.isLoading && (
                  <Button
                    type="button"
                    shape="round"
                    onClick={() => handleRemoveDroppedFile(file.id)}
                    className="absolute -top-1 -right-1 opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity z-10"
                    aria-label="Remove file"
                    variant="outline"
                    size="xs"
                  >
                    <X />
                  </Button>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* ROW 3 — CONTROLS, ON THE CANVAS. Settings and readouts only: Send has
          moved into the card, where the thing it acts on is.

          SPLIT IN TWO. Left is what the next message is answered BY — the
          reasoning knob and the model, one dot between them because they
          answer the same question. Right is what it is COSTING — headroom and
          money — which are readouts, not pickers, and read as such once they
          are the only things on that side.

          `pl-3.5` is solved for, like row 1's `pl-3`: 14px plus the reasoning
          chip's own 2px puts its glyph on the same column as the folder glyph
          above and the placeholder in between. See row 1 for the measurement
          that rejected replacing both with a flat 4px inset. No vertical
          padding at all — the shell's `gap-1.5` is the whole distance to the
          card, declared once, which is the mistake that produced a lopsided
          card an earlier round. */}
      <div
        ref={toolbarRef}
        data-testid="chat-input-toolbar"
        className="relative flex min-w-0 flex-row flex-nowrap items-center overflow-hidden pr-2.5 pl-3.5"
      >
        {/* The controls are hoisted to the component body, because row 1 needs
            the same objects this row does. What is left here is only the
            ARRANGEMENT — and it is a much shorter row than it was: the
            directory and the context group live in row 1 and Send lives in the
            card, so this row carries four things. That is also why the collapse
            threshold is now rarely reached.

            GAPS ARE OPTICAL, not literal. The design's 10px on the left group
            and 13px on the right are measured between bare spans; every control
            here carries 2–6px of its own hit-area padding, so the flex gap is
            set to leave the intended visible channel. Left: `gap-3.5` plus the
            chips' 2px lands the dot 16px clear on both sides — wider than the
            design's 10, deliberately, because "rammed" has been the standing
            note and this row has room to spare. (The gap is between BORDER
            boxes, so swapping the 1px rule for a 3px dot did not move those
            16s.) Right: `gap-1.5` plus the gauge's 6px and the cost's 2px is
            14px, which is the design's 13 within a pixel. */}
        {composerToolbarCollapsed ? (
          <div className="flex min-w-0 flex-1 items-center gap-3.5 overflow-hidden">
            <Popover>
              <PopoverTrigger asChild>
                <button
                  type="button"
                  aria-label="Chat tools and settings"
                  title="Chat tools and settings"
                  data-testid="composer-tools-collapsed"
                  // 28px, matching `h-7` on every other control in this row, so
                  // the row is exactly as tall collapsed as expanded and the
                  // composer does not resize when the artifact panel opens —
                  // only its contents change. This was 32px, which was harmless
                  // while the row carried 10px of its own vertical padding to
                  // absorb the difference; the row has none now (the shell's
                  // `gap-3` is the whole distance to the card), so the odd rung
                  // would move the card by 4px on every collapse.
                  className="flex size-7 flex-shrink-0 items-center justify-center rounded-element text-text-muted tint-interactive transition-colors hover:text-text-default"
                >
                  <Plus className="size-4" />
                </button>
              </PopoverTrigger>
              {/* Portaled, so the enclosing overflow-hidden never clips it.
                  side/align place it above the "+", growing up out of the row. */}
              <PopoverContent
                side="top"
                align="start"
                // The standard §3.8 menu container: 4px padding, 2px between
                // items.
                //
                // ⚠ **The width follows the CONTENT, and the floor is a floor,
                // not a size.** This carried `min-w-[15rem]` (240px) while the
                // three readouts inside measure 167px, so every collapsed
                // composer drew a box 60px wider than anything in it, and wider
                // again when cost reads "$0.00" rather than "Unavailable".
                // Three small readouts adrift in a 240px box read as a menu
                // that failed to load its items.
                //
                // 8rem is a floor against a comically narrow box when a readout
                // is hidden, not a target. It was measured against the row's
                // narrowest real state, not guessed.
                className="flex w-auto min-w-[8rem] max-w-[80vw] flex-col gap-0.5 p-1"
                data-testid="composer-tools-popover"
              >
                {/* The directory, extensions, skills and knowledge bases are NOT
                    here any more — they live in row 1, which has its own line
                    and therefore never runs out of room. Collapsing is now only
                    ever about this row. */}
                <div className="flex flex-wrap items-center gap-0.5">
                  {reasoning}
                  {contextGauge}
                  {cost}
                </div>
              </PopoverContent>
            </Popover>
            {/* The dot survives the collapse, because the "+" has taken the
                reasoning knob's place in the row and the seam it marks — the
                settings on one side, the model on the other — has not moved. */}
            <span aria-hidden="true" className={TOOLBAR_DIVIDER_CLASS} />
            {/* The model NEVER collapses. It changes what the next message costs
                and what it can do, so hiding it behind a disclosure at exactly
                the width where the artifact panel is competing for attention is
                when it matters most. */}
            {model}
          </div>
        ) : (
          <div className="flex min-w-0 flex-1 flex-row items-center gap-3.5 overflow-hidden">
            {reasoning}
            <span aria-hidden="true" className={TOOLBAR_DIVIDER_CLASS} />
            {model}
          </div>
        )}

        {/* THE READOUTS, right-aligned: how much headroom is left and what the
            conversation has cost. Both are things to READ, not controls to set,
            and with Send gone into the card they are finally the only things on
            this side — which is what lets them read that way instead of as two
            more pickers.

            Hidden while collapsed, because they are in the "+" popover. That
            leaves the row with nothing on the right at narrow widths, and
            `ml-auto` on a group that is not rendered costs nothing. */}
        {!composerToolbarCollapsed && (
          <div className="ml-auto flex flex-shrink-0 items-center gap-1.5 pl-2">
            {contextGauge}
            {cost}
          </div>
        )}
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
    </div>
  );
}
