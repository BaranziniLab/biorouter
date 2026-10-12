import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Copy,
  Download,
  FolderInput,
  FolderPlus,
  Pencil,
  Search,
  Trash2,
} from '../../icons/app-icons';
import type { KbFormat, KbListEntry } from '../../../api/types.gen';
import { Badge } from '../../ui/badge';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import { Field, fieldHelpId } from '../../ui/field';
import { InfoTip } from '../../ui/info-tip';
import { Input } from '../../ui/input';
import { Switch } from '../../ui/switch';
import { Skeleton } from '../../ui/skeleton';
import { RowActions, type RowActionItem } from '../../ui/row-actions';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { ModalShell } from '../../ModalShell';
import BuiltInBadge from '../../ui/BuiltInBadge';
import { BUILTIN_RECREATED_TITLE, isBuiltinKnowledgeBase } from '../../../utils/builtins';
import { copyToClipboard } from '../../../utils/clipboard';
import { toastError, toastSuccess } from '../../../toasts';
import { knowledgeCopy } from '../copy';
import { useKnowledge } from '../KnowledgeContext';
import { useKnowledgeBases } from '../hooks/useKnowledgeBases';
import { KbDot } from '../KbDot';
import { kbFormatLabel } from '../kbFormat';
import { KbFormatChooser } from './KbFormatChooser';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Open with the create flow already started (the picker's "Create…" row). */
  startInCreate?: boolean;
  /**
   * Radix's close-focus hook for the manager, for a caller that opens it from a control which is
   * gone by the time it closes (the picker's row): call `event.preventDefault()` and place focus.
   * The chooser needs none, because it always ends in the manager.
   */
  onCloseAutoFocus?: (event: Event) => void;
}

/**
 * Renaming is still an inline draft; CREATING is not.
 *
 * The two used to share one text field, and that is what made the format an
 * invisible daemon default: a create flow whose only input is a name has
 * nowhere to put the choice §4.3 requires the user to make. Rename genuinely is
 * one field and stays inline.
 */
type DraftMode = { kind: 'rename'; base: KbListEntry } | null;

/**
 * The knowledge-base manager: the half of the old `KBSelectorPalette` that is
 * about the COLLECTION rather than the pointer (ui-spec §4.2).
 *
 * ⚠ **`ModalShell size="lg"` (640px), not a hand-rolled 760px `Dialog`.** 760 is
 * not a dialog width — it is `--measure-chat` borrowed, and there is no fourth
 * width. `purpose="form"` so a stray backdrop click cannot throw away a
 * half-typed name.
 *
 * ⚠ **Every secondary action here is `variant="secondary"`, never `outline`.**
 * D-25 narrows `outline` to "a secondary action on an already-TINTED ground",
 * and every ground in this section — the page canvas, the dialog body, the row
 * — is untinted. A 1px box drawn around a panel's quietest actions was the
 * heaviest line in it.
 */
