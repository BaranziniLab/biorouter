import { useCallback, useEffect, useState } from 'react';
import { memoryDeleteCategory, memoryDeleteEntry, memoryInventory } from '../../../api';
import type {
  MemoryCategoryInventory,
  MemoryEntry,
  MemoryScope,
  MemoryStoreInventory,
} from '../../../api';
import { toastError, toastSuccess } from '../../../toasts';
import { getInitialWorkingDir } from '../../../utils/workingDir';
import { Button } from '../../ui/button';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { ChevronDown, ChevronRight, Globe, Folder, RefreshCw, Trash2 } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { Note } from '../../ui/note';
import { Skeleton } from '../../ui/skeleton';

/**
 * Managing what Biorouter has remembered.
 *
 * Issue #63 put every machine-wide memory read behind the user's approval. The
 * approval names a category — and until this section existed, nothing in the
 * app would show what that category held. Approving a disclosure of text you
 * have never seen is not consent, so the gate needed somewhere to send people.
 *
 * It lives in Settings → Chat, next to the Capabilities switch that turns the
 * memory extension on and off: memory is chat behaviour, the switch that
 * governs it is already here, and a store of at most a few dozen short notes
 * does not warrant a top-level route the way the Knowledge workspace does.
 *
 * The provenance shown is the provenance that exists. A memory is a line in a
 * flat per-category text file, so the store records the category, the scope,
 * whatever tags the model attached, and the file's size and modification time.
 * It does **not** record when an individual memory was written, which
 * conversation wrote it, or which model — so this section says "category file
 * last modified" rather than dating each row, and says so out loud rather than
 * showing a timestamp that would be read as the row's own.
 */

const scopeNoun = (scope: MemoryScope) => (scope === 'global' ? 'global' : 'local');

/** Who can read a store. This is the sentence the whole section exists to make. */
const scopeAudience = (scope: MemoryScope) =>
  scope === 'global'
    ? 'every chat on this computer, in any project'
    : 'every chat opened in this project';

const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;

const formatWhen = (secs?: number | null) => {
  if (!secs) return null;
  return new Date(secs * 1000).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
};

const formatBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} KB`;
};

/** Keep a confirmation readable when a memory runs to paragraphs. */
const quote = (text: string, max = 240) =>
  text.length <= max ? text : `${text.slice(0, max).trimEnd()}…`;

/**
 * A delete carries back the state the user was looking at when they clicked:
 * the row's `digest` and the category's `revision`. The daemon compares both
 * and refuses (409) if either moved, because the store is appended to by
 * conversations that may be running while this list sits open — see
 * `MemoryServer::delete_entry`. `revision` is why `category` is kept whole here
 * rather than reduced to its name.
 */
type PendingDelete =
  | { kind: 'category'; scope: MemoryScope; category: MemoryCategoryInventory }
  | {
      kind: 'entry';
      scope: MemoryScope;
      category: MemoryCategoryInventory;
      entry: MemoryEntry;
      isLastInCategory: boolean;
    };

export default function MemorySection() {
  const [workingDir] = useState<string>(() => getInitialWorkingDir());
  const [global, setGlobal] = useState<MemoryStoreInventory | null>(null);
  const [local, setLocal] = useState<MemoryStoreInventory | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [pending, setPending] = useState<PendingDelete | null>(null);
  const [deleting, setDeleting] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const response = await memoryInventory<true>({
        query: workingDir ? { working_dir: workingDir } : {},
        throwOnError: true,
      });
      setGlobal(response.data.global);
      setLocal(response.data.local ?? null);
      setError(null);
    } catch (err) {
      setError(
        `Could not read the memory stores: ${
          err instanceof Error ? err.message : 'the request failed.'
        }`
      );
    } finally {
      setLoading(false);
    }
  }, [workingDir]);

  useEffect(() => {
    void load();
  }, [load]);

  const toggle = (key: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const confirmDelete = async () => {
    if (!pending) return;
    setDeleting(true);
    try {
      if (pending.kind === 'category') {
        const response = await memoryDeleteCategory<true>({
          body: {
            scope: pending.scope,
            category: pending.category.name,
            revision: pending.category.revision,
            working_dir: workingDir,
          },
          throwOnError: true,
        });
        toastSuccess({
          title: pending.category.name,
          msg: `Deleted ${plural(response.data.removed_entries, 'memory').replace(
            'memorys',
            'memories'
          )}`,
        });
      } else {
        const response = await memoryDeleteEntry<true>({
          body: {
            scope: pending.scope,
            category: pending.category.name,
            index: pending.entry.index,
            digest: pending.entry.digest,
            revision: pending.category.revision,
            working_dir: workingDir,
          },
          throwOnError: true,
        });
        toastSuccess({
          title: pending.category.name,
          msg: response.data.category_removed
            ? 'Memory deleted; the category is now empty and was removed'
            : 'Memory deleted',
        });
      }
      setPending(null);
      await load();
    } catch (err) {
      toastError({
        title: 'Memory not deleted',
        msg:
          err instanceof Error
            ? err.message
            : 'Biorouter could not delete it. Reload and try again.',
      });
      setPending(null);
    } finally {
      setDeleting(false);
    }
  };

  const confirmation = (() => {
    if (!pending) return { title: '', message: '' };
    if (pending.kind === 'category') {
      const count = pending.category.entries.length;
      return {
        title: `Delete the ${scopeNoun(pending.scope)} category “${pending.category.name}”?`,
        message:
          `This permanently deletes ${count === 1 ? '1 memory' : `${count} memories`} that ` +
          `${scopeAudience(pending.scope)} can read. Biorouter keeps no copy: this cannot be ` +
          `undone.`,
      };
    }
    return {
      title: 'Delete this memory?',
      message:
        `“${quote(pending.entry.content)}” will be permanently removed from the ` +
        `${scopeNoun(pending.scope)} category “${pending.category.name}”, which ` +
        `${scopeAudience(pending.scope)} can read. This cannot be undone.` +
        (pending.isLastInCategory
          ? ` It is the last memory in “${pending.category.name}”, so the category goes with it.`
          : ''),
    };
  })();

  return (
    <div className="biorouter-settings-section" data-testid="memory-section">
      <div className="biorouter-settings-section-header flex flex-wrap items-end justify-between gap-2">
        {/* The `min-w-0` wrapper is what keeps Refresh clear of the text. The
            two sentences that were here are gone rather than restyled: the two
            store headings below say which chats can read which store, and the
            delete dialog says the delete is permanent — so the header was
            repeating its own page back at itself, which is the shape the audit
            called "a banner that is just a long block of text". */}
        <div className="min-w-0">
          <h2 className="mb-1 text-caps text-text-muted">Memory</h2>
          <p className="text-supporting text-text-muted">
            What Biorouter has been asked to remember, and everything it can disclose when a chat
            asks to read it.
          </p>
        </div>
        <Button type="button" variant="ghost" className="mr-3" onClick={() => void load()}>
          <RefreshCw />
          Refresh
        </Button>
      </div>

      {error && (
        <Note tone="danger" role="status" className="mb-2">
          {error}
        </Note>
      )}

      {loading && !global && (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-16 w-full" />
        </div>
      )}

      {/* Gated on `global`, not on `!loading`, so a failed load renders neither
          store block. Falling through would show the "no project open" card and
          tell the user their memories are not there — the one wrong answer for
          a store they are being asked to approve reads of. The error above
          already says what happened, and Refresh retries it. */}
      {global && (
        <div className="flex flex-col gap-4">
          <StoreBlock
            store={global}
            icon={<Globe className="h-3.5 w-3.5 text-text-muted" aria-hidden="true" />}
            heading="Global: shared by every chat on this computer"
            expanded={expanded}
            onToggle={toggle}
            onDeleteCategory={(category) =>
              setPending({ kind: 'category', scope: 'global', category })
            }
            onDeleteEntry={(category, entry, isLastInCategory) =>
              setPending({ kind: 'entry', scope: 'global', category, entry, isLastInCategory })
            }
          />

          {local ? (
            <StoreBlock
              store={local}
              icon={<Folder className="h-3.5 w-3.5 text-text-muted" aria-hidden="true" />}
              heading="This project: only chats opened here"
              expanded={expanded}
              onToggle={toggle}
              onDeleteCategory={(category) =>
                setPending({ kind: 'category', scope: 'local', category })
              }
              onDeleteEntry={(category, entry, isLastInCategory) =>
                setPending({ kind: 'entry', scope: 'local', category, entry, isLastInCategory })
              }
            />
          ) : (
            <Note tone="neutral">
              This window has no project open, so there is no local memory store to show. Local
              memories live in a project&rsquo;s <code>.biorouter/memory</code> and are managed from
              a window opened there.
            </Note>
          )}
        </div>
      )}

      <ConfirmationModal
        isOpen={pending !== null}
        title={confirmation.title}
        message={confirmation.message}
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        isSubmitting={deleting}
        onConfirm={confirmDelete}
        onCancel={() => setPending(null)}
      />
    </div>
  );
}

function StoreBlock({
  store,
  icon,
  heading,
  expanded,
  onToggle,
  onDeleteCategory,
  onDeleteEntry,
}: {
  store: MemoryStoreInventory | null;
  icon: React.ReactNode;
  heading: string;
  expanded: Set<string>;
  onToggle: (key: string) => void;
  onDeleteCategory: (category: MemoryCategoryInventory) => void;
  onDeleteEntry: (
    category: MemoryCategoryInventory,
    entry: MemoryEntry,
    isLastInCategory: boolean
  ) => void;
}) {
  if (!store) return null;
  const total = store.categories.reduce((sum, category) => sum + category.entries.length, 0);

  return (
    <div>
      <div className="mb-1 flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <span className="flex items-center gap-1.5 text-label text-text-default">
          {icon}
          {heading}
        </span>
        <span className="text-supporting tabular-nums text-text-subtle">
          {total === 0
            ? 'empty'
            : `${plural(store.categories.length, 'category').replace(
                'categorys',
                'categories'
              )} · ${total === 1 ? '1 memory' : `${total} memories`}`}
        </span>
      </div>
      <p className="mb-2 min-w-0 break-words font-mono text-supporting leading-4 text-text-subtle [overflow-wrap:anywhere]">
        {store.path}
      </p>

      {store.categories.length === 0 ? (
        <Note tone="neutral">
          Nothing has been remembered here yet. Biorouter adds a memory only when it asks you and
          you agree.
        </Note>
      ) : (
        // A flat `.biorouter-settings-list`, not a bordered `biorouter-list-shell`.
        // The shell and its rows move TOGETHER: `list-row` and `settings-row`
        // differ only in a 42%-vs-38% hover wash for the identical gesture, and
        // swapping one without the other would make a pairing that exists
        // nowhere else in the app.
        <div className="biorouter-settings-list">
          {store.categories.map((category) => (
            <CategoryRow
              key={`${store.scope}:${category.name}`}
              scope={store.scope}
              category={category}
              isExpanded={expanded.has(`${store.scope}:${category.name}`)}
              onToggle={() => onToggle(`${store.scope}:${category.name}`)}
              onDeleteCategory={() => onDeleteCategory(category)}
              onDeleteEntry={(entry) =>
                onDeleteEntry(category, entry, category.entries.length === 1)
              }
            />
          ))}
        </div>
      )}
    </div>
  );
}

function CategoryRow({
  scope,
  category,
  isExpanded,
  onToggle,
  onDeleteCategory,
  onDeleteEntry,
}: {
  scope: MemoryScope;
  category: MemoryCategoryInventory;
  isExpanded: boolean;
  onToggle: () => void;
  onDeleteCategory: () => void;
  onDeleteEntry: (entry: MemoryEntry) => void;
}) {
  const count = category.entries.length;
  const countLabel = count === 1 ? '1 memory' : `${count} memories`;
  const when = formatWhen(category.modified);

  return (
    <div className="biorouter-settings-row px-3 py-2.5">
      <div className="flex items-start gap-2">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={isExpanded}
          aria-label={`${isExpanded ? 'Hide' : 'Show'} the ${countLabel} in ${category.name}`}
          className="flex min-w-0 flex-1 items-start gap-2 text-left"
        >
          {isExpanded ? (
            <ChevronDown className="mt-0.5 h-3.5 w-3.5 shrink-0 text-text-muted" />
          ) : (
            <ChevronRight className="mt-0.5 h-3.5 w-3.5 shrink-0 text-text-muted" />
          )}
          <span className="min-w-0 flex-1">
            <span className="block min-w-0 break-words text-label text-text-default [overflow-wrap:anywhere]">
              {category.name}
            </span>
            <span className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-supporting text-text-subtle">
              <span className="tabular-nums">{countLabel}</span>
              <span className="tabular-nums">{formatBytes(category.size_bytes)}</span>
              {when && (
                <span title="When this category file was last written to">Updated {when}</span>
              )}
            </span>
          </span>
        </button>
        {/* The one row-trailing glyph action: `ghost` + `shape="round"` is the
            32×32 rung, and the cva base supplies the 16px glyph. The `h-7 w-7
            p-0` box was an off-ladder 28px and `hover:bg-background-danger/10`
            was a second hover behaviour beside the one `tint-interactive`
            already owns. */}
        <Button
          onClick={onDeleteCategory}
          variant="ghost"
          shape="round"
          className="shrink-0 text-text-danger"
          title={`Delete the whole ${scopeNoun(scope)} category`}
          aria-label={`Delete the ${scopeNoun(scope)} category ${category.name}`}
        >
          <Trash2 />
        </Button>
      </div>

      {isExpanded && (
        <ul className="mt-2 flex flex-col gap-1.5 border-t border-border-subtle pt-2">
          {category.entries.map((entry) => (
            <li key={entry.index} className="flex items-start gap-2">
              <div className="min-w-0 flex-1">
                {entry.tags.length > 0 && (
                  <div className="mb-1 flex flex-wrap gap-1">
                    {entry.tags.map((tag) => (
                      // `h-auto`/`py-0.5` because this badge WRAPS: a model
                      // may attach a tag longer than the column, and the
                      // primitive's fixed 20px box would spill it.
                      <Badge
                        key={tag}
                        tone="neutral"
                        className="h-auto min-h-5 min-w-0 shrink break-words py-0.5 [overflow-wrap:anywhere]"
                      >
                        {tag}
                      </Badge>
                    ))}
                  </div>
                )}
                <p className="min-w-0 whitespace-pre-wrap break-words text-supporting text-text-default [overflow-wrap:anywhere]">
                  {entry.content}
                </p>
              </div>
              <Button
                onClick={() => onDeleteEntry(entry)}
                variant="ghost"
                shape="round"
                className="shrink-0 text-text-danger"
                title="Delete this memory"
                aria-label={`Delete this memory from ${category.name}`}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
