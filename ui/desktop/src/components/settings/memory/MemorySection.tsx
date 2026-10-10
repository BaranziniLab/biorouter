import { Fragment, useCallback, useEffect, useId, useState } from 'react';
import { memoryDeleteCategory, memoryDeleteEntry, memoryInventory } from '../../../api';
import type {
  MemoryCategoryInventory,
  MemoryEntry,
  MemoryScope,
  MemoryStoreInventory,
} from '../../../api';
import { toastError, toastSuccess } from '../../../toasts';
import { getInitialWorkingDir } from '../../../utils/workingDir';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { ChevronDown, ChevronRight, RefreshCw, Trash2 } from '../../icons/app-icons';
import { Badge } from '../../ui/badge';
import { InfoTip } from '../../ui/info-tip';
import { Note } from '../../ui/note';
import { IconAction, RowActions } from '../../ui/row-actions';
import { SettingSection } from '../../ui/setting-row';
import { Skeleton } from '../../ui/skeleton';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { memoryCopy } from './copy';
import './memory.css';

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
 * conversation wrote it, or which model — so the only date is the category
 * file's, worded "Updated" in the tooltip on the category's name, and no row
 * carries a timestamp that would be read as its own.
 *
 * Layout (spec 3.14): one `SettingSection` of one-line rows. A store is a
 * heading (Global, This project) with its count on the right and an InfoTip
 * holding who can read it and where it lives; a category is a row whose meta
 * is its count and whose delete glyph waits for the pointer; an open
 * category's entries are sibling rows under it, never inside it.
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
    <SettingSection
      id={SETTINGS_SECTION_IDS.memory}
      title={memoryCopy.section}
      help={memoryCopy.help}
      data-testid="memory-section"
      action={
        <IconAction
          icon={RefreshCw}
          label={memoryCopy.refresh}
          size="sm"
          className="br-memory-refresh"
          onSelect={() => void load()}
        />
      }
    >
      {error && (
        <Note tone="danger" role="status" className="mx-3 mb-2">
          {error}
        </Note>
      )}

      {loading && !global && <Skeleton className="mx-3 h-10" />}

      {/* Gated on `global`, not on `!loading`, so a failed load renders neither
          store block. Falling through would show the "no project open" row and
          tell the user their memories are not there — the one wrong answer for
          a store they are being asked to approve reads of. The error above
          already says what happened, and Refresh retries it. */}
      {global && (
        <>
          <StoreBlock
            store={global}
            heading={memoryCopy.global.heading}
            tipLabel={memoryCopy.global.tipLabel}
            audience={memoryCopy.global.help}
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
              heading={memoryCopy.local.heading}
              tipLabel={memoryCopy.local.tipLabel}
              audience={memoryCopy.local.help}
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
            <StoreFrame heading={memoryCopy.local.heading}>
              <MutedRow
                text={memoryCopy.noProject}
                tipLabel={memoryCopy.noProjectTipLabel}
                help={memoryCopy.noProjectHelp}
              />
            </StoreFrame>
          )}
        </>
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
    </SettingSection>
  );
}

/**
 * A store: its heading over its own rows. The heading names the store (Global, This project) and
 * carries the count of what it holds on the right; who can read it, and where it lives, are its
 * InfoTip, and the delete confirmation says the audience again at the moment it matters.
 */
function StoreFrame({
  heading,
  tipLabel,
  help,
  count,
  children,
}: {
  heading: string;
  /** What the InfoTip is about ("global memory"), so its name reads "About global memory". */
  tipLabel?: string;
  /** The InfoTip's body: the audience sentence and the store's folder. */
  help?: React.ReactNode;
  count?: string;
  children: React.ReactNode;
}) {
  const headingId = useId();
  return (
    <div className="br-memory-store" role="group" aria-labelledby={headingId}>
      <div className="br-memory-store-head">
        <h3
          id={headingId}
          className="br-memory-store-title text-secondary font-semibold text-text-default"
        >
          {heading}
        </h3>
        {help ? <InfoTip label={tipLabel ?? heading}>{help}</InfoTip> : null}
        {count ? (
          <span className="br-memory-store-count text-supporting tabular-nums text-text-subtle">
            {count}
          </span>
        ) : null}
      </div>
      {children}
    </div>
  );
}

