import { Button } from '../ui/button';
import { Switch } from '../ui/switch';
import BuiltInBadge from '../ui/BuiltInBadge';
import { Copy, Trash2, FolderDot } from '../icons/app-icons';
import type { CatalogSkill } from '../../api';

interface SkillItemProps {
  skill: CatalogSkill;
  enabled: boolean;
  onClick: () => void;
  /**
   * Omitted where the skill is not the user's to delete: one Biorouter ships
   * and re-seeds on every start, or one an installed extension supplies. A
   * delete that succeeds and silently reverts is worse than no button — the
   * lesson `BUILTIN_SKILL_NAMES` was written for, applied to a second case.
   */
  onDelete?: () => void;
  onShare: () => void;
  onToggle: (enabled: boolean) => void;
}

export default function SkillItem({
  skill,
  enabled,
  onClick,
  onDelete,
  onShare,
  onToggle,
}: SkillItemProps) {
  // ⚠ From the daemon, not from the hand-synced `BUILTIN_SKILL_NAMES` copy.
  // Rust owns the seeder, so Rust owns the answer.
  const builtin = skill.builtin;
  return (
    <div className="biorouter-list-row group flex items-start gap-3 px-3 py-3">
      <button
        type="button"
        className="min-w-0 flex-1 cursor-pointer rounded-inner text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
        onClick={onClick}
        aria-label={`Open skill ${skill.name}`}
      >
        {/* ⚠ `min-w-0` on the name, `flex-shrink-0` on the badge (the `Badge`
            primitive carries its own). A flex item's `min-width: auto` is its
            min-content width, so a long skill name would otherwise push the
            "Built-in" badge out of the row rather than ellipsing — the reading
            column leaves about 704px here, minus the three actions and the
            switch on the trailing edge. */}
        <div className="flex min-w-0 items-center gap-1.5">
          <p className="min-w-0 truncate text-label text-text-default">{skill.name}</p>
          {builtin && <BuiltInBadge />}
        </div>
        <p className="mt-0.5 line-clamp-1 text-supporting text-text-muted">{skill.description}</p>
        {skill.source.kind !== 'biorouter' && (
          <p className="mt-0.5 truncate font-mono text-supporting text-text-subtle">
            {skill.sourceRoot}
          </p>
        )}
      </button>
      <div className="mt-0.5 flex shrink-0 items-center gap-2">
        <div
          className="flex gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
          onClick={(e) => e.stopPropagation()}
        >
          <Button
            variant="ghost"
            shape="round"
            onClick={() => onClick()}
            title="Open in Finder"
            aria-label={`Open ${skill.name} in Finder`}
          >
            <FolderDot className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            shape="round"
            onClick={() => onShare()}
            title="Copy SKILL.md to clipboard"
            aria-label={`Copy ${skill.name} SKILL.md to clipboard`}
          >
            <Copy className="h-4 w-4" />
          </Button>
          {/* V7 — a quiet destructive row action is `ghost` plus danger ink, on
              the same `round` rung as the two glyph actions beside it. It was
              `size="sm"`, a 28px pill in a cluster of 32px squares. */}
          {onDelete && !builtin && (
            <Button
              variant="ghost"
              shape="round"
              className="text-text-danger"
              onClick={() => onDelete()}
              title="Delete this skill"
              aria-label={`Delete ${skill.name}`}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          )}
        </div>
        <div onClick={(e) => e.stopPropagation()}>
          <Switch
            checked={enabled}
            onCheckedChange={onToggle}
            variant="mono"
            aria-label={`${enabled ? 'Disable' : 'Enable'} ${skill.name}`}
          />
        </div>
      </div>
    </div>
  );
}
