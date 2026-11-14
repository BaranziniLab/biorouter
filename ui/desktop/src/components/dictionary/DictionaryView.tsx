import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { DictionaryEntryView } from '../../api';
import { toastError, toastSuccess } from '../../toasts';
import { savedFileParts } from '../../utils/exportConversation';
import { isBrowserSurface } from '../../utils/surface';
import { ModalShell } from '../ModalShell';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { PageHeader, PageHeaderMenu } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { DICTATION_ACTIONS, dictationMessage } from '../dictation/copy';
import { noteDictationInterest, useDictationStatus } from '../dictation/dictationStore';
import { ChevronDown, Plus } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { DangerousConfirmDialog } from '../ui/DangerousConfirmDialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { EmptyState } from '../ui/empty-state';
import { FilterInput } from '../ui/filter-input';
import { InfoTip, useInfoTipId } from '../ui/info-tip';
import { Note } from '../ui/note';
import { RowActionMenuItems, type RowActionItem } from '../ui/row-actions';
import { ScrollArea } from '../ui/scroll-area';
import { Skeleton } from '../ui/skeleton';
import AddTermsDialog, { type AddTermsSource } from './AddTermsDialog';
import { DICTIONARY_EXPORT_FILE, DICTIONARY_LABELS, dictionaryMessage } from './copy';
import DictionaryList from './DictionaryList';
import type { DictionaryRowActions } from './DictionaryRow';
import ReviewSuggestionsDialog from './ReviewSuggestionsDialog';
import { useHoverHold } from './hoverHold';
import { fileOverImportBytes } from './parseTermList';
import { groupEntries, publicizeWarning, reviewRows, type SettledKind } from './rows';
import TermDialog, { type TermDialogMode } from './TermDialog';
import {
  acceptTerms,
  deleteTerm,
  dismissTerms,
  keepTerms,
  learnFromChatsGate,
  useDictionary,
  type Outcome,
} from './useDictionary';
import { scanLine, startScan, stopScan, useSuggestionScan } from './useSuggestionScan';
import './dictionary.css';

type Confirm =
  | { kind: 'delete'; entry: DictionaryEntryView }
  | { kind: 'publicize'; entry: DictionaryEntryView }
  | { kind: 'privatize'; entry: DictionaryEntryView };

/** Run a row's write; say what it did in place, or why it failed. */
async function actOnRow(
  entry: DictionaryEntryView,
  kind: SettledKind,
  run: () => Promise<Outcome<unknown>>,
  settle: (id: string, kind: SettledKind) => void
): Promise<void> {
  const outcome = await run();
  if (outcome.ok) settle(entry.id, kind);
  else toastError({ title: entry.written, msg: outcome.error.message });
}

async function copyTerm(entry: DictionaryEntryView): Promise<void> {
  try {
    await navigator.clipboard.writeText(entry.written);
    toastSuccess({ title: DICTIONARY_LABELS.copied, msg: entry.written });
  } catch {
    toastError({ title: entry.written, msg: dictationMessage('MSG-SETTINGS-UNREACHABLE') });
  }
}

/**
 * **Dictionary** (DD-67, R4; "The left sidebar Dictionary page" in
 * `docs/dictation/dictionary.md`): the words dictation should spell the
 * person's way, with the add dialogs, import and export, the review dialog and
 * the row actions. The Skills page is the template: the band holds the title,
 * the count, the help, **Filter terms** (⌘F), a ghost **Export…**, the one
 * primary **Add** menu and a `⋯` with **Delete all terms…**; the rows are flat
 * list rows in groups, drawn only near the viewport (`DictionaryList`), so the
 * page keeps up at the store's 5,000 row cap. The count in the band is the rows
 * the page shows, which the hover hold keeps as they were while the pointer
 * rests on the list.
 *
 * In a served browser, the daemon's authority fields decide which writes and
 * private term actions are available. Managed `dictionary: false` shows MSG-POLICY and the list read
 * only, with **Export…** and deleting still allowed. The settings area at the
 * page's foot holds the controls that work end to end (DD-94): **Agent's terms**
 * and **Suggest dictionary words from my chats** for the `dictionary` capability
 * (PR-23), and **Suggest words from my messages** for the chat extractor (PR-24,
 * owner 2026-11-13), with **Find words in my recent chats** under it while it is
 * on. When the person's own Keep and Dismiss counts show most suggestions were
 * dismissed, one note at the top of the list offers to turn suggestions off.
 */
