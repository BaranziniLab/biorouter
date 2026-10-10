import * as React from 'react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  skillCatalogToggleKey,
  useSkillCatalog,
  type SkillCatalogEntry,
} from '../skills/useSkillCatalog';
import { rankCatalogEntries } from '../skills/searchCatalog';
import { toastService } from '../../toasts';
import BuiltInBadge from '../ui/BuiltInBadge';
import { uiCopy } from '../ui/copy';
import { SKILLS_COPY, TOOLS_COPY } from './copy';
import { PickerPanel, PickerRow } from './PickerPanel';

/** What the skills list needs from its picker state (see `useSkillPicker`). */
export interface SkillPicker {
  sessionId: string | null;
  /** Enabled skills, the chip's share of the Tools count. */
  activeCount: number;
  query: string;
  setQuery: (query: string) => void;
  entries: SkillCatalogEntry[];
  toggle: (entry: SkillCatalogEntry) => void;
  bulk: { label: string; onClick: () => void } | null;
  emptyText: string;
}

/**
 * The composer's skill list: state and toggles, without any trigger.
 *
 * ⚠ **Every toggle here goes to the daemon and waits for its answer** (#113).
 * The session branch used to write React state and raise a success toast
 * reading "<skill> enabled for this chat" without calling anything. A control
 * that reports intent as if it were state is worse than no control, so a
 * refusal puts the switch back and says why. There is no success toast at all
 * now (decision 6.9): the switch is the confirmation.
 *
 * The inventory is fetched rather than scanned, for the reason recorded on
 * `useSkillCatalog`: a three-root scan omitted skills bundled inside installed
 * extensions, which the backend has always loaded.
 *
 * @param open whether the list is on screen. Opening it asks the daemon to
 *   rescan, the cheapest honest moment to notice an install made elsewhere.
 */
export function useSkillPicker(sessionId: string | null, { open }: { open: boolean }): SkillPicker {
  const [searchQuery, setSearchQuery] = useState('');
  const catalog = useSkillCatalog(sessionId);
  const { entries, reload, setEnabled } = catalog;

  useEffect(() => {
    if (open) void reload(true);
  }, [open, reload]);

  const applyToggle = useCallback(
    async (keys: string[], enabled: boolean) => {
      const result = await setEnabled(keys, enabled);
      if (result.ok) return;
      // The reason travels back with the result rather than through hook
      // state, which this callback's captured render would have read stale.
      toastService.error({
        title: SKILLS_COPY.updateFailedTitle,
        msg: SKILLS_COPY.updateFailed(result.error),
      });
    },
    [setEnabled]
  );

  const toggle = useCallback(
    (entry: SkillCatalogEntry) => {
      void applyToggle([skillCatalogToggleKey(entry)], !entry.enabled);
    },
    [applyToggle]
  );

  // One matcher, shared with Settings -> Skills, the Browse modals and the
  // model's own search — see `skills/searchCatalog.ts`. This filter used to ask
  // whether the WHOLE query occurred inside one field (QA finding F5), so a
  // phrase naming two installed skills found neither. The list is flat, so the
  // ranking reaches it directly: best match first, catalog order when nothing
  // is typed.
  //
  // ⚠ It also feeds "Enable all", which writes every row left on screen — so a
  // filter that leaks (`R` matching the letter anywhere) is a bulk write over
  // rows the user never asked about, not just a long list.
  const filteredEntries = useMemo(
    () => rankCatalogEntries(entries, searchQuery).hits.map((hit) => hit.entry),
    [entries, searchQuery]
  );

  const activeCount = useMemo(() => entries.filter((e) => e.enabled).length, [entries]);
  const visibleEnabledCount = useMemo(
    () => filteredEntries.filter((e) => e.enabled).length,
    [filteredEntries]
  );

  const handleBulkToggle = useCallback(() => {
    if (filteredEntries.length === 0) return;
    const targetEnabled = visibleEnabledCount === 0;
    const keys = [
      ...new Set(
        filteredEntries.filter((e) => e.enabled !== targetEnabled).map(skillCatalogToggleKey)
      ),
    ];
    if (keys.length === 0) return;
    void applyToggle(keys, targetEnabled);
  }, [applyToggle, filteredEntries, visibleEnabledCount]);

  const emptyText = catalog.error
    ? catalog.error
    : catalog.loading
      ? SKILLS_COPY.loading
      : searchQuery
        ? SKILLS_COPY.noMatch
        : SKILLS_COPY.empty;

  return {
    sessionId,
    activeCount,
    query: searchQuery,
    setQuery: setSearchQuery,
    entries: filteredEntries,
    toggle,
    bulk:
      filteredEntries.length > 0
        ? {
            label:
              visibleEnabledCount === 0
                ? SKILLS_COPY.enableAll(filteredEntries.length)
                : SKILLS_COPY.disableAll(visibleEnabledCount),
            onClick: handleBulkToggle,
          }
        : null,
    emptyText,
  };
}

/** The skills list inside the Tools popover. */
export function SkillsPanel({
  picker,
  inputRef,
}: {
  picker: SkillPicker;
  inputRef?: React.Ref<HTMLInputElement>;
}) {
  return (
    <PickerPanel
      data-testid="tools-skills-panel"
      listLabel={SKILLS_COPY.listLabel}
      searchPlaceholder={SKILLS_COPY.search}
      query={picker.query}
      onQueryChange={picker.setQuery}
      scopeLabel={picker.sessionId ? TOOLS_COPY.scope.thisChat : TOOLS_COPY.scope.newChats}
      scopeHelp={picker.sessionId ? SKILLS_COPY.helpChat : SKILLS_COPY.helpNewChats}
      bulk={picker.bulk}
      emptyText={picker.emptyText}
      inputRef={inputRef}
    >
      {picker.entries.map((entry) => (
        <SkillRow key={entry.key} entry={entry} onToggle={picker.toggle} />
      ))}
    </PickerPanel>
  );
}

function SkillRow({
  entry,
  onToggle,
}: {
  entry: SkillCatalogEntry;
  onToggle: (entry: SkillCatalogEntry) => void;
}) {
  if (entry.kind === 'single') {
    const { skill, enabled } = entry;
    // `label` is the extension's own name when the skill was bundled inside one:
    // provenance the picker could not show while it scanned only three roots.
    const from = skill.source.kind === 'extension' ? skill.source.label : undefined;
    return (
      <PickerRow
        name={skill.name}
        checked={enabled}
        onToggle={() => onToggle(entry)}
        badge={skill.builtin ? <BuiltInBadge /> : undefined}
        meta={from}
        // A shipped skill with no description still explains its badge.
        description={skill.description || (skill.builtin ? uiCopy.builtInHelp : undefined)}
      />
    );
  }

  // A bundle: its count beside the name, its members in the tooltip.
  const { bundle, enabled } = entry;
  return (
    <PickerRow
      name={bundle.displayName}
      checked={enabled}
      onToggle={() => onToggle(entry)}
      meta={SKILLS_COPY.bundleCount(bundle.skills.length)}
      description={SKILLS_COPY.bundleMembers(bundle.skills)}
    />
  );
}
