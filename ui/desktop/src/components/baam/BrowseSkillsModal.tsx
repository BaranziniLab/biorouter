import { useEffect, useMemo, useState } from 'react';
import { Button } from '../ui/button';
import { toastSuccess, toastError } from '../../toasts';
import {
  loadRegistry,
  skillMatches,
  catalogFreshnessLine,
  type BaamRegistry,
  type RegistrySkill,
  type SkillCategory,
} from './registry';
import { installRegistrySkill } from './installSkill';
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

/**
 * The install button's label.
 *
 * ⚠ The count goes INSIDE the conditional along with its trailing space, not
 * beside it. The original wrote `` `Install ${n > 0 ? n : ''} skill…` `` — an
 * empty substitution between two literal spaces — so the button read
 * **"Install  skills"** with a double space in the state it spends most of its
 * life in: nothing selected, and therefore disabled and in front of the user
 * from the moment the dialog opens.
 *
 * Three shapes, all pinned in `BrowseSkillsModal.test.tsx`: 0 → "Install
 * skills" (plural, because it is an invitation, not a count), 1 → "Install 1
 * skill", n → "Install n skills".
 */
export function installButtonLabel(selectedCount: number): string {
  const count = selectedCount > 0 ? `${selectedCount} ` : '';
  return `Install ${count}skill${selectedCount !== 1 ? 's' : ''}`;
}

export default function BrowseSkillsModal({ onClose, onInstalled, installedIds }: Props) {
  const [registry, setRegistry] = useState<BaamRegistry | null>(null);
  const [live, setLive] = useState(false);
  const [fetchedAt, setFetchedAt] = useState<string | undefined>(undefined);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<Filter>('All');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);

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

  const filtered = useMemo(() => {
    if (!registry) return [];
    return registry.skills.filter(
      (s) => (filter === 'All' || s.category === filter) && skillMatches(s, search)
    );
  }, [registry, filter, search]);

  const grouped = useMemo(() => {
    const map = new Map<SkillCategory, RegistrySkill[]>();
    for (const cat of CATEGORY_ORDER) map.set(cat, []);
    for (const s of filtered) map.get(s.category)?.push(s);
    return map;
  }, [filtered]);

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

  const handleInstall = async () => {
    if (installing || !registry || selected.size === 0) return;
    const targets = registry.skills.filter((s) => selected.has(s.id) && !isInstalled(s));
    setInstalling(true);
    setProgress({ done: 0, total: targets.length });

    const failures: string[] = [];
    let done = 0;
    for (const skill of targets) {
      try {
        const res = await installRegistrySkill(skill);
        if (!res.ok) failures.push(`${res.name}: ${res.error ?? 'failed'}`);
      } catch (error) {
        failures.push(
          `${skill.name}: ${error instanceof Error ? error.message : 'installation failed'}`
        );
      }
      done += 1;
      setProgress({ done, total: targets.length });
    }

    setInstalling(false);
    setProgress(null);

    const ok = targets.length - failures.length;
    if (ok > 0) {
      toastSuccess({
        title: `${ok} skill${ok !== 1 ? 's' : ''} installed`,
        msg: 'Added to Biorouter Skills',
      });
      onInstalled();
    }
    if (failures.length > 0) {
      toastError({
        title: `${failures.length} skill${failures.length !== 1 ? 's' : ''} failed`,
        msg: failures[0],
      });
    }
    if (failures.length === 0) onClose();
    else
      setSelected(
        new Set(targets.filter((t) => failures.some((f) => f.startsWith(t.name))).map((t) => t.id))
      );
  };

  const selectedCount = selected.size;

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
            CATEGORY_ORDER.map((cat) => {
              const items = grouped.get(cat) ?? [];
              if (items.length === 0) return null;
              return (
                <div key={cat} className="mb-4">
                  <h3 className="text-xs font-medium text-text-muted uppercase tracking-wider mb-2">
                    {CATEGORY_LABELS[cat]} ({items.length})
                  </h3>
                  <div className="flex flex-col gap-1.5">
                    {items.map((skill) => {
                      const installed = isInstalled(skill);
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
              ? `Installing ${progress.done}/${progress.total}…`
              : `${selectedCount} selected`}
          </span>
          <div className="flex gap-2">
            <Button variant="outline" onClick={onClose} disabled={installing}>
              Cancel
            </Button>
            <Button onClick={handleInstall} disabled={selectedCount === 0 || installing}>
              {installing ? 'Installing…' : installButtonLabel(selectedCount)}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
