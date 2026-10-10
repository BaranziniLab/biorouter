import { useEffect, useMemo, useState } from 'react';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Checkbox } from '../ui/Checkbox';
import { toastSuccess } from '../../toasts';
import {
  loadRegistry,
  rankSkills,
  type BaamRegistry,
  type RegistrySkill,
  type SkillCategory,
} from './registry';
import { isBrowseQuery } from './search';
import { installRegistrySkill } from './installSkill';
import {
  installButtonLabel,
  installedToast,
  installProgressLabel,
  registrySkillCount,
  type LandedInstall,
} from './installCopy';
import { reportInstallRun, type FailedRow } from './installReport';
import { MarketplaceDialog, MarketplaceRow, MarketplaceSection } from './MarketplaceDialog';
import { BROWSE_SKILLS_COPY, MARKETPLACE_COPY } from './copy';

interface Props {
  onClose: () => void;
  onInstalled: () => void;
  /** Lowercased ids/names/folder-slugs of skills already on disk. */
  installedIds: Set<string>;
}

const CATEGORY_ORDER: SkillCategory[] = ['Core', 'Developer', 'Biomedical'];
const CATEGORY_LABELS: Record<SkillCategory, string> = {
  Core: 'Core skills',
  Developer: 'Developer & authoring',
  Biomedical: 'Biomedical analysis',
};

type Filter = 'All' | SkillCategory;