export default function DictionaryView() {
  const dictionary = useDictionary();
  const navigate = useNavigate();
  const { status } = useDictationStatus();
  const [query, setQuery] = useState('');
  const [termMode, setTermMode] = useState<TermDialogMode | null>(null);
  const [termKey, setTermKey] = useState(0);
  const [addTerms, setAddTerms] = useState<AddTermsSource | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [deleteAllOpen, setDeleteAllOpen] = useState(false);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const learnHelpId = useInfoTipId();
  const learnReasonId = useId();
  const [answeringOffer, setAnsweringOffer] = useState(false);

  // The page mounting is a sign dictation may be used: the status store loads and the
  // device list warms ("The status store" in `docs/dictation/surfaces.md`).
  useEffect(() => noteDictationInterest(), []);

  const hold = useHoverHold(dictionary.entries);
  const { settle } = hold;
  const groups = useMemo(() => groupEntries(hold.shown, query), [hold.shown, query]);
  const waitingRows = useMemo(() => reviewRows(dictionary.entries), [dictionary.entries]);

  const authority = dictionary.settings?.authority ?? null;
  const canRaiseAgentSettings = authority?.raise_settings.allowed === true;
  const policyOff = status?.policy.dictionary.allowed === false;
  const writeAllowed = authority?.write.allowed ?? !isBrowserSurface();
  const canWrite = !dictionary.absent && writeAllowed && !policyOff;
  const deleteAllowed =
    authority?.delete?.allowed ?? (!isBrowserSurface() && (authority?.write.allowed ?? true));
  const canDelete = !dictionary.absent && deleteAllowed;
  const tierChangeAllowed = canWrite && (authority?.tier_change.allowed ?? !isBrowserSurface());
  const tierChangeReason = authority?.tier_change.reason ?? null;
  const learnGate = learnFromChatsGate(dictionary.settings);
  // Find words in my recent chats: while suggestions are on, for a caller that may write.
  const canScan =
    canWrite && dictionary.settings?.learn_from_chats === true && authority?.write.allowed === true;
  const offerTurningOff =
    !dictionary.absent &&
    !policyOff &&
    !!authority &&
    dictionary.settings?.learn_from_chats === true &&
    dictionary.settings.suggestions?.offer_turning_off === true;
  // What can be acted on is what the daemon holds now; what the page counts is what it shows,
  // which the hover hold keeps as it was while the pointer rests on the list.
  const total = dictionary.entries.length;
  const shownTotal = hold.shown.length;
  const firstLoad = dictionary.load !== 'loaded' && total === 0 && !dictionary.error;

  const openAdd = () => {
    setTermMode({ kind: 'add' });
    setTermKey((key) => key + 1);
  };

  const navigateRef = useRef(navigate);
  useEffect(() => {
    navigateRef.current = navigate;
  }, [navigate]);

  // One object for every row, whose callbacks keep their identity: a drawn row renders again
  // only when its own entry or state changes, never because a dialog opened.
  const rowActions = useMemo<DictionaryRowActions>(() => {
    const act = (
      entry: DictionaryEntryView,
      kind: SettledKind,
      run: () => Promise<Outcome<unknown>>
    ) => void actOnRow(entry, kind, run, settle);
    return {
      onEdit: (entry) => {
        setTermMode({ kind: 'edit', entry });
        setTermKey((key) => key + 1);
      },
      onCopy: (entry) => void copyTerm(entry),
      onOpenChat: (entry) =>
        navigateRef.current(
          `/pair?resumeSessionId=${encodeURIComponent(entry.suggested_in ?? '')}`
        ),
      onMakePublic: (entry) => setConfirm({ kind: 'publicize', entry }),
      onMakePrivate: (entry) => setConfirm({ kind: 'privatize', entry }),
      onDelete: (entry) => setConfirm({ kind: 'delete', entry }),
      onAccept: (entry) => act(entry, 'accepted', () => acceptTerms([entry.id])),
      onDismiss: (entry) => act(entry, 'dismissed', () => dismissTerms([entry.id])),
      onKeep: (entry) => act(entry, 'kept', () => keepTerms([entry.id])),
      onRemove: (entry) => act(entry, 'removed', () => deleteTerm(entry.id)),
    };
  }, [settle]);
  const openReview = useCallback(() => setReviewing(true), []);

  const importFile = async (file: File | undefined | null) => {
    if (!file) return;
    // Its size is in bytes already, so a file over the daemon's limit is never read.
    if (fileOverImportBytes(file)) {
      toastError({
        title: DICTIONARY_LABELS.importFile,
        msg: dictionaryMessage('MSG-DICTIONARY-TOO-LARGE'),
      });
      return;
    }
    try {
      setAddTerms({ content: await file.text(), fileName: file.name });
    } catch {
      toastError({
        title: DICTIONARY_LABELS.importFile,
        msg: dictionaryMessage('MSG-IMPORT-ROW-UNREADABLE'),
      });
    }
  };

  const runExport = async (includePrivate: boolean) => {
    setExportOpen(false);
    const outcome = await dictionary.exportFile(includePrivate);
    if (!outcome.ok) {
      toastError({ title: DICTIONARY_LABELS.exportDialog.failed, msg: outcome.error.message });
      return;
    }
    await saveExportFile(outcome.data);
  };

  const startExport = () => {
    // Private rows are listed only to a caller with private standing, so their
    // presence is the standing the confirmation is for.
    if (dictionary.entries.some((entry) => entry.tier === 'private')) setExportOpen(true);
    else void runExport(false);
  };

  const confirmAction = async () => {
    if (!confirm) return;
    const { entry } = confirm;
    setBusy(true);
    try {
      if (confirm.kind === 'delete') {
        await actOnRow(entry, 'deleted', () => deleteTerm(entry.id), settle);
      } else {
        const tier = confirm.kind === 'publicize' ? 'public' : 'private';
        const outcome = await dictionary.setTier(entry.id, tier);
        if (!outcome.ok) toastError({ title: entry.written, msg: outcome.error.message });
      }
    } finally {
      setBusy(false);
      setConfirm(null);
    }
  };

  /** The offer's two answers, through the settings write; a refusal says why. */
  const answerOffer = async (turnOff: boolean) => {
    setAnsweringOffer(true);
    try {
      const outcome = await (turnOff
        ? dictionary.setLearnFromChats(false)
        : dictionary.keepSuggesting());
      if (!outcome.ok) toastError({ title: DICTIONARY_LABELS.title, msg: outcome.error.message });
    } finally {
      setAnsweringOffer(false);
    }
  };

  const deleteAll = async () => {
    setBusy(true);
    try {
      const outcome = await dictionary.removeMany(dictionary.entries.map((entry) => entry.id));
      if (!outcome.ok) {
        toastError({ title: DICTIONARY_LABELS.deleteAll, msg: outcome.error.message });
      }
    } finally {
      setBusy(false);
      setDeleteAllOpen(false);
    }
  };

  const addItems: RowActionItem[] = [
    { label: DICTIONARY_LABELS.addTerm, onSelect: openAdd, testId: 'dictionary-add-term' },
    {
      label: DICTIONARY_LABELS.pasteList,
      onSelect: () => setAddTerms({ content: '', fileName: null }),
      testId: 'dictionary-paste-list',
    },
    {
      label: DICTIONARY_LABELS.importFile,
      onSelect: () => fileRef.current?.click(),
      testId: 'dictionary-import-file',
    },
  ];

  // The band's Add is the page's one accent; the empty state repeats the menu in the
  // secondary skin, so two accents never show at once.
  const addMenu = (variant: 'default' | 'secondary') =>
    canWrite ? (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant={variant} data-testid="dictionary-add">
            <Plus aria-hidden />
            {DICTIONARY_LABELS.add}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align={variant === 'default' ? 'end' : 'center'}>
          <RowActionMenuItems items={addItems} />
        </DropdownMenuContent>
      </DropdownMenu>
    ) : null;

  const notice = dictionary.absent
    ? dictionary.error
    : policyOff
      ? (status?.policy.dictionary.reason ?? dictationMessage('MSG-POLICY'))
      : dictionary.error;

  return (
    <MainPanelLayout removeTopPadding>
      <div className="flex min-h-0 flex-1 flex-col" data-testid="dictionary-view">
        {/* The band sits outside the scroller, as Skills' does: it is the window's drag
            region on this route, and the filter and Add go with it otherwise. */}
        <PageHeader
          title={DICTIONARY_LABELS.title}
          info={dictionaryMessage('MSG-DICTIONARY-HELP')}
          adornment={shownTotal > 0 ? shownTotal : undefined}
          actions={
            <>
              <FilterInput
                value={query}
                onValueChange={setQuery}
                label={DICTIONARY_LABELS.filter}
              />
              <Button
                variant="ghost"
                onClick={startExport}
                disabled={dictionary.absent || total === 0}
                data-testid="dictionary-export"
              >
                {DICTIONARY_LABELS.export}
              </Button>
              {addMenu('default')}
              <PageHeaderMenu
                label={DICTIONARY_LABELS.pageMenu}
                items={[
                  {
                    label: DICTIONARY_LABELS.deleteAll,
                    onSelect: () => setDeleteAllOpen(true),
                    destructive: true,
                    disabled: !canDelete || total === 0,
                    testId: 'dictionary-delete-all',
                  },
                ]}
              />
            </>
          }
        />
        <input
          ref={fileRef}
          type="file"
          accept=".txt,.csv,.tsv,.json,text/plain,text/csv,text/tab-separated-values,application/json"
          hidden
          onChange={(event) => {
            void importFile(event.target.files?.[0]);
            event.target.value = '';
          }}
          data-testid="dictionary-import-input"
        />

        <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6">
          <ScrollArea className="h-full">
            <div className="relative py-4">
              {notice ? (
                <Note
                  tone={dictionary.absent || policyOff ? 'warning' : 'danger'}
                  role="alert"
                  className="mb-4"
                  testId="dictionary-notice"
                >
                  {notice}
                </Note>
              ) : null}

              {offerTurningOff ? (
                <Note tone="neutral" role="status" className="mb-4" testId="dictionary-back-off">
                  {/* One line and its actions (owner rule, 2026-11-13: no walls of text); how
                      many chats a word now needs is the InfoTip's. */}
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                    <p className="flex min-w-0 flex-1 items-center gap-1">
                      <span data-testid="dictionary-back-off-line">
                        {dictionaryMessage('MSG-SUGGESTIONS-BACK-OFF')}
                      </span>
                      <InfoTip
                        label={DICTIONARY_LABELS.backOff.why}
                        help={dictionaryMessage('MSG-SUGGESTIONS-BACK-OFF-HELP', {
                          n: String(dictionary.settings?.suggestions.required_chats ?? 2),
                        })}
                        data-testid="dictionary-back-off-help"
                      />
                    </p>
                    <div className="flex flex-wrap gap-2">
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        disabled={answeringOffer}
                        onClick={() => void answerOffer(true)}
                        data-testid="dictionary-back-off-turn-off"
                      >
                        {DICTIONARY_LABELS.backOff.turnOff}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        disabled={answeringOffer}
                        onClick={() => void answerOffer(false)}
                        data-testid="dictionary-back-off-keep"
                      >
                        {DICTIONARY_LABELS.backOff.keepSuggesting}
                      </Button>
                    </div>
                  </div>
                </Note>
              ) : null}

              {firstLoad ? (
                <div className="biorouter-list-shell br-rows-flat" aria-hidden>
                  <TermRowSkeleton />
                  <TermRowSkeleton />
                  <TermRowSkeleton />
                </div>
              ) : null}

              <DictionaryList
                groups={groups}
                settled={hold.settled}
                readOnly={!canWrite}
                deleteAllowed={canDelete}
                tierChangeAllowed={tierChangeAllowed}
                tierChangeReason={tierChangeReason}
                actions={rowActions}
                onReviewAll={canWrite && waitingRows.length > 0 ? openReview : undefined}
                holding={hold.holding}
                handlers={hold.handlers}
              />

              {dictionary.load === 'loaded' && shownTotal === 0 && !dictionary.absent ? (
                <EmptyState
                  icon={ENTITY_ICONS.dictionary}
                  title={DICTIONARY_LABELS.title}
                  description={dictionaryMessage('MSG-EMPTY')}
                  actions={addMenu('secondary')}
                />
              ) : null}
              {shownTotal > 0 && query.trim() && groups[0]?.entries.length === 0 ? (
                <EmptyState
                  icon={ENTITY_ICONS.dictionary}
                  title={DICTIONARY_LABELS.noMatches}
                  description={dictionaryMessage('MSG-DICTIONARY-NO-MATCH')}
                  compact
                />
              ) : null}
            </div>
          </ScrollArea>
        </ReadableContent>
        {/* The settings sit in the list's own column (the same measure and gutter), so their
            rule spans the rows above edge to edge, and the same 12px inset as a group heading
            and a row's term puts their first word on the list's text edge. */}
        {dictionary.settings && !dictionary.absent && !policyOff ? (
          <ReadableContent size="chat" className="px-6">
            <div
              className="flex flex-col gap-3 border-t px-3 py-3"
              data-testid="dictionary-agent-settings"
            >
              <div className="flex flex-wrap items-center gap-4">
                <div className="flex flex-wrap items-center gap-2 text-label">
                  <span id="dictionary-agent-writes-label">
                    {DICTIONARY_LABELS.agentSettings.title}
                  </span>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild disabled={!authority}>
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        className="gap-2"
                        aria-labelledby="dictionary-agent-writes-label dictionary-agent-writes-value"
                        data-testid="dictionary-agent-writes"
                        disabled={!authority}
                      >
                        <span id="dictionary-agent-writes-value">
                          {dictionary.settings.agent_writes === 'suggest'
                            ? DICTIONARY_LABELS.agentSettings.suggest
                            : dictionary.settings.agent_writes === 'on_request'
                              ? DICTIONARY_LABELS.agentSettings.onRequest
                              : DICTIONARY_LABELS.agentSettings.add}
                        </span>
                        <ChevronDown aria-hidden="true" className="size-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="start">
                      <DropdownMenuRadioGroup
                        value={dictionary.settings.agent_writes}
                        onValueChange={(value) => {
                          if (!authority) return;
                          const mode = value as 'suggest' | 'on_request' | 'add';
                          if (
                            ['suggest', 'on_request', 'add'].indexOf(mode) >
                              ['suggest', 'on_request', 'add'].indexOf(
                                dictionary.settings!.agent_writes
                              ) &&
                            !canRaiseAgentSettings
                          )
                            return;
                          void dictionary.setAgentWrites(mode).then((result) => {
                            if (!result.ok)
                              toastError({
                                title: DICTIONARY_LABELS.agentSettings.title,
                                msg: result.error.message,
                              });
                          });
                        }}
                      >
                        <DropdownMenuRadioItem value="suggest">
                          {DICTIONARY_LABELS.agentSettings.suggest}
                        </DropdownMenuRadioItem>
                        <DropdownMenuRadioItem
                          value="on_request"
                          disabled={
                            !canRaiseAgentSettings && dictionary.settings.agent_writes === 'suggest'
                          }
                        >
                          {DICTIONARY_LABELS.agentSettings.onRequest}
                        </DropdownMenuRadioItem>
                        <DropdownMenuRadioItem
                          value="add"
                          disabled={
                            !canRaiseAgentSettings && dictionary.settings.agent_writes !== 'add'
                          }
                        >
                          {DICTIONARY_LABELS.agentSettings.add}
                        </DropdownMenuRadioItem>
                      </DropdownMenuRadioGroup>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
                <label className="flex items-center gap-2 text-label">
                  <Checkbox
                    checked={dictionary.settings.suggest_from_chats}
                    disabled={
                      !authority ||
                      (!dictionary.settings.suggest_from_chats && !canRaiseAgentSettings)
                    }
                    onChange={(event) => {
                      if (!authority || (event.target.checked && !canRaiseAgentSettings)) return;
                      void dictionary.setSuggestFromChats(event.target.checked).then((result) => {
                        if (!result.ok)
                          toastError({ title: DICTIONARY_LABELS.title, msg: result.error.message });
                      });
                    }}
                  />
                  {DICTATION_ACTIONS.suggestFromChats}
                </label>
              </div>
              {/* The chat extractor's switch (PR-24), on its own line with what belongs to it
                  under its label: the reason it cannot be turned on here, and Find words in my
                  recent chats while it is on. Turning it on is a raise, as for the capability
                  above; turning it off always works. The InfoTip sits beside the label, never
                  inside it. */}
              <div
                className="flex flex-col items-start gap-2"
                data-testid="dictionary-learn-from-chats-group"
              >
                <div className="flex items-center text-label">
                  <label className="flex items-center gap-2">
                    <Checkbox
                      checked={dictionary.settings.learn_from_chats}
                      disabled={learnGate.disabled}
                      aria-describedby={
                        learnGate.reason ? `${learnHelpId} ${learnReasonId}` : learnHelpId
                      }
                      data-testid="dictionary-learn-from-chats"
                      onChange={(event) => {
                        if (!authority || (event.target.checked && !canRaiseAgentSettings)) return;
                        void dictionary.setLearnFromChats(event.target.checked).then((result) => {
                          if (!result.ok)
                            toastError({
                              title: DICTIONARY_LABELS.title,
                              msg: result.error.message,
                            });
                        });
                      }}
                    />
                    {DICTIONARY_LABELS.agentSettings.learnFromChats}
                  </label>
                  <InfoTip
                    id={learnHelpId}
                    label={DICTIONARY_LABELS.agentSettings.learnFromChats}
                    help={dictionaryMessage('MSG-LEARN-FROM-CHATS-HELP')}
                  />
                </div>
                {learnGate.reason ? (
                  <p
                    id={learnReasonId}
                    className="pl-6 text-supporting text-text-muted"
                    data-testid="dictionary-learn-from-chats-reason"
                  >
                    {learnGate.reason}
                  </p>
                ) : null}
                {canScan ? <SuggestionScan /> : null}
              </div>
            </div>
          </ReadableContent>
        ) : null}
      </div>

      {termMode ? (
        <TermDialog
          key={termKey}
          mode={termMode}
          authority={authority}
          onClose={() => setTermMode(null)}
        />
      ) : null}
      {addTerms ? (
        <AddTermsDialog
          initial={addTerms}
          authority={authority}
          onClose={() => setAddTerms(null)}
        />
      ) : null}
      {reviewing ? (
        <ReviewSuggestionsDialog rows={waitingRows} onClose={() => setReviewing(false)} />
      ) : null}

      <ModalShell
        open={exportOpen}
        onOpenChange={(open) => !open && setExportOpen(false)}
        size="sm"
        title={DICTIONARY_LABELS.exportDialog.title}
        testId="dictionary-export-dialog"
        footer={
          <>
            <Button variant="secondary" onClick={() => setExportOpen(false)}>
              {DICTIONARY_LABELS.cancel}
            </Button>
            <Button
              variant="secondary"
              onClick={() => void runExport(false)}
              data-testid="dictionary-export-public"
            >
              {DICTIONARY_LABELS.exportDialog.publicOnly}
            </Button>
            <Button onClick={() => void runExport(true)} data-testid="dictionary-export-private">
              {DICTIONARY_LABELS.exportDialog.withPrivate}
            </Button>
          </>
        }
      >
        <p className="text-body text-text-default">{dictionaryMessage('MSG-EXPORT-PRIVATE')}</p>
      </ModalShell>

      <ConfirmationModal
        isOpen={confirm !== null}
        title={
          confirm?.kind === 'delete'
            ? DICTIONARY_LABELS.deleteTerm.title(confirm.entry.written)
            : confirm?.kind === 'publicize'
              ? DICTIONARY_LABELS.publicize.title
              : DICTIONARY_LABELS.privatize.title
        }
        message={
          confirm?.kind === 'delete'
            ? dictionaryMessage('MSG-DELETE-TERM', { term: confirm.entry.written })
            : confirm?.kind === 'publicize'
              ? dictionaryMessage(publicizeWarning(confirm.entry))
              : dictionaryMessage('MSG-PRIVATIZE-CONFIRM')
        }
        confirmLabel={
          confirm?.kind === 'delete'
            ? DICTIONARY_LABELS.deleteTerm.confirm
            : confirm?.kind === 'publicize'
              ? DICTIONARY_LABELS.publicize.confirm
              : DICTIONARY_LABELS.privatize.confirm
        }
        cancelLabel={DICTIONARY_LABELS.cancel}
        confirmVariant={confirm?.kind === 'delete' ? 'destructive' : 'default'}
        isSubmitting={busy}
        onConfirm={() => void confirmAction()}
        onCancel={() => setConfirm(null)}
      />

      <DangerousConfirmDialog
        open={deleteAllOpen}
        title={DICTIONARY_LABELS.deleteAllDialog.title}
        description={dictionaryMessage('MSG-DELETE-ALL', { n: total.toLocaleString('en-US') })}
        phrase="delete"
        fieldLabel={DICTIONARY_LABELS.deleteAllDialog.field}
        confirmLabel={DICTIONARY_LABELS.deleteAllDialog.confirm}
        cancelLabel={DICTIONARY_LABELS.cancel}
        busy={busy}
        onConfirm={() => void deleteAll()}
        onCancel={() => setDeleteAllOpen(false)}
      />
    </MainPanelLayout>
  );
}