export function KBManagerDialog({
  open,
  onOpenChange,
  startInCreate = false,
  onCloseAutoFocus,
}: Props) {
  const { bases, loading, primaryKbId, hiddenKbIds, refresh, setPrimaryKbId, toggleKbHidden } =
    useKnowledge();
  const { create, exportArchive, importArchive, remove, rename } = useKnowledgeBases();
  const [query, setQuery] = useState('');
  const [draft, setDraft] = useState('');
  const [draftMode, setDraftMode] = useState<DraftMode>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [baseToDelete, setBaseToDelete] = useState<KbListEntry | null>(null);
  const [error, setError] = useState<string | null>(null);
  // What is wrong with the name typed into the rename card: the Field's error, on the field.
  // `error` above is a request that failed, which is not about the field.
  const [draftError, setDraftError] = useState<string | null>(null);
  const [chooserOpen, setChooserOpen] = useState(false);
  const importRef = useRef<HTMLInputElement>(null);
  const renameRef = useRef<HTMLInputElement>(null);
  const createRef = useRef<HTMLButtonElement>(null);
  // True from the moment the format chooser takes this dialog's place until the manager comes back
  // under it. The manager is UNMOUNTED while the chooser is up (one usable dialog, see the return
  // below), so what comes back is a new dialog whose first focus would otherwise be the first
  // tabbable in its header, the "About bases in this chat" InfoTip, ringed after a keyboard close
  // (V3-R2-4). `placeFocus` sends it to the button that opened the chooser instead.
  const returnToCreate = useRef(false);

  useEffect(() => {
    if (chooserOpen) returnToCreate.current = true;
  }, [chooserOpen]);

  useEffect(() => {
    returnToCreate.current = false;
    if (!open) return;
    setDraftMode(null);
    setChooserOpen(startInCreate);
    setDraft('');
    setDraftError(null);
    setError(null);
    // A base created elsewhere (chat, the knowledge MCP tools) is invisible here
    // until something asks again.
    void refresh();
  }, [open, startInCreate, refresh]);

  const filtered = useMemo(() => {
    const value = query.trim().toLowerCase();
    if (!value) return bases;
    return bases.filter(
      (base) => base.name.toLowerCase().includes(value) || base.id.toLowerCase().includes(value)
    );
  }, [bases, query]);

  function slugify(input: string): string {
    return input
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .substring(0, 64);
  }

  function makeUniqueId(name: string): string {
    const baseSlug = slugify(name);
    if (!baseSlug) return '';
    if (!bases.some((base) => base.id === baseSlug)) return baseSlug;

    let suffix = 2;
    let candidate = `${baseSlug}-${suffix}`;
    while (bases.some((base) => base.id === candidate) && suffix < 1000) {
      suffix += 1;
      candidate = `${baseSlug}-${suffix}`;
    }
    return candidate;
  }

  function startCreate() {
    setError(null);
    setDraftMode(null);
    setChooserOpen(true);
  }

  /**
   * Radix's open-focus hook for the manager: when it is coming back from the chooser, focus goes to
   * "Create knowledge base", the control the person was on, and not to the header's InfoTip. A
   * first open, and a reopen, keep Radix's own first stop. No ring is forced: a script focus
   * wears one only when the chooser was closed with the keyboard, which is the person who needs it.
   */
  function placeFocus(event: Event) {
    if (!returnToCreate.current) return;
    returnToCreate.current = false;
    const target = createRef.current;
    if (!target) return;
    event.preventDefault();
    target.focus({ preventScroll: true });
  }

  /**
   * Create through the chooser.
   *
   * The id is a LOCAL uniqueness pass over the bases this client knows about;
   * the daemon still owns collision handling, which is why §4.3's preview line
   * says the final id may differ and why the primary is set from the manifest
   * the daemon returns rather than from `id`.
   */
  async function createWithFormat(name: string, format: KbFormat): Promise<string | undefined> {
    const id = makeUniqueId(name);
    if (!id) throw new Error(knowledgeCopy.format.nameNeedsLetters);
    setBusyId('__create');
    try {
      const manifest = await create(id, name, { format });
      if (manifest?.id) setPrimaryKbId(manifest.id);
      await refresh();
      return manifest?.id;
    } finally {
      setBusyId(null);
    }
  }

  function startRename(base: KbListEntry) {
    setError(null);
    setDraftError(null);
    setDraftMode({ kind: 'rename', base });
    setDraft(base.name);
  }

  function resetDraft() {
    setDraft('');
    setDraftMode(null);
    setDraftError(null);
    setError(null);
  }

  async function submitDraft() {
    const trimmed = draft.trim();
    if (!trimmed) {
      // On the field, with the caret in it (V3-R4-3): a bare line under the card had no danger
      // edge and no `aria-invalid`, and left focus on Save.
      setDraftError(knowledgeCopy.format.nameRequired);
      setError(null);
      renameRef.current?.focus();
      return;
    }
    setDraftError(null);

    try {
      if (draftMode?.kind === 'rename') {
        setBusyId(draftMode.base.id);
        // No `setPrimaryKbId(manifest.id)` after this: a rename moves every
        // pointer that named the base, machine default and chats alike, and
        // `rename` ends by re-reading the selection. Re-pinning it from here
        // turned a chat that only inherited the renamed base into one that
        // pinned it.
        await rename(draftMode.base.id, trimmed);
      }
      resetDraft();
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyId(null);
    }
  }

  async function handleRemove() {
    if (!baseToDelete || busyId !== null) return;
    const base = baseToDelete;
    setError(null);
    setBusyId(base.id);
    try {
      await remove(base.id);
      setBaseToDelete(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyId(null);
    }
  }

  async function handleExport(base: KbListEntry) {
    setError(null);
    setBusyId(base.id);
    try {
      await exportArchive(base.id, base.name);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyId(null);
    }
  }

  // The id is no longer printed on every row (principle 10, Crew's "IDs for
  // support"); it is one menu item away for the person who needs it.
  async function handleCopyId(base: KbListEntry) {
    if (await copyToClipboard(base.id, null)) {
      toastSuccess({
        title: knowledgeCopy.manager.copied,
        msg: base.id,
        toastOptions: { autoClose: 2000 },
      });
    } else {
      toastError({ title: knowledgeCopy.manager.copyFailed, msg: base.id });
    }
  }

  async function handleImport(file: File | null) {
    if (!file) return;
    setError(null);
    setBusyId('__import');
    try {
      await importArchive(file);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyId(null);
    }
  }

  // The menu behind a row's ⋯. A destructive item stays behind the separator, and every item is
  // inert while this row's own request is in flight (the old ⋯ disabled itself for that).
  function rowMenu(base: KbListEntry, busy: boolean): RowActionItem[] {
    return [
      {
        label: knowledgeCopy.actions.export,
        icon: Download,
        disabled: busy,
        onSelect: () => void handleExport(base),
      },
      {
        label: knowledgeCopy.manager.rename,
        icon: Pencil,
        disabled: busy,
        onSelect: () => startRename(base),
      },
      {
        label: knowledgeCopy.manager.copyId,
        icon: Copy,
        disabled: busy,
        onSelect: () => void handleCopyId(base),
      },
      { kind: 'separator' },
      {
        label: knowledgeCopy.actions.delete,
        icon: Trash2,
        destructive: true,
        disabled: busy,
        onSelect: () => setBaseToDelete(base),
      },
    ];
  }

  return (
    <>
      {!chooserOpen && (
        <ModalShell
          open={open}
          onOpenChange={(next) => {
            if (!next && busyId !== null) return;
            onOpenChange(next);
          }}
          size="lg"
          purpose={busyId !== null ? 'required' : 'form'}
          scrollBody
          onOpenAutoFocus={placeFocus}
          onCloseAutoFocus={onCloseAutoFocus}
          title={knowledgeCopy.manager.title}
          // ONE sentence, because ModalShell makes the subtitle the dialog's
          // `aria-describedby` (ModalShell.test.tsx names this dialog). The rest
          // of the explanation, and the footer Tip that repeated it, are the
          // InfoTip at the end of the line (spec 3.12). The dialog has its ×, so
          // there is no footer.
          subtitle={
            <span className="inline-flex items-center gap-1">
              {knowledgeCopy.manager.subtitle}
              <InfoTip
                label={knowledgeCopy.manager.infoLabel}
                help={knowledgeCopy.manager.info}
                side="bottom"
              />
            </span>
          }
          bodyClassName="px-0"
        >
          {/* ⚠ **Pinned** (R-07). Search and the two create paths lived in the
            scrolling body, so at the app's own minimum 600px window height —
            where the 85vh cap leaves about 245px of list, six rows, three with
            the rename card open — they scrolled away exactly when a long list
            made them useful. `sticky top-0` is right HERE and wrong for the
            ingest rail's footer: this is the FIRST thing in the scroll order, so
            content passes under it and stays reachable, where a pinned footer
            covered the region new content mounted into. */}
          <div className="sticky top-0 z-10 border-b border-border-subtle bg-background-default px-4 pb-3 pt-3">
            <div className="relative">
              <Search
                className="pointer-events-none absolute left-2 top-1/2 h-icon-row w-icon-row -translate-y-1/2 text-text-muted"
                aria-hidden="true"
              />
              <Input
                data-testid="knowledge-kb-search"
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search knowledge bases"
                aria-label="Search knowledge bases"
                className="pl-8"
              />
            </div>

            <div className="mt-3 flex flex-wrap gap-2">
              <Button
                ref={createRef}
                data-testid="knowledge-kb-create"
                type="button"
                variant="default"
                onClick={startCreate}
              >
                <FolderPlus aria-hidden="true" />
                Create knowledge base
              </Button>
              <Button
                data-testid="knowledge-kb-import"
                type="button"
                variant="secondary"
                onClick={() => importRef.current?.click()}
                disabled={busyId === '__import'}
              >
                <FolderInput aria-hidden="true" />
                {busyId === '__import' ? 'Importing…' : 'Import from .brkb'}
              </Button>
              <input
                ref={importRef}
                type="file"
                accept=".brkb"
                className="hidden"
                onChange={(e) => {
                  const file = e.target.files?.[0] ?? null;
                  void handleImport(file);
                  e.target.value = '';
                }}
              />
            </div>

            {draftMode && (
              // An element inside the 12px dialog container takes the next step
              // DOWN, so 8px — not the 12px this block used to carry.
              // The label points at the Input by id, and the row is the Field's child (the schedule
              // dialog's shape for a field with buttons beside it), so the Input names its own
              // description and validity.
              <Field
                id="kb-rename-name"
                label={`Rename "${draftMode.base.name}"`}
                error={draftError ? <span role="alert">{draftError}</span> : undefined}
                className="mt-3"
              >
                <div id="kb-rename-name-row" className="flex flex-col gap-2 sm:flex-row">
                  <Input
                    ref={renameRef}
                    id="kb-rename-name"
                    data-testid="knowledge-kb-name-input"
                    type="text"
                    aria-required
                    aria-invalid={draftError ? true : undefined}
                    aria-describedby={draftError ? fieldHelpId('kb-rename-name') : undefined}
                    value={draft}
                    onChange={(e) => {
                      setDraft(e.target.value);
                      setDraftError(null);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        void submitDraft();
                      }
                    }}
                    placeholder="Knowledge base name"
                    className="flex-1"
                    autoFocus
                  />
                  <div className="flex gap-2">
                    <Button type="button" variant="secondary" onClick={resetDraft}>
                      Cancel
                    </Button>
                    <Button
                      data-testid="knowledge-kb-submit"
                      type="button"
                      onClick={() => void submitDraft()}
                    >
                      Save
                    </Button>
                  </div>
                </div>
              </Field>
            )}

            {error && <div className="mt-3 text-body text-text-danger">{error}</div>}
          </div>

          <div className="px-4 py-2">
            {loading && bases.length === 0 ? (
              <div role="status" aria-label="Loading knowledge bases" className="flex flex-col">
                {[0, 1, 2].map((row) => (
                  <div key={row} className="flex h-row items-center gap-3 px-3" aria-hidden="true">
                    <Skeleton
                      className="h-2 w-2 rounded-full"
                      style={{ animationDelay: `${row * -180}ms` }}
                    />
                    <Skeleton className="h-3 w-40" style={{ animationDelay: `${row * -180}ms` }} />
                  </div>
                ))}
              </div>
            ) : filtered.length === 0 ? (
              <EmptyState
                compact
                icon={Search}
                title="No knowledge bases match"
                description="Try a different name or id."
              />
            ) : (
              <div
                className="biorouter-list-shell br-rows-flat"
                role="listbox"
                aria-label="Knowledge bases"
              >
                {filtered.map((base) => {
                  const isPrimary = primaryKbId === base.id;
                  const isBusy = busyId === base.id;
                  const hidden = hiddenKbIds.includes(base.id);

                  return (
                    <div
                      key={base.id}
                      // ⚠ **No row tint for primary** (R-07). Primary was drawn
                      // TWICE — a `PRIMARY` badge and a `tint-selected` row fill —
                      // and of the two the badge is the one to keep: D-15 makes
                      // focus a SURFACE shift, so a row-wide tint is exactly what
                      // the legend's `asChild` comment exists to avoid, competing
                      // with the focus surface of every control inside the row.
                      className="biorouter-list-row flex items-center gap-3 px-3"
                    >
                      <button
                        type="button"
                        // The row IS the choice, so it carries the option role and
                        // says whether it is the current one. Without this the list
                        // is a pile of buttons to a screen reader, and the tier badge
                        // has no row to belong to.
                        role="option"
                        aria-selected={isPrimary}
                        // Making a base primary is not a navigation: this dialog is
                        // where the whole collection is managed, so it stays open.
                        onClick={() => setPrimaryKbId(base.id)}
                        className="flex min-w-0 flex-1 items-center gap-3 text-left"
                      >
                        <KbDot color={base.color} />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <div
                              id={`kb-row-name-${base.id}`}
                              className="truncate text-label text-text-default"
                            >
                              {base.name}
                            </div>
                            {isBuiltinKnowledgeBase(base.id) && (
                              <BuiltInBadge title={BUILTIN_RECREATED_TITLE} />
                            )}
                            {/* Issue #56 DR-18. This list is the switch, so the tier
                              has to be legible BEFORE the user switches — a badge
                              only on the base you already chose tells you what you
                              did, not what you are about to do. Private is the
                              marked state; Public is quiet, so the marking stays a
                              marking.

                              `!== 'public'`, not `=== 'private'`: anything the
                              daemon sends that is not exactly Public is marked,
                              which is the polarity the whole feature uses and the
                              one that fails safe if the union ever widens. */}
                            {/* The format the base's PAGES are written in. It reads
                              `schema_version` as well as `format`, because
                              `format` alone is the DR-6 trap: it is
                              `serde(default)` on the daemon, so every legacy
                              manifest deserializes as `okf` and a badge written
                              against the field alone would call every legacy base
                              OKF. */}
                            <Badge>{kbFormatLabel(base)}</Badge>
                            {base.tier !== 'public' && <PrivacyBadge tier={base.tier} />}
                            {/* ⚠ **No `Not in this chat` badge** (R-07). The switch
                              at the end of this row already states membership,
                              and drawing the same fact twice is what made a
                              40px row carry 12 visual objects. */}
                          </div>
                        </div>
                        {/* Neutral, not accent: the accent marks state and adds
                            zest, and a word on one row of a list is neither
                            (principle 7). */}
                        {isPrimary && <Badge>{knowledgeCopy.manager.primary}</Badge>}
                      </button>

                      <div className="flex shrink-0 items-center gap-2">
                        {/* Unwrapped. The switch is its own affordance and carries
                          its own name; the bordered `px-2 py-1` box around it was
                          a second control drawn around a control. */}
                        {/* Named by the row's label, the base's name (spec 2.6):
                          the switch says whether the base is in this chat. */}
                        <Switch
                          checked={!hidden}
                          onCheckedChange={() => toggleKbHidden(base.id)}
                          aria-labelledby={`kb-row-name-${base.id}`}
                        />
                        {/* ⚠ **ONE overflow control, not three** (R-07). Export,
                          rename and ⋯ reserved a fixed 160px cluster at the end
                          of every row before the name column got anything.
                          Export and rename are not frequent enough to each own
                          a permanent target, so they join the menu, and so does
                          the base's id: it used to be printed in mono on every
                          row, and an id belongs behind "Copy ID" (principle
                          10).

                          It is `RowActions`, so the ⋯ shows on hover, focus or an
                          open menu and always without a hover pointer, like every
                          other list row (V3-09). It sat at full opacity here and
                          was the one row action in the app that did. The switch
                          stays drawn: it is state, not an action. A destructive
                          item stays behind the separator (ROWS-3). */}
                        <RowActions
                          menuLabel={`More actions for ${base.name}`}
                          menu={rowMenu(base, isBusy)}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </ModalShell>
      )}

      <KbFormatChooser
        open={open && chooserOpen}
        onOpenChange={setChooserOpen}
        initialName={query.trim()}
        onCreate={createWithFormat}
      />

      <ConfirmationModal
        isOpen={baseToDelete !== null}
        title={`Delete "${baseToDelete?.name ?? ''}"?`}
        message="This permanently removes the knowledge base and cannot be undone."
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        isSubmitting={baseToDelete !== null && busyId === baseToDelete.id}
        onConfirm={() => void handleRemove()}
        onCancel={() => setBaseToDelete(null)}
      />
    </>
  );
}
