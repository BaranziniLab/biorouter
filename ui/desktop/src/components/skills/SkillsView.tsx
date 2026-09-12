import { useCallback, useMemo, useState } from 'react';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { PageHeader } from '../Layout/PageHeader';
import { Button } from '../ui/button';
import { Switch } from '../ui/switch';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { Plus, Upload, Globe, Trash2, ChevronRight } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import SkillItem from './SkillItem';
import BuiltInBadge from '../ui/BuiltInBadge';
import AddSkillModal from './AddSkillModal';
import CustomSkillModal from './CustomSkillModal';
import BrowseSkillsModal from '../baam/BrowseSkillsModal';
import { toastSuccess, toastError } from '../../toasts';
import { SearchView } from '../conversation/SearchView';
import { getSearchShortcutText } from '../../utils/keyboardShortcuts';
import { ReadableContent } from '../Layout/ReadableContent';
import { removeSkillPackage } from '../../api';
import type { CatalogBundle, CatalogSkill } from '../../api';
import { skillCatalogToggleKey, useSkillCatalog, type SkillCatalogEntry } from './useSkillCatalog';
import { isBrowseQuery, rankCatalogEntries } from './searchCatalog';

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
          msg: `The change was not saved: ${result.error}`,
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
    // same way, down to the "Matches (n)" heading.
    if (!isBrowseQuery(searchTerm)) {
      return visible.length > 0 ? [{ key: 'matches', title: 'Matches', entries: visible }] : [];
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
        title: 'Biorouter Skills',
        entries: biorouter,
      });
    for (const [extension, extensionEntries] of [...byExtension].sort()) {
      out.push({
        key: `extension:${extension}`,
        title: `From ${extension}`,
        entries: extensionEntries,
      });
    }
    if (other.length)
      out.push({
        key: 'other',
        title: 'Skills From Other Agents',
        entries: other,
      });
    if (project.length)
      out.push({
        key: 'project',
        title: 'From This Project',
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
        msg: entry.kind === 'bundle' ? 'Package removed' : 'Skill deleted',
      });
      await reload(true);
    } catch (err) {
      toastError({
        title: 'Delete failed',
        msg: err instanceof Error ? err.message : 'Could not remove it',
      });
    } finally {
      setIsDeleting(false);
      setPendingDelete(null);
    }
  };

  const installedIds = useMemo(
    () =>
      new Set(
        entries
          .flatMap((entry) =>
            entry.kind === 'single'
              ? [entry.skill.name, lastPathComponent(entry.skill.slug)]
              : [entry.bundle.name, entry.bundle.displayName]
          )
          .map((value) => value.toLowerCase())
          .filter(Boolean)
      ),
    [entries]
  );

  return (
    <MainPanelLayout>
      <div
        className="flex flex-col min-w-0 flex-1 overflow-y-auto relative"
        data-search-scroll-area
      >
        {/* The one page header. The hairline it draws is full-bleed because the
            wrapper carrying it sits OUTSIDE the reading column — which is the
            defect this view used to be the only holder of: a `border-b` on the
            `ReadableContent` itself stopped the rule at the measure while every
            sibling view's ran edge to edge. */}
        <PageHeader
          title="Skills"
          // Written as one template string rather than JSX text: the sentence is
          // interrupted by a keyboard shortcut, and JSX drops the whitespace
          // around a line break next to an expression, so the spacing here was
          // being held up by an explicit `{' '}` that any reflow could lose.
          description={`Reusable instruction sets that guide Biorouter's behavior. ${getSearchShortcutText()} to search.`}
          actions={
            <>
              <Button variant="default" onClick={() => setIsAddModalOpen(true)}>
                <Upload className="h-4 w-4" />
                Add skill
              </Button>
              <Button variant="outline" onClick={() => setIsBrowseModalOpen(true)}>
                <Globe className="h-4 w-4" />
                Browse skills
              </Button>
              <Button variant="outline" onClick={() => setIsCustomModalOpen(true)}>
                <Plus className="h-4 w-4" />
                Add custom skill
              </Button>
            </>
          }
        />

        <SearchView
          onSearch={(term, _caseSensitive) => setSearchTerm(term)}
          placeholder="Search skills..."
        >
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
                <div key={group.key} className="min-w-0">
                  {/* ⚠ The gap between groups lives on the wrapper above, not on
                      this header. It was `mt-6 … first:mt-0` here, and the
                      header is ALWAYS the first child of its group's div — so
                      `first:mt-0` (0,2,0) beat `mt-6` (0,1,0) for every group
                      and the rhythm it was written for never rendered. */}
                  <h2 className="text-caps text-text-muted mb-3 flex min-w-0 items-center gap-2">
                    {/* Punctuation, not semantics — see the note this replaced:
                        two group markers once carried different hues in the same
                        role, which a reader could not recover a meaning for. */}
                    <span className="inline-block h-1.5 w-1.5 shrink-0 rounded-full bg-background-strong" />
                    <span className="min-w-0 truncate">
                      {group.title} ({group.entries.length})
                    </span>
                  </h2>
                  <div className="biorouter-list-shell">
                    {group.entries.map((entry) => {
                      // ⚠ Per ENTRY, not per group. A skill an installed
                      // extension supplies is not the user's to delete — the
                      // extension would put it back — and under a query every
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
                          onOpen={() =>
                            void window.electron.openDirectoryInExplorer(entry.bundle.directory)
                          }
                          onDelete={fromExtension ? undefined : () => setPendingDelete(entry)}
                          onToggle={(enabled) => void toggle(entry, enabled)}
                        />
                      ) : (
                        <SkillItem
                          key={entry.key}
                          skill={entry.skill}
                          enabled={entry.enabled}
                          onClick={() =>
                            void window.electron.openDirectoryInExplorer(entry.skill.directory)
                          }
                          onDelete={fromExtension ? undefined : () => setPendingDelete(entry)}
                          onShare={() => void copySkill(entry.skill)}
                          onToggle={(enabled) => void toggle(entry, enabled)}
                        />
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>

            {!catalog.loading &&
              !catalog.error &&
              total === 0 &&
              (searchTerm ? (
                <EmptyState
                  icon={ENTITY_ICONS.skill}
                  title="No matching skills"
                  description="Try a different name, description or package."
                  compact
                />
              ) : (
                <EmptyState
                  icon={ENTITY_ICONS.skill}
                  title="No skills yet"
                  description="Add a skill from a repository or a .zip file, browse the ones Biorouter publishes, or write your own."
                  actions={
                    <Button onClick={() => setIsAddModalOpen(true)}>
                      <Upload className="h-4 w-4" />
                      Add skill
                    </Button>
                  }
                />
              ))}
          </ReadableContent>
        </SearchView>
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
            ? `Delete package "${pendingDelete.bundle.displayName}"?`
            : `Delete "${pendingDelete ? displayNameOf(pendingDelete) : ''}"?`
        }
        message={
          pendingDelete?.kind === 'bundle'
            ? `This will permanently remove all ${pendingDelete.bundle.skills.length} skills in this package. This action cannot be undone.`
            : 'This will permanently remove the skill folder from disk. This action cannot be undone.'
        }
        confirmLabel={pendingDelete?.kind === 'bundle' ? 'Delete package' : 'Delete'}
        cancelLabel="Cancel"
        confirmVariant="destructive"
        isSubmitting={isDeleting}
        onConfirm={confirmDelete}
        onCancel={() => setPendingDelete(null)}
      />
    </MainPanelLayout>
  );
}

// ---------------------------------------------------------------------------

/**
 * Loading is rows that are the shape of rows, not a "Loading skills…" line in
 * the middle of an empty column — the same construction the Scheduler uses.
 */
function SkillRowSkeleton() {
  return (
    <div className="biorouter-list-row flex items-start gap-3 px-3 py-3">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-40" />
        <Skeleton className="mt-2 h-3 w-64" />
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
    toastSuccess({ title: skill.name, msg: 'SKILL.md copied to clipboard' });
  } catch {
    toastError({ title: 'Copy failed', msg: 'Could not copy to clipboard' });
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
 * rather than N unrelated rows — so the row carries the package's own name, its
 * version and entry point when a manifest declared them, and opens to show each
 * component with its group.
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
  // directory; `CatalogSkill.builtin` gates the Trash on a member and reaches
  // nothing here.
  //
  // ⚠ Defence in depth: the one shipped bundle is a Context, and
  // `pickerBundles` removes Contexts before this component sees a row, so on
  // today's data this cannot fire. It is here for a seeded bundle that is not
  // a Context — and the refusal that actually holds on every surface lives in
  // the daemon, in `skill_package::refuse_shipped`.
  const builtin = bundle.builtin;
  return (
    <div className="biorouter-list-row group px-3 py-3">
      <div className="flex items-start gap-2">
        {/* A disclosure trigger, drawn the way the swept settings surfaces draw
            theirs (`settings/memory/MemorySection.tsx`): a bare glyph inside a
            plain button, not the `Button` primitive. `variant="ghost"
            shape="round"` is for the row's TRAILING actions; a 32px tinted box
            at the leading edge would also break the `ml-7` the expanded list
            below is indented by, which is this button's own width plus the
            row's gap. */}
        <button
          type="button"
          onClick={onExpandToggle}
          className="mt-0.5 shrink-0 cursor-pointer rounded-inner p-0.5 text-text-muted hover:text-text-default focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
          aria-expanded={expanded}
          aria-label={`${expanded ? 'Collapse' : 'Expand'} ${bundle.displayName}`}
        >
          <ChevronRight
            className={`h-4 w-4 transition-transform duration-[var(--motion-fast)] ${
              expanded ? 'rotate-90' : ''
            }`}
          />
        </button>
        <button
          type="button"
          className="min-w-0 flex-1 cursor-pointer rounded-inner text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
          onClick={onOpen}
          aria-label={`Open skill package ${bundle.displayName}`}
        >
          {/* ⚠ `min-w-0` on the name and `shrink-0` on the two metadata spans,
              not the other way round. A flex item's `min-width: auto` resolves
              to its min-content width, so without this the version and the skill
              count are what give — the count wrapping or being clipped by a long
              package name, at the 704px the chat measure leaves for content. */}
          <div className="flex min-w-0 items-center gap-1.5">
            <p className="min-w-0 truncate text-label text-text-default">{bundle.displayName}</p>
            {bundle.package?.version && (
              <span className="shrink-0 text-supporting text-text-subtle">
                {bundle.package.version}
              </span>
            )}
            <span className="shrink-0 text-supporting text-text-subtle">
              · {bundle.skills.length} skill{bundle.skills.length === 1 ? '' : 's'}
            </span>
          </div>
          {entryPoint && (
            <p className="mt-0.5 truncate text-supporting text-text-subtle">
              entry point: {entryPoint}
            </p>
          )}
          {!expanded && (
            // ⚠ NOT `font-mono`. These are skill NAMES, and `entryPoint` three
            // lines above is one of them — so a collapsed package card printed
            // the same string ("hyperframes") twice, in two typefaces, both
            // visible at once. Expanding the row rendered those same names in
            // the body font again (the <li>s below), so the face also flipped
            // on expand.
            // D-31 in styles/main.css settles it: "mono keeps the jobs it
            // EARNS — code, the terminal, paths, figures where columns must
            // align… Mono for data, sans for chrome." A skill name is a name,
            // and every other skill-name render in the app (SkillItem, the
            // composer picker, the @-mention list, BrowseSkillsModal) is body.
            <p className="mt-1 truncate text-supporting text-text-subtle">
              {bundle.skills.join(' · ')}
            </p>
          )}
        </button>
        <div className="mt-0.5 flex shrink-0 items-center gap-2">
          {builtin && <BuiltInBadge />}
          {onDelete && !builtin && (
            <div
              className="flex gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
              onClick={(e) => e.stopPropagation()}
            >
              {/* A row-trailing glyph-only action is `ghost` + `round` (V7), the
                  one 32×32 rung row actions share. It was `size="sm"` — a 28px
                  PILL — beside the 32px round controls on the skill rows next to
                  it, which is the off-ladder 28px V7 exists to retire. */}
              <Button
                variant="ghost"
                shape="round"
                className="text-text-danger"
                onClick={onDelete}
                title="Delete package"
                aria-label={`Delete skill package ${bundle.displayName}`}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          )}
          <div onClick={(e) => e.stopPropagation()}>
            <Switch
              checked={enabled}
              onCheckedChange={onToggle}
              variant="mono"
              aria-label={`${enabled ? 'Disable' : 'Enable'} ${bundle.displayName}`}
            />
          </div>
        </div>
      </div>

      {expanded && (
        <ul className="mt-2 ml-7 flex flex-col gap-1">
          {members.map((member) => (
            <li key={member.name} className="min-w-0">
              <p className="text-supporting text-text-default truncate">
                {member.name === entryPoint && <span className="text-text-subtle">→ </span>}
                {member.name}
                {groupOf(bundle, member.name) && (
                  <span className="text-text-subtle"> [{groupOf(bundle, member.name)}]</span>
                )}
              </p>
              {member.description && (
                <p className="text-supporting text-text-subtle truncate">{member.description}</p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function groupOf(bundle: CatalogBundle, name: string): string | null {
  const groups = (bundle.package?.groups ?? {}) as Record<string, unknown>;
  for (const [group, names] of Object.entries(groups)) {
    if (Array.isArray(names) && names.includes(name)) return group;
  }
  return null;
}