export default function BrowseSkillsModal({ onClose, onInstalled, installedIds }: Props) {
  const [registry, setRegistry] = useState<BaamRegistry | null>(null);
  const [live, setLive] = useState(false);
  const [fetchedAt, setFetchedAt] = useState<string | undefined>(undefined);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<Filter>('All');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<{
    name: string;
    position: number;
    total: number;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadRegistry()
      .then(({ registry, live, fetchedAt }) => {
        if (cancelled) return;
        setRegistry(registry);
        setLive(live);
        setFetchedAt(fetchedAt);
      })
      .catch(() => !cancelled && setLoadError(true));
    return () => {
      cancelled = true;
    };
  }, []);

  const isInstalled = (s: RegistrySkill) =>
    installedIds.has(s.id.toLowerCase()) || installedIds.has(s.name.toLowerCase());

  // ⚠ **A selection ends when its row is seen installed: withdrawn, not hidden.**
  // When another window (or the agent) installed a row this dialog had
  // selected, its id stayed in `selected` and only the checkbox hid it while the
  // row was installed — so removing the package again brought the check back
  // by itself: Read QC checked, "1 selected", "Install 7 skills", with nobody
  // having touched it. What the user selected it for has happened; installing it
  // again is a new decision. So `selected` never holds an installed row, and
  // this restores that during render — before any frame shows it — whatever put
  // the id there: a catalog update, or a failed run re-selecting a row another
  // window installed meanwhile. It converges in one pass: the pruned set holds
  // no installed row, so the condition is false on the next render.
  const installedSelection = (registry?.skills ?? []).filter(
    (s) => selected.has(s.id) && isInstalled(s)
  );
  if (installedSelection.length > 0) {
    const withdrawn = new Set(installedSelection.map((s) => s.id));
    setSelected(new Set([...selected].filter((id) => !withdrawn.has(id))));
  }

  /** Best match first under a query; registry order when there is none. */
  const filtered = useMemo(() => {
    if (!registry) return [];
    const inCategory = registry.skills.filter((s) => filter === 'All' || s.category === filter);
    return rankSkills(inCategory, search).hits.map((hit) => hit.entry);
  }, [registry, filter, search]);

  /**
   * Browsing groups the catalog under its category headings. A search is one
   * list in rank order instead: under the headings, a Core skill matching one
   * word of the query would sit above a Biomedical skill matching all of them.
   */
  const sections = useMemo(() => {
    if (!isBrowseQuery(search)) {
      return filtered.length > 0
        ? [{ key: 'matches', label: BROWSE_SKILLS_COPY.matches, items: filtered }]
        : [];
    }
    return CATEGORY_ORDER.map((cat) => ({
      key: cat,
      label: CATEGORY_LABELS[cat],
      items: filtered.filter((s) => s.category === cat),
    })).filter((section) => section.items.length > 0);
  }, [filtered, search]);

  const selectableFiltered = filtered.filter((s) => !isInstalled(s));
  const allFilteredSelected =
    selectableFiltered.length > 0 && selectableFiltered.every((s) => selected.has(s.id));

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const toggleAllFiltered = () =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (allFilteredSelected) selectableFiltered.forEach((s) => next.delete(s.id));
      else selectableFiltered.forEach((s) => next.add(s.id));
      return next;
    });

  /** The selected rows an install would act on — everything selected but not already on disk. */
  const targets = registry
    ? registry.skills.filter((s) => selected.has(s.id) && !isInstalled(s))
    : [];
  /** What the install button promises: skills, as each selected row says it holds. */
  const selectedSkillCount = targets.reduce((sum, s) => sum + registrySkillCount(s), 0);

  const handleInstall = async () => {
    if (installing || targets.length === 0) return;
    setInstalling(true);

    const landed: LandedInstall[] = [];
    const failures: FailedRow[] = [];
    for (const [index, skill] of targets.entries()) {
      setProgress({ name: skill.name, position: index + 1, total: targets.length });
      try {
        const res = await installRegistrySkill(skill);
        if (!res.ok) {
          failures.push({ id: skill.id, name: skill.name, error: res.error ?? 'failed' });
        } else if (res.installed && res.installed.length > 0) {
          // The daemon's account of what landed, not the row's claim.
          landed.push(
            ...res.installed.map((unit) => ({
              name: unit.name,
              skills: unit.skills.length,
              isPackage: unit.kind === 'bundle',
              replaced: unit.replaced,
            }))
          );
        } else {
          const skills = registrySkillCount(skill);
          landed.push({ name: skill.name, skills, isPackage: skills > 1 });
        }
      } catch (error) {
        failures.push({
          id: skill.id,
          name: skill.name,
          error: error instanceof Error ? error.message : 'installation failed',
        });
      }
    }

    setInstalling(false);
    setProgress(null);

    if (landed.length > 0) {
      toastSuccess(installedToast(landed));
      onInstalled();
    }
    // Every run, not only a failing one: a retry that lands must take back the
    // report that said it had not. See `installReport.ts`.
    reportInstallRun({
      attempted: new Set(targets.map((skill) => skill.id)),
      failures,
      isInstalled: (id) => {
        const row = registry?.skills.find((skill) => skill.id === id);
        return row ? isInstalled(row) : false;
      },
    });
    if (failures.length === 0) onClose();
    // Keep exactly the rows that failed selected, by id. Matching the failure
    // text's prefix against a name re-selected "Alignment" when "Alignment
    // Files" failed.
    else setSelected(new Set(failures.map((failure) => failure.id)));
  };

  // What an install would act on. The same number as `selected.size` now that an
  // installed row is withdrawn from the selection (above); counted from
  // `targets` so the footer and the install can never disagree.
  const selectedCount = targets.length;

  return (
    <MarketplaceDialog
      title={BROWSE_SKILLS_COPY.title}
      help={BROWSE_SKILLS_COPY.help}
      live={live}
      fetchedAt={fetchedAt}
      search={search}
      onSearchChange={setSearch}
      searchLabel={BROWSE_SKILLS_COPY.searchLabel}
      status={loadError ? 'error' : registry ? 'ready' : 'loading'}
      empty={filtered.length === 0}
      emptyText={BROWSE_SKILLS_COPY.empty}
      busy={installing}
      onClose={onClose}
      toolbar={
        <div className="flex items-center gap-2">
          <div
            role="group"
            aria-label={BROWSE_SKILLS_COPY.filterLabel}
            className="flex min-w-0 flex-1 flex-wrap items-center gap-2"
          >
            {(['All', ...CATEGORY_ORDER] as Filter[]).map((f) => {
              const on = filter === f;
              return (
                <Badge
                  key={f}
                  variant="chip"
                  asChild
                  className={
                    on
                      ? 'tint-selected tint-interactive text-text-default'
                      : 'tint-interactive text-text-muted'
                  }
                >
                  <button type="button" aria-pressed={on} onClick={() => setFilter(f)}>
                    {f === 'All' ? BROWSE_SKILLS_COPY.all : CATEGORY_LABELS[f]}
                  </button>
                </Badge>
              );
            })}
          </div>
          {/* Counts ROWS, on purpose: it checks boxes, and a package is one box.
              The install button is what translates a selection into skills. */}
          {selectableFiltered.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              className="shrink-0"
              onClick={toggleAllFiltered}
              disabled={installing}
            >
              {allFilteredSelected
                ? BROWSE_SKILLS_COPY.clearSelection
                : BROWSE_SKILLS_COPY.selectAll(selectableFiltered.length)}
            </Button>
          )}
        </div>
      }
      footer={
        <>
          <span className="mr-auto text-supporting text-text-muted tabular-nums" aria-live="polite">
            {progress
              ? installProgressLabel(progress.name, progress.position, progress.total)
              : BROWSE_SKILLS_COPY.selected(selectedCount)}
          </span>
          <Button variant="secondary" onClick={onClose} disabled={installing}>
            {BROWSE_SKILLS_COPY.cancel}
          </Button>
          <Button onClick={handleInstall} disabled={targets.length === 0 || installing}>
            {installing ? BROWSE_SKILLS_COPY.installing : installButtonLabel(selectedSkillCount)}
          </Button>
        </>
      }
    >
      {sections.map(({ key, label, items }) => (
        <MarketplaceSection key={key} label={label} count={items.length}>
          {items.map((skill) => {
            const installed = isInstalled(skill);
            // Never true for an installed row: see the withdrawal above.
            const checked = selected.has(skill.id);
            return (
              <MarketplaceRow
                key={skill.id}
                as="label"
                disabled={installed || installing}
                leading={
                  <Checkbox
                    // Named by the skill alone; the row's other text is its context.
                    aria-label={skill.name}
                    checked={checked}
                    disabled={installed || installing}
                    onChange={() => toggle(skill.id)}
                  />
                }
                title={skill.name}
                badges={installed && <Badge tone="neutral">{MARKETPLACE_COPY.installed}</Badge>}
                // NOT `font-mono`. Registry `type` values are English phrases —
                // "5 skills · auto-applied", "User-invocable · /scientific-research"
                // — not machine tokens, and this sits inline beside `skill.name`
                // in the body font on the same row. D-31 in styles/main.css: mono
                // for data, sans for chrome.
                meta={skill.type}
                description={skill.description}
              />
            );
          })}
        </MarketplaceSection>
      ))}
    </MarketplaceDialog>
  );
}
