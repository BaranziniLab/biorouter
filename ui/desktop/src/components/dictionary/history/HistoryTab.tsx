import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { DictationHistoryEntryView } from '../../../api';
import { toastError, toastSuccess } from '../../../toasts';
import { isBrowserSurface } from '../../../utils/surface';
import { ReadableContent } from '../../Layout/ReadableContent';
import { dictationMessage } from '../../dictation/copy';
import { remoteOldValues } from '../../dictation/dictationStore';
import { Mic } from '../../icons/app-icons';
import { PrivacyTiersOffNote } from '../../privacy/PrivacyTiersOffNote';
import { Button } from '../../ui/button';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { EmptyState } from '../../ui/empty-state';
import { Note } from '../../ui/note';
import { ScrollArea } from '../../ui/scroll-area';
import { Skeleton } from '../../ui/skeleton';
import { DICTIONARY_LABELS, dictionaryMessage } from '../copy';
import { useHoverHold } from '../hoverHold';
import type { TermDialogMode } from '../TermDialog';
import { historyEntryOrigin } from '../termOrigin';
import {
  deleteHistoryEntry,
  loadMoreHistory,
  setHistorySettings,
  setHistoryTier,
  type HistoryHook,
} from '../useDictationHistory';
import HistoryList from './HistoryList';
import type { HistoryRowActions } from './HistoryRow';
import {
  groupDictationsByDate,
  historyPublicizeWarning,
  turnOnGate,
  type HistorySettledKind,
} from './historyRows';

const LABELS = DICTIONARY_LABELS.history;

type Confirm =
  | { kind: 'publicize'; entry: DictationHistoryEntryView }
  | { kind: 'delete'; entry: DictationHistoryEntryView };

const keyOfEntry = (entry: DictationHistoryEntryView) => entry.id;

/** The words on a row of `entry` after the change, until the pointer leaves the list. */
function settledFor(confirm: Confirm): HistorySettledKind {
  return confirm.kind === 'delete' ? 'deleted' : 'madePublic';
}

async function copyText(entry: DictationHistoryEntryView): Promise<void> {
  try {
    await navigator.clipboard.writeText(entry.text);
    toastSuccess({ title: LABELS.copied });
  } catch {
    toastError({ title: LABELS.copy, msg: dictationMessage('MSG-SETTINGS-UNREACHABLE') });
  }
}

export interface HistoryTabProps {
  history: HistoryHook;
  /**
   * Managed `dictation.history`, from the window's dictation status: `false` stops recording and
   * tier changes, and the list stays, with delete and clear still allowed.
   */
  policy: { allowed: boolean; reason?: string | null } | null;
  /** The daemon's host, for MSG-HISTORY-PRIVATE-ELSEWHERE when its own sentence is missing. */
  host?: string | null;
  /** Whether the caller may write the dictionary: **Add to dictionary…** shows only then. */
  canAddToDictionary: boolean;
  /** Open the term dialog with a word taken from a dictation. */
  onAddToDictionary: (mode: Extract<TermDialogMode, { kind: 'add' }>) => void;
}

/**
 * The Dictionary page's **History** tab (DD-96; `history-design.md` 10.2 to 10.6): the
 * dictations this caller may list under the shared chat date buckets, flat rows at one pitch,
 * **Copy** on hover and a `⋯` menu, a click that expands a row in place, and every empty and
 * special state of 10.4. The band (search, the settings link and **Clear history…**) is the
 * page's, in `DictionaryView`, which reads the same store.
 *
 * No row moves under a hovering pointer (`hoverHold.ts`): a new dictation waits until the pointer
 * leaves, while the next page read at the end of the list is let through. A row the person acts
 * on meanwhile stays dimmed and says "Deleted" or "Made public" in place.
 */