/** One muted line, for a store with nothing to list. The rest of the explanation is its InfoTip. */
function MutedRow({ text, tipLabel, help }: { text: string; tipLabel: string; help: string }) {
  return (
    <div className="br-memory-empty">
      <span className="text-secondary text-text-muted">{text}</span>
      <InfoTip label={tipLabel} help={help} />
    </div>
  );
}

function StoreBlock({
  store,
  heading,
  tipLabel,
  audience,
  expanded,
  onToggle,
  onDeleteCategory,
  onDeleteEntry,
}: {
  store: MemoryStoreInventory | null;
  heading: string;
  tipLabel: string;
  audience: string;
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
    <StoreFrame
      heading={heading}
      tipLabel={tipLabel}
      // The folder is a machine string, so it is mono and on its own line, and it stays in the DOM
      // (the tip's hidden description) for anything that reads it.
      help={
        <>
          {audience} <span className="br-memory-path">{store.path}</span>
        </>
      }
      count={total === 0 ? undefined : countLabelFor(total)}
    >
      {store.categories.length === 0 ? (
        <MutedRow
          text={memoryCopy.empty}
          tipLabel={memoryCopy.emptyTipLabel}
          help={memoryCopy.emptyHelp}
        />
      ) : (
        // The rows are the shared settings row and abut: the hairline is their only
        // separator, and a store's last row has none.
        store.categories.map((category) => (
          <CategoryRow
            key={`${store.scope}:${category.name}`}
            scope={store.scope}
            category={category}
            isExpanded={expanded.has(`${store.scope}:${category.name}`)}
            onToggle={() => onToggle(`${store.scope}:${category.name}`)}
            onDeleteCategory={() => onDeleteCategory(category)}
            onDeleteEntry={(entry) => onDeleteEntry(category, entry, category.entries.length === 1)}
          />
        ))
      )}
    </StoreFrame>
  );
}

const countLabelFor = (count: number) => (count === 1 ? '1 memory' : `${count} memories`);

/**
 * A category and, when open, its entries as SIBLING rows. They used to render inside the category
 * row, so its hover wash covered the whole list and its hairline moved to the bottom of the list.
 * The count is the row's meta: it shows at rest and gives way to the delete glyph on hover or
 * focus. Updated date and size are the name's tooltip, not standing text.
 */
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
  const countLabel = countLabelFor(category.entries.length);
  const when = formatWhen(category.modified);
  const Chevron = isExpanded ? ChevronDown : ChevronRight;

  return (
    <Fragment>
      <div className="biorouter-settings-row br-setting-row">
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={onToggle}
              aria-expanded={isExpanded}
              aria-label={`${isExpanded ? 'Hide' : 'Show'} the ${countLabel} in ${category.name}`}
              className="br-memory-toggle"
            >
              <Chevron aria-hidden="true" />
              <span className="min-w-0 break-words text-label text-text-default [overflow-wrap:anywhere]">
                {category.name}
              </span>
            </button>
          </TooltipTrigger>
          <TooltipContent>
            {when ? `Updated ${when} · ` : ''}
            {formatBytes(category.size_bytes)}
          </TooltipContent>
        </Tooltip>
        <div className="br-setting-row-control">
          <RowActions
            meta={countLabel}
            primary={
              <IconAction
                icon={Trash2}
                label={`Delete the ${scopeNoun(scope)} category ${category.name}`}
                tooltip="Delete category"
                className="br-memory-delete"
                onSelect={onDeleteCategory}
              />
            }
          />
        </div>
      </div>

      {isExpanded &&
        category.entries.map((entry) => (
          <div
            key={entry.index}
            className="biorouter-settings-row br-setting-row br-memory-entry br-enter"
          >
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
              <p className="min-w-0 whitespace-pre-wrap break-words text-secondary text-text-default [overflow-wrap:anywhere]">
                {entry.content}
              </p>
            </div>
            <div className="br-setting-row-control">
              <RowActions
                primary={
                  <IconAction
                    icon={Trash2}
                    label={`Delete this memory from ${category.name}`}
                    tooltip="Delete memory"
                    className="br-memory-delete"
                    onSelect={() => onDeleteEntry(entry)}
                  />
                }
              />
            </div>
          </div>
        ))}
    </Fragment>
  );
}