/**
 * Save the export where the person chooses: through the main process's Save
 * window in the app, announcing only a written file, or as a browser download
 * (the shim's Save window would write on the server), which claims nothing.
 */
async function saveExportFile(json: string): Promise<void> {
  const electron = window.electron;
  if (isBrowserSurface() || !electron?.showSaveDialog || !electron?.writeFile) {
    const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = DICTIONARY_EXPORT_FILE;
    document.body.appendChild(anchor);
    anchor.click();
    document.body.removeChild(anchor);
    URL.revokeObjectURL(url);
    return;
  }
  const choice = await electron.showSaveDialog({
    title: DICTIONARY_LABELS.exportDialog.saveTitle,
    defaultPath: DICTIONARY_EXPORT_FILE,
    filters: [{ name: DICTIONARY_LABELS.exportDialog.fileType, extensions: ['json'] }],
  });
  if (choice.canceled || !choice.filePath) return;
  const { file, folder } = savedFileParts(choice.filePath);
  let written = false;
  try {
    written = await electron.writeFile(choice.filePath, json);
  } catch {
    written = false;
  }
  if (written) {
    toastSuccess({
      title: DICTIONARY_LABELS.exportDialog.saved,
      msg: dictionaryMessage('MSG-EXPORT-SAVED', { file, folder }),
    });
  } else {
    toastError({
      title: DICTIONARY_LABELS.exportDialog.failed,
      msg: dictionaryMessage('MSG-EXPORT-WRITE-FAILED', { folder }),
    });
  }
}

