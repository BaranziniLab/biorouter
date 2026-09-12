import { useCallback, useEffect, useMemo, useState } from 'react';
import { Layers } from '../icons/app-icons';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Input } from '../ui/input';
import { Switch } from '../ui/switch';
import {
  skillCatalogToggleKey,
  useSkillCatalog,
  type SkillCatalogEntry,
} from '../skills/useSkillCatalog';
import { rankCatalogEntries } from '../skills/searchCatalog';
import { toastService } from '../../toasts';
import BuiltInBadge from '../ui/BuiltInBadge';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';

interface BottomMenuSkillSelectionProps {
  sessionId: string | null;
}

/**
 * The composer's skill menu.
 *
 * ⚠ **Every toggle here goes to the daemon and waits for its answer** (#113).
 * The session branch used to write React state and raise a success toast
 * reading "<skill> enabled for this chat" without calling anything — no
 * request, no `extension_data` write, no catalog refresh, no live agent. The
 * switch moved, the toast was green, and the next turn saw the same skills as
 * before. A control that reports intent as if it were state is worse than no
 * control, so the toast is raised only on a confirmed write, and a refusal puts
 * the switch back.
 *
 * The inventory is fetched rather than scanned, for the reason recorded on
 * `useSkillCatalog`: this component's own three-root scan omitted skills
 * bundled inside installed extensions, which the backend has always loaded.
 */
export const BottomMenuSkillSelection = ({ sessionId }: BottomMenuSkillSelectionProps) => {
  const [searchQuery, setSearchQuery] = useState('');
  const [isOpen, setIsOpen] = useState(false);
  const catalog = useSkillCatalog(sessionId);
  const { entries, reload, setEnabled } = catalog;
  const scope = sessionId ? 'for this chat' : 'in new chats';

  // Opening the menu is the cheapest honest moment to notice an install made
  // elsewhere — a marketplace click, `biorouter skill install` in a terminal —
  // so it asks the daemon to rescan rather than trusting the cached snapshot.
  useEffect(() => {
    if (isOpen) void reload(true);
  }, [isOpen, reload]);

  const applyToggle = useCallback(
    async (keys: string[], enabled: boolean, describe: (count: number) => string) => {
      const result = await setEnabled(keys, enabled);
      if (result.ok) {
        toastService.success({
          title: keys.length === 1 ? 'Skill updated' : 'Skills updated',
          msg: describe(keys.length),
        });
        return;
      }
      // The reason travels back with the result rather than through hook
      // state, which this callback's captured render would have read stale.
      toastService.error({
        title: 'Skill update failed',
        msg: `The change was not saved: ${result.error}`,
      });
    },
    [setEnabled]
  );

  const handleToggle = useCallback(
    (key: string, displayName: string, currentlyEnabled: boolean) => {
      void applyToggle(
        [key],
        !currentlyEnabled,
        () => `${displayName} ${!currentlyEnabled ? 'enabled' : 'disabled'} ${scope}`
      );
    },
    [applyToggle, scope]
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
    void applyToggle(
      keys,
      targetEnabled,
      (count) =>
        `${count} skill${count === 1 ? '' : 's'} ${targetEnabled ? 'enabled' : 'disabled'} ${scope}`
    );
  }, [applyToggle, filteredEntries, scope, visibleEnabledCount]);

  const emptyMessage = () => {
    if (catalog.error) return catalog.error;
    if (catalog.loading) return 'Loading skills…';
    return searchQuery ? 'No skills found' : 'No skills available';
  };

  return (
    <DropdownMenu
      open={isOpen}
      onOpenChange={(open) => {
        setIsOpen(open);
        if (!open) {
          setSearchQuery('');
        }
      }}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              // `text-supporting`, not the `text-secondary` main.css prescribes
              // for a dense control — the override is explained once in
              // ChatInput.tsx, search "THE RAILS' TYPE".
              className="flex h-7 items-center rounded-md px-0.5 cursor-pointer [&_svg]:size-4 text-text-default/70 hover:bg-background-medium hover:text-text-default text-supporting"
              aria-label={`Manage skills (${activeCount} enabled)`}
            >
              <Layers className="mr-0.5 h-4 w-4" />
              <span>{activeCount}</span>
            </button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="top">Manage skills</TooltipContent>
      </Tooltip>
      <DropdownMenuContent
        side="top"
        align="center"
        className="w-64"
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        <div className="p-2">
          <Input
            type="text"
            placeholder="Search skills..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-8"
            autoFocus
          />
          {filteredEntries.length > 0 && (
            <button
              type="button"
              onClick={handleBulkToggle}
              className="mt-1.5 cursor-pointer text-supporting text-text-default/70 underline-offset-2 hover:text-text-default hover:underline"
            >
              {visibleEnabledCount === 0
                ? `Enable all (${filteredEntries.length})`
                : `Disable all (${visibleEnabledCount})`}
            </button>
          )}
        </div>
        <div className="max-h-[400px] overflow-y-auto">
          {filteredEntries.length === 0 ? (
            <div className="px-3 py-4 text-center text-secondary text-text-muted">
              {emptyMessage()}
            </div>
          ) : (
            filteredEntries.map((entry) => (
              <SkillRow key={entry.key} entry={entry} onToggle={handleToggle} />
            ))
          )}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
};

