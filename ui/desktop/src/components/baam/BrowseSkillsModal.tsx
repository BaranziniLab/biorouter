import { useEffect, useMemo, useState } from 'react';
import { Button } from '../ui/button';
import { toastSuccess } from '../../toasts';
import {
  loadRegistry,
  rankSkills,
  catalogFreshnessLine,
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
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog';

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
      return filtered.length > 0 ? [{ key: 'matches', label: 'Matches', items: filtered }] : [];
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
    <Dialog open onOpenChange={(open) => !open && !installing && onClose()}>
      <DialogContent
        dismissible={!installing}
        className="flex max-h-[86vh] w-[720px] max-w-[92vw] flex-col gap-0 overflow-hidden p-0 sm:max-w-[92vw] lg:max-w-[720px]"
      >
        {/* Header */}
        <div className="px-6 pt-5 pb-4 pr-14 border-b border-border-subtle">
          <div>
            <DialogTitle>Browse skills</DialogTitle>
            <DialogDescription className="text-xs text-text-muted mt-0.5">
              Install skills from the Biorouter marketplace. Select as many as you like. Skills need
              no setup.
              {catalogFreshnessLine({ live, fetchedAt }) && (
                <span className="text-text-subtle">
                  {' '}
                  · {catalogFreshnessLine({ live, fetchedAt })}
                </span>
              )}
            </DialogDescription>
          </div>
        </div>

        {/* Controls */}
        <div className="px-6 pt-4 pb-3 flex flex-col gap-3 border-b border-border-subtle">
          <input
            type="text"
            autoFocus
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search skills by name, description, or tag…"
            className="biorouter-modal-panel w-full rounded-lg px-3 py-2 text-sm"
          />
          <div className="flex items-center gap-2 flex-wrap">
            {(['All', ...CATEGORY_ORDER] as Filter[]).map((f) => (
              <button
                key={f}
                onClick={() => setFilter(f)}
                className={[
                  'text-xs px-2.5 py-1 rounded-full transition-colors',
                  filter === f
                    ? 'bg-background-info/15 text-text-default'
                    : 'bg-background-muted/60 text-text-muted hover:bg-background-medium',
                ].join(' ')}
              >
                {f === 'All' ? 'All' : CATEGORY_LABELS[f]}
              </button>
            ))}
            <div className="flex-1" />
            {/* Counts ROWS, on purpose: it checks boxes, and a package is one box.
                The install button is what translates a selection into skills. */}
            {selectableFiltered.length > 0 && (
              <button
                onClick={toggleAllFiltered}
                className="text-xs text-text-muted underline hover:text-text-default"
              >
                {allFilteredSelected
                  ? 'Clear selection'
                  : `Select all (${selectableFiltered.length})`}
              </button>
            )}
          </div>
        </div>

        {/* List */}
        <div className="flex-1 overflow-y-auto px-6 py-3">
          {loadError && (
            <p className="text-sm text-text-danger text-center mt-10">
              Could not load the marketplace catalog.
            </p>
          )}
          {!registry && !loadError && (
            <p className="text-sm text-text-muted text-center mt-10 animate-pulse">
              Loading catalog…
            </p>
          )}
          {registry && filtered.length === 0 && (
            <p className="text-sm text-text-muted text-center mt-10">
              No skills match your search.
            </p>
          )}
          {registry &&
            sections.map(({ key, label, items }) => {
              return (
                <div key={key} className="mb-4">
                  <h3 className="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">
                    {label} ({items.length})
                  </h3>
                  <div className="flex flex-col gap-1.5">
                    {items.map((skill) => {
                      const installed = isInstalled(skill);
                      // Never true for an installed row: see the withdrawal above.
                      const checked = selected.has(skill.id);
                      return (
                        <label
                          key={skill.id}
                          className={[
                            'flex items-start gap-3 rounded-lg px-3 py-2.5 transition-colors',
                            installed
                              ? 'biorouter-modal-row opacity-70 cursor-default'
                              : checked
                                ? 'bg-background-info/10 ring-1 ring-background-info/30 cursor-pointer'
                                : 'biorouter-modal-row hover:bg-background-default cursor-pointer',
                          ].join(' ')}
                        >
                          <input
                            type="checkbox"
                            className="mt-1 accent-[var(--background-info)]"
                            checked={checked}
                            disabled={installed || installing}
                            onChange={() => toggle(skill.id)}
                          />
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-2 flex-wrap">
                              <span className="text-sm font-medium text-text-default">
                                {skill.name}
                              </span>
                              {skill.type && (
                                // NOT `font-mono`. Registry `type` values are
                                // English phrases — "5 skills · auto-applied",
                                // "User-invocable · /scientific-research" — not
                                // machine tokens, and this sits inline beside
                                // `skill.name` in the body font on the same row.
                                // D-31 in styles/main.css: mono for data, sans
                                // for chrome.
                                <span className="text-[11px] text-text-subtle">{skill.type}</span>
                              )}
                              {installed && (
                                <span className="text-[10px] uppercase tracking-wide text-background-info bg-background-info/10 rounded px-1.5 py-0.5">
                                  Installed
                                </span>
                              )}
                            </div>
                            <p className="text-xs text-text-muted mt-0.5 leading-relaxed">
                              {skill.description}
                            </p>
                            {skill.tags.length > 0 && (
                              <div className="flex gap-1 flex-wrap mt-1.5">
                                {skill.tags.map((t) => (
                                  <span
                                    key={t}
                                    className="text-[10px] text-text-subtle bg-background-medium rounded px-1.5 py-0.5"
                                  >
                                    {t}
                                  </span>
                                ))}
                              </div>
                            )}
                          </div>
                        </label>
                      );
                    })}
                  </div>
                </div>
              );
            })}
        </div>

        {/* Footer */}
        <div className="px-6 py-4 border-t border-border-subtle flex items-center justify-between gap-3">
          <span className="text-xs text-text-muted">
            {progress
              ? installProgressLabel(progress.name, progress.position, progress.total)
              : `${selectedCount} selected`}
          </span>
          <div className="flex gap-2">
            <Button variant="outline" onClick={onClose} disabled={installing}>
              Cancel
            </Button>
            <Button onClick={handleInstall} disabled={targets.length === 0 || installing}>
              {installing ? 'Installing…' : installButtonLabel(selectedSkillCount)}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
