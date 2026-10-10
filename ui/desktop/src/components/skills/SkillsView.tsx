import { useCallback, useId, useMemo, useState } from 'react';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { Button } from '../ui/button';
import { Switch } from '../ui/switch';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { FilterInput } from '../ui/filter-input';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import {
  RowActionMenuItems,
  RowActions,
  RowContextMenu,
  type RowActionItem,
} from '../ui/row-actions';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { Plus, ChevronRight } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import SkillItem from './SkillItem';
import BuiltInBadge from '../ui/BuiltInBadge';
import AddSkillModal from './AddSkillModal';
import CustomSkillModal from './CustomSkillModal';
import BrowseSkillsModal from '../baam/BrowseSkillsModal';
import { toastSuccess, toastError } from '../../toasts';
import { removeSkillPackage } from '../../api';
import type { CatalogBundle, CatalogSkill } from '../../api';
import { skillCatalogToggleKey, useSkillCatalog, type SkillCatalogEntry } from './useSkillCatalog';
import { isBrowseQuery, rankCatalogEntries } from './searchCatalog';
import { withoutStagingNonce } from './skillUtils';
import { SKILLS_COPY } from './copy';

/**
 * Settings → Skills.
 *
 * ⚠ **The inventory is the daemon's** (#113). This view used to scan
 * `BIOROUTER_SKILLS_DIR` and `OTHER_SKILL_DIRS` itself — three roots against the
 * backend's seven — so BiorOffice's Word/Excel/PowerPoint skills and
 * MarkItDown's converter were loaded by the model and had no row here at all.
 * There is no scanner left; `useSkillCatalog` fetches, and everything below
 * groups what it returns.
 *
 * Deletion goes to the importer's remover, which renames the directory aside
 * before deleting it — so a package leaves in one step rather than emptying out
 * under a catalog scan in flight.
 */
type Group = {
  key: string;
  title: string;
  entries: SkillCatalogEntry[];
};