/**
 * **Find words in my recent chats**, under **Suggest words from my messages** at the label's
 * text inset: a small button, which a running scan replaces with its progress and **Stop**, and
 * after it how the scan ended, or the daemon's refusal. The line is one polite live region that
 * stays mounted, so each change is read out.
 */
function SuggestionScan() {
  const state = useSuggestionScan(true);
  const running = state.scan?.state === 'running';
  const line = scanLine(state);
  const status = (
    <p
      key="status"
      aria-live="polite"
      className="text-supporting text-text-muted"
      data-testid="dictionary-scan-status"
    >
      {line}
    </p>
  );
  const action = running ? (
    <Button
      key="action"
      type="button"
      size="sm"
      variant="ghost"
      disabled={state.stopping}
      onClick={() => void stopScan()}
      data-testid="dictionary-scan-stop"
    >
      {DICTIONARY_LABELS.scan.stop}
    </Button>
  ) : (
    <Button
      key="action"
      type="button"
      size="sm"
      variant="secondary"
      disabled={state.starting}
      aria-busy={state.starting || undefined}
      onClick={() => void startScan()}
      data-testid="dictionary-scan-start"
    >
      {DICTIONARY_LABELS.scan.find}
    </Button>
  );
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 pl-6" data-testid="dictionary-scan">
      {running ? [status, action] : [action, status]}
    </div>
  );
}

function TermRowSkeleton() {
  return (
    <div className="biorouter-list-row flex items-center gap-3 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-32" />
        <Skeleton className="mt-1 h-3 w-56" />
      </div>
    </div>
  );
}
