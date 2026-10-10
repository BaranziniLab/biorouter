import { useId } from 'react';
import { Switch } from '../ui/switch';
import BuiltInBadge from '../ui/BuiltInBadge';
import { RowActions, RowContextMenu, type RowActionItem } from '../ui/row-actions';
import type { CatalogSkill } from '../../api';
import { SKILLS_COPY } from './copy';

interface SkillItemProps {
  skill: CatalogSkill;
  enabled: boolean;
  /** Open the skill's folder in the system file browser. */
  onOpen: () => void;
  /**
   * Omitted where the skill is not the user's to delete: one Biorouter ships
   * and re-seeds on every start, or one an installed extension supplies. A
   * delete that succeeds and silently reverts is worse than no item — the
   * lesson `BUILTIN_SKILL_NAMES` was written for, applied to a second case.
   */
  onDelete?: () => void;
  onShare: () => void;
  onToggle: (enabled: boolean) => void;
}

/**
 * One skill: Crew's row (spec 3.11). The name and one muted line, then a `⋯`
 * menu revealed on hover or focus, then the switch. The same menu opens on
 * right-click, Shift+F10 and the ContextMenu key.
 *
 * The text block is not a button: it used to open the folder, which the
 * folder button beside it also did, so one action had two targets. And no
 * third line names the source folder: the group heading already says where the
 * skill came from, and "Open folder" goes there.
 */
export default function SkillItem({
  skill,
  enabled,
  onOpen,
  onDelete,
  onShare,
  onToggle,
}: SkillItemProps) {
  const titleId = useId();
  // ⚠ From the daemon, not from the hand-synced `BUILTIN_SKILL_NAMES` copy.
  // Rust owns the seeder, so Rust owns the answer.
  const builtin = skill.builtin;
  const menu: RowActionItem[] = [
    { label: SKILLS_COPY.openFolder, onSelect: onOpen },
    { label: SKILLS_COPY.copySkillMd, onSelect: onShare },
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
      <div
        className="biorouter-list-row flex items-center gap-3 px-3 py-2.5"
        data-skill-row="single"
      >
        <div className="min-w-0 flex-1">
          {/* ⚠ `min-w-0` on the name, `flex-shrink-0` on the badge (the `Badge`
              primitive carries its own). A flex item's `min-width: auto` is its
              min-content width, so a long skill name would otherwise push the
              "Built-in" badge out of the row rather than ellipsing. */}
          <div className="flex min-w-0 items-center gap-1.5">
            <p id={titleId} className="min-w-0 truncate text-label text-text-default">
              {skill.name}
            </p>
            {builtin && <BuiltInBadge />}
          </div>
          <p className="truncate text-supporting text-text-muted">{skill.description}</p>
        </div>
        <RowActions menu={menu} />
        {/* Named by the row's title, the same in both states: the state is the
            switch's `aria-checked`, not part of its name (spec 2.6). */}
        <Switch checked={enabled} onCheckedChange={onToggle} aria-labelledby={titleId} />
      </div>
    </RowContextMenu>
  );
}