function SkillRow({
  entry,
  onToggle,
}: {
  entry: SkillCatalogEntry;
  onToggle: (key: string, displayName: string, currentlyEnabled: boolean) => void;
}) {
  if (entry.kind === 'single') {
    const { skill, enabled } = entry;
    // `label` is the extension's own name when the skill was bundled inside one
    // — the piece of provenance the picker could not show while it scanned
    // only three roots and never saw those skills at all.
    const from = skill.source.kind === 'extension' ? skill.source.label : null;
    return (
      <DropdownMenuCheckboxItem
        checked={enabled}
        showIndicator={false}
        onCheckedChange={() => onToggle(skillCatalogToggleKey(entry), skill.name, enabled)}
        onSelect={(event) => event.preventDefault()}
        className="flex cursor-pointer items-center justify-between px-2 py-2 transition-colors duration-[var(--motion-fast)] hover:bg-background-medium"
        title={skill.description || skill.name}
      >
        <div className="flex items-center gap-1.5 min-w-0 pr-2">
          <div className="font-medium text-text-default truncate">{skill.name}</div>
          {skill.builtin && <BuiltInBadge />}
          {from && <span className="text-supporting text-text-subtle truncate">{from}</span>}
        </div>
        <div className="pointer-events-none" aria-hidden="true">
          <Switch checked={enabled} variant="mono" tabIndex={-1} aria-hidden="true" />
        </div>
      </DropdownMenuCheckboxItem>
    );
  }

  const { bundle, enabled } = entry;
  const subNames = bundle.skills.join(', ');
  const entryPoint = bundle.package?.entryPoint ?? null;
  return (
    <DropdownMenuCheckboxItem
      checked={enabled}
      showIndicator={false}
      onCheckedChange={() => onToggle(skillCatalogToggleKey(entry), bundle.displayName, enabled)}
      onSelect={(event) => event.preventDefault()}
      className="flex cursor-pointer items-start justify-between px-2 py-2 transition-colors duration-[var(--motion-fast)] hover:bg-background-medium"
      title={`Bundle: ${subNames}`}
    >
      <div className="flex-1 min-w-0 pr-2">
        <div className="font-medium text-text-default">
          {bundle.displayName}
          <span className="ml-1 text-supporting text-text-subtle">
            {bundle.skills.length} skill{bundle.skills.length === 1 ? '' : 's'}
          </span>
        </div>
        <div className="text-supporting text-text-subtle truncate">
          {entryPoint ? `entry point: ${entryPoint} · ${subNames}` : subNames}
        </div>
      </div>
      <div className="pointer-events-none mt-0.5 flex-shrink-0" aria-hidden="true">
        <Switch checked={enabled} variant="mono" tabIndex={-1} aria-hidden="true" />
      </div>
    </DropdownMenuCheckboxItem>
  );
}