export default function SkillsView() {
  const catalog = useSkillCatalog(null);
  const { entries, reload, setEnabled } = catalog;

  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [isCustomModalOpen, setIsCustomModalOpen] = useState(false);
  const [isBrowseModalOpen, setIsBrowseModalOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<SkillCatalogEntry | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [searchTerm, setSearchTerm] = useState('');
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const toggle = useCallback(
    async (entry: SkillCatalogEntry, enabled: boolean) => {
      const result = await setEnabled([skillCatalogToggleKey(entry)], enabled);
      if (!result.ok) {
        toastError({
          title: displayNameOf(entry),
          msg: SKILLS_COPY.notSaved(result.error),
        });
      }
    },
    [setEnabled]
  );

  const groups = useMemo((): Group[] => {
    // One matcher, shared with the composer's picker, the Browse modals and the
    // model's own search — see `searchCatalog.ts`. The filter here used to ask
    // whether the WHOLE query occurred inside one field (QA finding F5).
    const visible = rankCatalogEntries(entries, searchTerm).hits.map((hit) => hit.entry);

    // ⚠ **A search is one ranked list, not the provenance headings.** The
    // headings are a grouping, and a grouping discards the rank: a Biorouter
    // skill matching one word of the query would sit above a project skill
    // matching all of them. `BrowseSkillsModal` resolved the same tension the
    // same way, down to the "Matches" heading.
    if (!isBrowseQuery(searchTerm)) {
      return visible.length > 0
        ? [{ key: 'matches', title: SKILLS_COPY.groups.matches, entries: visible }]
        : [];
    }

    const biorouter = visible.filter((e) => sourceOf(e).kind === 'biorouter');
    const project = visible.filter((e) => sourceOf(e).kind === 'project');
    const other = visible.filter((e) => ['claudeHome', 'agentsHome'].includes(sourceOf(e).kind));

    // One group per extension, so a bundled skill says which extension it came
    // from rather than appearing among the user's own installs.
    const byExtension = new Map<string, SkillCatalogEntry[]>();
    for (const entry of visible) {
      const source = sourceOf(entry);
      if (source.kind !== 'extension') continue;
      const label = source.extension ?? source.label;
      byExtension.set(label, [...(byExtension.get(label) ?? []), entry]);
    }

    const out: Group[] = [];
    if (biorouter.length)
      out.push({
        key: 'biorouter',
        title: SKILLS_COPY.groups.biorouter,
        entries: biorouter,
      });
    for (const [extension, extensionEntries] of [...byExtension].sort()) {
      out.push({
        key: `extension:${extension}`,
        title: SKILLS_COPY.groups.extension(extension),
        entries: extensionEntries,
      });
    }
    if (other.length)
      out.push({
        key: 'other',
        title: SKILLS_COPY.groups.other,
        entries: other,
      });
    if (project.length)
      out.push({
        key: 'project',
        title: SKILLS_COPY.groups.project,
        entries: project,
      });
    return out;
  }, [entries, searchTerm]);

  const total = groups.reduce((sum, group) => sum + group.entries.length, 0);

  const confirmDelete = async () => {
    if (!pendingDelete) return;
    setIsDeleting(true);
    const entry = pendingDelete;
    try {
      await removeSkillPackage<true>({
        body: {
          id: installedIdOf(entry),
          sourceRoot: sourceRootOf(entry),
        },
        throwOnError: true,
      });
      toastSuccess({
        title: displayNameOf(entry),
        msg: entry.kind === 'bundle' ? SKILLS_COPY.packageRemoved : SKILLS_COPY.skillDeleted,
      });
      await reload(true);
    } catch (err) {
      toastError({
        title: SKILLS_COPY.deleteFailed,
        msg: err instanceof Error ? err.message : SKILLS_COPY.deleteFailedFallback,
      });
    } finally {
      setIsDeleting(false);
      setPendingDelete(null);
    }
  };

  /**
   * Every spelling under which Browse skills may recognise something already on
   * disk — the frontmatter name, the folder, and a bundle's display name.
   *
   * ⚠ **Plus the de-nonced alias of each.** Until `utils/registryDownload`, a
   * marketplace download was staged as `<12 hex>-<asset>.zip` and the importer
   * read that stem as the package id for any archive declaring no name of its
   * own — which every BAAM bundle is. So a `single-cell` bundle installed as
   * `d92c1c985d54-single-cell`, matched none of the registry ids the modal
   * compares against, and was offered for install again on every visit. New
   * installs no longer carry the nonce; this alias is what stops the ones
   * already on disk from being re-installed indefinitely. It adds a name, it
   * never removes one: the package keeps the identity it was installed under.
   */
  const installedIds = useMemo(
    () =>
      new Set(
        entries
          .flatMap((entry) =>
            entry.kind === 'single'
              ? [entry.skill.name, lastPathComponent(entry.skill.slug)]
              : [entry.bundle.name, entry.bundle.displayName]
          )
          .flatMap((value) => [value, withoutStagingNonce(value)])
          .filter((value): value is string => Boolean(value))
          .map((value) => value.toLowerCase())
      ),
    [entries]
  );

  const openFolder = (directory: string) => void window.electron.openDirectoryInExplorer(directory);

  return (
    <MainPanelLayout removeTopPadding>
      <div className="relative flex min-w-0 flex-1 flex-col overflow-y-auto">
        {/* The band (spec 3.11): title, help, count, the filter and one Add
            menu. It sits OUTSIDE the reading column, so its hairline runs edge
            to edge; only the body below takes the column. */}
        <PageHeader
          title={SKILLS_COPY.title}
          info={SKILLS_COPY.info}
          adornment={entries.length > 0 ? entries.length : undefined}
          actions={
            <>
              {/* ⚠ **One character is a real query here.** The shared
                  matcher's short-term rule (`baam/search.ts`) holds a term
                  under three characters to whole WORDS, so `R` finds the R
                  skills and not every row holding the letter. The filter
                  reports every keystroke; the old find bar's two-character
                  floor once put that query out of reach. */}
              <FilterInput value={searchTerm} onValueChange={setSearchTerm} />
              <AddSkillMenu
                onBrowse={() => setIsBrowseModalOpen(true)}
                onFromSource={() => setIsAddModalOpen(true)}
                onWrite={() => setIsCustomModalOpen(true)}
              />
            </>
          }
        />

        <ReadableContent size="chat" className="px-6 py-4">
          {catalog.error && (
            <Note tone="danger" role="alert" className="mb-4">
              {catalog.error}
            </Note>
          )}

          {/* ⚠ Guarded on an EMPTY list, not on `loading` alone. `reload` sets
              `loading` after every install, delete and `catalog:changed`
              rescan as well as on first load, so a bare `loading` branch would
              swap the whole list for skeletons every time a switch was
              flipped. The three branches stay mutually exclusive without it:
              skeletons and the empty state both require `total === 0`, and the
              empty state additionally requires the load to have finished. */}
          {catalog.loading && !catalog.error && total === 0 && (
            <div className="biorouter-list-shell" aria-hidden>
              <SkillRowSkeleton />
              <SkillRowSkeleton />
              <SkillRowSkeleton />
            </div>
          )}

          <div className="flex flex-col gap-6">
            {groups.map((group) => (
              <SkillGroup key={group.key} title={group.title} count={group.entries.length}>
                {group.entries.map((entry) => {
                  // ⚠ Per ENTRY, not per group. A skill an installed
                  // extension supplies is not the user's to delete (the
                  // extension would put it back), and under a query every
                  // provenance is in one "Matches" list, so a flag on the
                  // group would offer Delete on rows that must not have it.
                  const fromExtension = sourceOf(entry).kind === 'extension';
                  return entry.kind === 'bundle' ? (
                    <BundleRow
                      key={entry.key}
                      bundle={entry.bundle}
                      skills={catalog.skills}
                      enabled={entry.enabled}
                      expanded={expanded.has(entry.key)}
                      onExpandToggle={() =>
                        setExpanded((current) => {
                          const next = new Set(current);
                          if (next.has(entry.key)) next.delete(entry.key);
                          else next.add(entry.key);
                          return next;
                        })
                      }
                      onOpen={() => openFolder(entry.bundle.directory)}
                      onDelete={fromExtension ? undefined : () => setPendingDelete(entry)}
                      onToggle={(enabled) => void toggle(entry, enabled)}
                    />
                  ) : (
                    <SkillItem
                      key={entry.key}
                      skill={entry.skill}
                      enabled={entry.enabled}
                      onOpen={() => openFolder(entry.skill.directory)}
                      onDelete={fromExtension ? undefined : () => setPendingDelete(entry)}
                      onShare={() => void copySkill(entry.skill)}
                      onToggle={(enabled) => void toggle(entry, enabled)}
                    />
                  );
                })}
              </SkillGroup>
            ))}
          </div>

          {!catalog.loading &&
            !catalog.error &&
            total === 0 &&
            (searchTerm ? (
              <EmptyState
                icon={ENTITY_ICONS.skill}
                title={SKILLS_COPY.noMatchTitle}
                description={SKILLS_COPY.noMatchDescription}
                compact
              />
            ) : (
              <EmptyState
                icon={ENTITY_ICONS.skill}
                title={SKILLS_COPY.emptyTitle}
                description={SKILLS_COPY.emptyDescription}
                actions={
                  <Button variant="link" onClick={() => setIsBrowseModalOpen(true)}>
                    {SKILLS_COPY.browse}
                  </Button>
                }
              />
            ))}
        </ReadableContent>
      </div>

      {isAddModalOpen && (
        <AddSkillModal onClose={() => setIsAddModalOpen(false)} onSaved={() => void reload(true)} />
      )}
      {isCustomModalOpen && (
        <CustomSkillModal
          onClose={() => setIsCustomModalOpen(false)}
          onSaved={() => void reload(true)}
        />
      )}
      {isBrowseModalOpen && (
        <BrowseSkillsModal
          onClose={() => setIsBrowseModalOpen(false)}
          onInstalled={() => void reload(true)}
          installedIds={installedIds}
        />
      )}

      <ConfirmationModal
        isOpen={pendingDelete !== null}
        title={
          pendingDelete?.kind === 'bundle'
            ? SKILLS_COPY.confirmDeletePackageTitle(pendingDelete.bundle.displayName)
            : SKILLS_COPY.confirmDeleteSkillTitle(pendingDelete ? displayNameOf(pendingDelete) : '')
        }
        message={
          pendingDelete?.kind === 'bundle'
            ? SKILLS_COPY.confirmDeletePackageMessage(pendingDelete.bundle.skills.length)
            : SKILLS_COPY.confirmDeleteSkillMessage
        }
        confirmLabel={
          pendingDelete?.kind === 'bundle'
            ? SKILLS_COPY.confirmDeletePackage
            : SKILLS_COPY.confirmDelete
        }
        cancelLabel={SKILLS_COPY.cancel}
        confirmVariant="destructive"
        isSubmitting={isDeleting}
        onConfirm={confirmDelete}
        onCancel={() => setPendingDelete(null)}
      />
    </MainPanelLayout>
  );
}

/**
 * The band's one primary action: a menu of the three ways a skill arrives.
 * Text-only, like every view menu (spec 2.6).
 */
function AddSkillMenu({
  onBrowse,
  onFromSource,
  onWrite,
}: {
  onBrowse: () => void;
  onFromSource: () => void;
  onWrite: () => void;
}) {
  const items: RowActionItem[] = [
    { label: SKILLS_COPY.browse, onSelect: onBrowse },
    { label: SKILLS_COPY.fromSource, onSelect: onFromSource },
    { label: SKILLS_COPY.write, onSelect: onWrite },
  ];
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button>
          <Plus aria-hidden />
          {SKILLS_COPY.add}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <RowActionMenuItems items={items} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * One provenance group: a caps label carrying its count (Crew's form, no dot),
 * then the rows. The label is inset to the rows' text so the two share a left
 * edge.
 */
function SkillGroup({
  title,
  count,
  children,
}: {
  title: string;
  count: number;
  children: React.ReactNode;
}) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="min-w-0">
      <h2 id={headingId} className="mb-1 truncate px-3 text-caps text-text-muted">
        {title} <span className="tabular-nums">{count}</span>
      </h2>
      <div className="biorouter-list-shell">{children}</div>
    </section>
  );
}

// ---------------------------------------------------------------------------

/**
 * Loading is rows that are the shape of rows, not a "Loading skills…" line in
 * the middle of an empty column — the same construction the Scheduler uses.
 */
function SkillRowSkeleton() {
  return (
    <div className="biorouter-list-row flex items-center gap-3 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-40" />
        <Skeleton className="mt-1 h-3 w-64" />
      </div>
    </div>
  );
}

function sourceOf(entry: SkillCatalogEntry) {
  return entry.kind === 'single' ? entry.skill.source : entry.bundle.source;
}

function sourceRootOf(entry: SkillCatalogEntry): string {
  return entry.kind === 'single' ? entry.skill.sourceRoot : entry.bundle.sourceRoot;
}

function displayNameOf(entry: SkillCatalogEntry): string {
  return entry.kind === 'single' ? entry.skill.name : entry.bundle.displayName;
}

/**
 * The directory name to remove.
 *
 * ⚠ The **installed directory**, not the frontmatter name. A skill stored in
 * `run-gwas/` may declare `name: gwas-pipeline`, and the two are allowed to
 * differ — removing by the declared name would miss it.
 */
function installedIdOf(entry: SkillCatalogEntry): string {
  return entry.kind === 'single' ? lastPathComponent(entry.skill.slug) : entry.bundle.name;
}

function lastPathComponent(slug: string): string {
  return slug.split('/').pop() ?? slug;
}

async function copySkill(skill: CatalogSkill) {
  try {
    const result = await window.electron.readFile(`${skill.directory}/SKILL.md`);
    if (!result.found || !result.file) throw new Error('SKILL.md could not be read');
    await navigator.clipboard.writeText(result.file);
    toastSuccess({ title: skill.name, msg: SKILLS_COPY.copied });
  } catch {
    toastError({ title: SKILLS_COPY.copyFailed, msg: SKILLS_COPY.copyFailedMessage });
  }
}

interface BundleRowProps {
  bundle: CatalogBundle;
  skills: CatalogSkill[];
  enabled: boolean;
  expanded: boolean;
  onExpandToggle: () => void;
  onOpen: () => void;
  onDelete?: () => void;
  onToggle: (enabled: boolean) => void;
}

/**
 * One package, expandable.
 *
 * #115 asks for "one expandable bundle in the UI with component details"
 * rather than N unrelated rows. Collapsed, the row is the package's name and
 * one line, "{n} skills · {version}"; the entry point and the members appear
 * only when it is open (spec 3.11). The text block is the disclosure, so the
 * name keeps the left edge every other row's name has.
 */
function BundleRow({
  bundle,
  skills,
  enabled,
  expanded,
  onExpandToggle,
  onOpen,
  onDelete,
  onToggle,
}: BundleRowProps) {
  const titleId = useId();
  const membersId = useId();
  const declaredMembers = new Set(bundle.skills);
  const members = skills.filter(
    (skill) =>
      skill.sourceRoot === bundle.sourceRoot &&
      skill.bundle === bundle.name &&
      declaredMembers.has(skill.name)
  );
  const entryPoint = bundle.package?.entryPoint ?? null;
  // ⚠ From the daemon, not from a list here. Rust owns the seeder, so Rust owns
  // the answer — the same rule `SkillItem` follows for a skill row. A bundle
  // needs its own answer because this is a different control over a different
  // directory; `CatalogSkill.builtin` gates Delete on a member and reaches
  // nothing here.
  //
  // ⚠ Defence in depth: the one shipped bundle is a Context, and
  // `pickerBundles` removes Contexts before this component sees a row, so on
  // today's data this cannot fire. It is here for a seeded bundle that is not
  // a Context — and the refusal that actually holds on every surface lives in
  // the daemon, in `skill_package::refuse_shipped`.
  const builtin = bundle.builtin;
  const menu: RowActionItem[] = [
    { label: SKILLS_COPY.openFolder, onSelect: onOpen },
    ...(onDelete && !builtin
      ? ([
          { kind: 'separator' },
          {
            label: SKILLS_COPY.delete,
            onSelect: onDelete,
            destructive: true,
            testId: 'skill-row-delete',
          },
        ] satisfies RowActionItem[])
      : []),
  ];
  return (
    <RowContextMenu items={menu}>
      <div className="biorouter-list-row px-3 py-2.5" data-skill-row="bundle">
        <div className="flex items-center gap-3">
          {/* The disclosure. A bare text button rather than the `Button`
              primitive: the row's TRAILING actions are the buttons; this is the
              row's own name, opening in place. */}
          <button
            type="button"
            onClick={onExpandToggle}
            aria-expanded={expanded}
            aria-controls={expanded ? membersId : undefined}
            aria-label={
              expanded
                ? SKILLS_COPY.collapse(bundle.displayName)
                : SKILLS_COPY.expand(bundle.displayName)
            }
            className="biorouter-focus-surface min-w-0 flex-1 cursor-pointer rounded-inner text-left"
          >
            {/* ⚠ `min-w-0` on the name, `shrink-0` on the badge and the
                chevron: a flex item's `min-width: auto` is its min-content
                width, so a long package name would otherwise push them out. */}
            <span className="flex min-w-0 items-center gap-1.5">
              <span id={titleId} className="min-w-0 truncate text-label text-text-default">
                {bundle.displayName}
              </span>
              {builtin && <BuiltInBadge />}
              <ChevronRight
                aria-hidden
                className={`h-4 w-4 shrink-0 text-text-muted transition-transform duration-[var(--dur-fast-max)] ease-[var(--ease-out)] ${
                  expanded ? 'rotate-90' : ''
                }`}
              />
            </span>
            {/* NOT `font-mono`: a version and a count are chrome, not data. */}
            <span className="block truncate text-supporting text-text-muted">
              {SKILLS_COPY.bundleSummary(bundle.skills.length, bundle.package?.version)}
            </span>
          </button>
          <RowActions menu={menu} />
          <Switch checked={enabled} onCheckedChange={onToggle} aria-labelledby={titleId} />
        </div>

        {expanded && (
          <div id={membersId} className="mt-2 flex flex-col gap-1">
            {entryPoint && (
              <p className="text-supporting text-text-muted">
                {SKILLS_COPY.entryPoint(entryPoint)}
              </p>
            )}
            {/* Skill NAMES, so the body font, like every other skill-name
                render in the app (D-31: mono for data, sans for chrome). */}
            <ul className="flex flex-col gap-1">
              {members.map((member) => {
                const group = groupOf(bundle, member.name);
                return (
                  <li key={member.name} className="min-w-0 truncate text-supporting">
                    <span className="text-text-default">{member.name}</span>
                    {group && <span className="text-text-muted"> · {group}</span>}
                    {member.description && (
                      <span className="text-text-muted"> · {member.description}</span>
                    )}
                  </li>
                );
              })}
            </ul>
          </div>
        )}
      </div>
    </RowContextMenu>
  );
}

function groupOf(bundle: CatalogBundle, name: string): string | null {
  const groups = (bundle.package?.groups ?? {}) as Record<string, unknown>;
  for (const [group, names] of Object.entries(groups)) {
    if (Array.isArray(names) && names.includes(name)) return group;
  }
  return null;
}