export default function HistoryTab({
  history,
  policy,
  host,
  canAddToDictionary,
  onAddToDictionary,
}: HistoryTabProps) {
  const navigate = useNavigate();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [busy, setBusy] = useState(false);
  const [turningOn, setTurningOn] = useState(false);

  const hold = useHoverHold<DictationHistoryEntryView, HistorySettledKind>(history.entries, {
    keyOf: keyOfEntry,
  });
  const { settle } = hold;
  // One clock for every row of one render, so the rows' memo holds between renders.
  const { now, groups } = useMemo(() => {
    const at = Date.now();
    return { now: at, groups: groupDictationsByDate(hold.shown, at) };
  }, [hold.shown]);

  const settings = history.settings;
  const authority = settings?.authority ?? null;
  const policyOff = policy?.allowed === false;
  const writeAllowed = !history.absent && (authority?.write.allowed ?? !isBrowserSurface());
  const tierChangeAllowed =
    !history.absent && !policyOff && (authority?.tier_change.allowed ?? !isBrowserSurface());
  const tierReason = policyOff
    ? (policy?.reason ?? dictationMessage('MSG-POLICY'))
    : (authority?.tier_change.reason ?? null);
  const historyOff = settings?.keep_history === false;
  const turnOn = turnOnGate(settings);
  const listsPrivate = authority?.list_private.allowed !== false;

  const navigateRef = useRef(navigate);
  const addRef = useRef(onAddToDictionary);
  useEffect(() => {
    navigateRef.current = navigate;
    addRef.current = onAddToDictionary;
  }, [navigate, onAddToDictionary]);

  const actions = useMemo<HistoryRowActions>(
    () => ({
      onCopy: (entry) => void copyText(entry),
      ...(canAddToDictionary
        ? {
            onAddToDictionary: (entry: DictationHistoryEntryView, selection: string) =>
              addRef.current({
                kind: 'add',
                text: selection,
                origin: historyEntryOrigin({
                  tier: entry.tier,
                  sessionId: entry.session_id,
                  surface: entry.surface,
                }),
                startPrivate: entry.tier === 'private',
              }),
          }
        : {}),
      onOpenChat: (entry) =>
        navigateRef.current(`/pair?resumeSessionId=${encodeURIComponent(entry.session_id ?? '')}`),
      onMakePublic: (entry) => setConfirm({ kind: 'publicize', entry }),
      onMakePrivate: (entry) => {
        // Making a dictation private only narrows who may read it: one step, no question.
        void (async () => {
          const outcome = await setHistoryTier(entry.id, 'private');
          if (outcome.ok) settle(entry.id, 'madePrivate');
          else toastError({ title: LABELS.makePrivate, msg: outcome.error.message });
        })();
      },
      onDelete: (entry) => setConfirm({ kind: 'delete', entry }),
      onToggle: (id) =>
        setExpanded((current) => {
          const next = new Set(current);
          if (next.has(id)) next.delete(id);
          else next.add(id);
          return next;
        }),
    }),
    [canAddToDictionary, settle]
  );

  const confirmAction = async () => {
    if (!confirm) return;
    const { entry } = confirm;
    setBusy(true);
    try {
      const outcome =
        confirm.kind === 'delete'
          ? await deleteHistoryEntry(entry.id)
          : await setHistoryTier(entry.id, 'public');
      if (outcome.ok) settle(entry.id, settledFor(confirm));
      else {
        toastError({
          title: confirm.kind === 'delete' ? LABELS.deleteOne.title : LABELS.publicize.title,
          msg: outcome.error.message,
        });
      }
    } finally {
      setBusy(false);
      setConfirm(null);
    }
  };

  const turnHistoryOn = async () => {
    setTurningOn(true);
    try {
      const outcome = await setHistorySettings({ keep_history: true });
      if (!outcome.ok) toastError({ title: LABELS.off, msg: outcome.error.message });
    } finally {
      setTurningOn(false);
    }
  };

  const turnOnButton = (variant: 'secondary' | 'ghost') =>
    policyOff ? null : (
      <Button
        type="button"
        size="sm"
        variant={variant}
        disabled={turnOn.disabled || turningOn}
        onClick={() => void turnHistoryOn()}
        data-testid="history-turn-on"
      >
        {LABELS.turnOn}
      </Button>
    );

  const loaded = history.load === 'loaded';
  const shownTotal = hold.shown.length;
  const firstLoad = !loaded && shownTotal === 0 && !history.error;
  const failure = history.error;
  const notice = history.absent
    ? dictationMessage('MSG-REMOTE-OLD', remoteOldValues())
    : (failure?.message ?? null);

  return (
    <>
      <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6">
        <ScrollArea className="h-full">
          <div className="relative py-4" data-testid="history-tab">
            <PrivacyTiersOffNote className="mb-4" />
            {policyOff ? (
              <Note tone="warning" role="alert" className="mb-4" testId="history-policy">
                {policy?.reason ?? dictationMessage('MSG-POLICY')}
              </Note>
            ) : null}
            {notice ? (
              <Note
                tone={history.absent ? 'warning' : 'danger'}
                role="alert"
                className="mb-4"
                testId="history-notice"
              >
                {notice}
              </Note>
            ) : null}
            {historyOff && !policyOff && shownTotal > 0 ? (
              <Note tone="neutral" role="status" className="mb-4" testId="history-off-note">
                <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                  <p className="min-w-0 flex-1">{dictionaryMessage('MSG-HISTORY-OFF')}</p>
                  {turnOnButton('secondary')}
                </div>
                {turnOn.reason ? (
                  <p
                    className="mt-1 text-supporting text-text-muted"
                    data-testid="history-turn-on-reason"
                  >
                    {turnOn.reason}
                  </p>
                ) : null}
              </Note>
            ) : null}
            {/* Said on every surface that cannot list private dictations, whether or not any
                exist, so it tells nothing about the history. */}
            {settings && !listsPrivate ? (
              <p
                className="mb-4 px-3 text-supporting text-text-muted"
                data-testid="history-private-elsewhere"
              >
                {authority?.list_private.reason?.trim() ||
                  dictionaryMessage('MSG-HISTORY-PRIVATE-ELSEWHERE', host ? { host } : {})}
              </p>
            ) : null}

            {firstLoad ? (
              <div
                className="biorouter-list-shell br-rows-flat"
                aria-hidden
                data-testid="history-skeleton"
              >
                <HistoryRowSkeleton />
                <HistoryRowSkeleton />
                <HistoryRowSkeleton />
              </div>
            ) : null}

            {shownTotal > 0 ? (
              <HistoryList
                groups={groups}
                settled={hold.settled}
                expanded={expanded}
                writeAllowed={writeAllowed}
                tierChangeAllowed={tierChangeAllowed}
                tierChangeReason={tierReason}
                actions={actions}
                now={now}
                onNearEnd={history.nextCursor ? loadMoreHistory : undefined}
                holding={hold.holding}
                handlers={hold.handlers}
              />
            ) : null}

            {loaded && shownTotal === 0 && !history.absent && !failure ? (
              history.query ? (
                <EmptyState
                  icon={Mic}
                  title={LABELS.noMatches}
                  description={dictionaryMessage('MSG-HISTORY-NO-MATCH')}
                  compact
                />
              ) : policyOff ? null : historyOff ? (
                <div data-testid="history-empty-off">
                  <EmptyState
                    icon={Mic}
                    title={LABELS.off}
                    description={dictionaryMessage('MSG-HISTORY-OFF')}
                    actions={turnOnButton('secondary')}
                  />
                  {turnOn.reason ? (
                    <p
                      className="-mt-8 text-center text-supporting text-text-muted"
                      data-testid="history-turn-on-reason"
                    >
                      {turnOn.reason}
                    </p>
                  ) : null}
                </div>
              ) : (
                <div data-testid="history-empty">
                  <EmptyState
                    icon={Mic}
                    title={LABELS.empty}
                    description={dictionaryMessage('MSG-HISTORY-EMPTY')}
                  />
                </div>
              )
            ) : null}
          </div>
        </ScrollArea>
      </ReadableContent>

      <ConfirmationModal
        isOpen={confirm !== null}
        title={confirm?.kind === 'delete' ? LABELS.deleteOne.title : LABELS.publicize.title}
        message={
          confirm?.kind === 'delete'
            ? dictionaryMessage('MSG-HISTORY-DELETE')
            : confirm
              ? dictionaryMessage(historyPublicizeWarning(confirm.entry))
              : ''
        }
        confirmLabel={
          confirm?.kind === 'delete' ? LABELS.deleteOne.confirm : LABELS.publicize.confirm
        }
        cancelLabel={DICTIONARY_LABELS.cancel}
        confirmVariant={confirm?.kind === 'delete' ? 'destructive' : 'default'}
        isSubmitting={busy}
        onConfirm={() => void confirmAction()}
        onCancel={() => setConfirm(null)}
      />
    </>
  );
}

function HistoryRowSkeleton() {
  return (
    <div className="biorouter-list-row flex items-center gap-3 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-full max-w-md" />
        <Skeleton className="mt-1 h-4 w-64" />
        <Skeleton className="mt-1 h-3 w-40" />
      </div>
    </div>
  );
}
