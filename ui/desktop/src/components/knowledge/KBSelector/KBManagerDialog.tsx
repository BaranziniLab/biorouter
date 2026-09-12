import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Download,
  FolderInput,
  FolderPlus,
  MoreHorizontal,
  Pencil,
  Search,
  Trash2,
} from '../../icons/app-icons';
import type { KbFormat, KbListEntry } from '../../../api/types.gen';
import { Badge } from '../../ui/badge';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { Button } from '../../ui/button';
import { EmptyState } from '../../ui/empty-state';
import { Input } from '../../ui/input';
import { Switch } from '../../ui/switch';
import { Skeleton } from '../../ui/skeleton';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { ModalShell } from '../../ModalShell';
import BuiltInBadge from '../../ui/BuiltInBadge';
import { BUILTIN_RECREATED_TITLE, isBuiltinKnowledgeBase } from '../../../utils/builtins';
import { useKnowledge } from '../KnowledgeContext';
import { useKnowledgeBases } from '../hooks/useKnowledgeBases';
import { KbDot } from '../KbDot';
import { kbFormatLabel, LEGACY_FORMAT_TITLE } from '../kbFormat';
import { KbFormatChooser } from './KbFormatChooser';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Open with the create flow already started (the picker's "Create…" row). */
  startInCreate?: boolean;
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
export function KBManagerDialog({ open, onOpenChange, startInCreate = false }: Props) {
  const { bases, loading, primaryKbId, hiddenKbIds, refresh, setPrimaryKbId, toggleKbHidden } =
    useKnowledge();
  const { create, exportArchive, importArchive, remove, rename } = useKnowledgeBases();
  const [query, setQuery] = useState('');
  const [draft, setDraft] = useState('');
  const [draftMode, setDraftMode] = useState<DraftMode>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [baseToDelete, setBaseToDelete] = useState<KbListEntry | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chooserOpen, setChooserOpen] = useState(false);
  const importRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setDraftMode(null);
    setChooserOpen(startInCreate);
    setDraft('');
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
   * Create through the chooser.
   *
   * The id is a LOCAL uniqueness pass over the bases this client knows about;
   * the daemon still owns collision handling, which is why §4.3's preview line
   * says the final id may differ and why the primary is set from the manifest
   * the daemon returns rather than from `id`.
   */
  async function createWithFormat(name: string, format: KbFormat): Promise<string | undefined> {
    const id = makeUniqueId(name);
    if (!id) throw new Error('Choose a name with letters or numbers.');
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
    setDraftMode({ kind: 'rename', base });
    setDraft(base.name);
  }

  function resetDraft() {
    setDraft('');
    setDraftMode(null);
    setError(null);
  }

  async function submitDraft() {
    const trimmed = draft.trim();
    if (!trimmed) {
      setError('Enter a name.');
      return;
    }

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
          title="Knowledge bases"
          subtitle="Choose which knowledge bases this chat uses, and which one of them is the primary: the base a knowledge write lands in when none is named."
          bodyClassName="px-0"
          footer={
            <div className="flex w-full flex-wrap items-center justify-between gap-2">
              <p className="min-w-0 flex-1 text-supporting text-text-muted">
                Tip: the switch decides whether a base is in this chat; clicking its name makes it
                the primary. Bases left out are still reachable by naming them explicitly.
              </p>
              <Button
                type="button"
                variant="secondary"
                onClick={() => onOpenChange(false)}
                disabled={busyId !== null}
              >
                Close
              </Button>
            </div>
          }
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
              <div className="mt-4 rounded-element border border-border-subtle p-3">
                <div className="mb-2 text-label">{`Rename "${draftMode.base.name}"`}</div>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <Input
                    data-testid="knowledge-kb-name-input"
                    type="text"
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
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
              </div>
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
              <div className="biorouter-list-shell" role="listbox" aria-label="Knowledge bases">
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
                            <div className="truncate text-label text-text-default">{base.name}</div>
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
                            <Badge
                              uppercase
                              title={
                                kbFormatLabel(base) === 'Legacy' ? LEGACY_FORMAT_TITLE : undefined
                              }
                            >
                              {kbFormatLabel(base)}
                            </Badge>
                            {base.tier !== 'public' && <PrivacyBadge tier={base.tier} />}
                            {/* ⚠ **No `Not in this chat` badge** (R-07). The switch
                              at the end of this row already states membership,
                              and drawing the same fact twice is what made a
                              40px row carry 12 visual objects. */}
                          </div>
                        </div>
                        {isPrimary && (
                          <Badge uppercase tone="accent">
                            Primary
                          </Badge>
                        )}
                      </button>

                      <div className="flex shrink-0 items-center gap-2">
                        {/* Unwrapped. The switch is its own affordance and carries
                          its own name; the bordered `px-2 py-1` box around it was
                          a second control drawn around a control. */}
                        <Switch
                          checked={!hidden}
                          onCheckedChange={() => toggleKbHidden(base.id)}
                          variant="mono"
                          aria-label={`Include ${base.name} in this chat`}
                        />
                        {/* ⚠ **ONE overflow control, not three** (R-07). Export,
                          rename and ⋯ reserved a fixed 160px cluster at the end
                          of every row before the name column got anything —
                          40 for the switch, three 32px buttons and three 8px
                          gaps — on a 582px row. Export and rename are not
                          frequent enough to each own a permanent target, so
                          they join the menu that was already there and the
                          reclaimed width carries what the row could never show:
                          how big the base is and when it last changed. */}
                        <span className="shrink-0 font-mono text-supporting tabular-nums text-text-muted">
                          {base.id}
                        </span>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              type="button"
                              variant="ghost"
                              shape="round"
                              disabled={isBusy}
                              aria-label={`More actions for ${base.name}`}
                            >
                              <MoreHorizontal aria-hidden="true" />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onSelect={() => void handleExport(base)}>
                              <Download aria-hidden="true" />
                              Export as .brkb
                            </DropdownMenuItem>
                            <DropdownMenuItem onSelect={() => startRename(base)}>
                              <Pencil aria-hidden="true" />
                              Rename knowledge base
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            {/* A destructive control never sits visible in a hover
                              cluster (ROWS-3), and stays behind the separator. */}
                            <DropdownMenuItem
                              variant="destructive"
                              onSelect={() => setBaseToDelete(base)}
                            >
                              <Trash2 aria-hidden="true" />
                              Delete knowledge base
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
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
