import React from 'react';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from '../../styles/themes.generated';
import { Button } from '../ui/button';
import { cn } from '../../utils';
import { useTheme, type ThemeFamily } from '../../contexts/ThemeContext';

interface ThemeFamilySelectorProps {
  className?: string;
  horizontal?: boolean;
}

/**
 * Selects the theme *family* (Parchment / Alma Mater / Roche Limit) — the second
 * axis beside the light/dark {@link ThemeSelector}. Both render the same
 * segmented-button look, so they read as siblings in the Appearance settings.
 * The small swatch is the family's accent (terracotta for Parchment, UCSF teal
 * for Alma Mater, Jupyter-adjacent orange for Roche Limit).
 *
 * ⚠ **The swatch is ALWAYS `family.swatch`.** It used to switch to
 * `currentColor` while active, which discarded the family's identity at exactly
 * the moment you picked it — the one button whose colour you were choosing was
 * the one that stopped showing it. Its ring is the authored `.br-swatch-ring`
 * (a 1px `--background-default` ring, so the mark's adjacent colour is a known
 * ground) in place of a hardcoded `inset 0 0 0 1px rgba(0,0,0,.15)` that had no
 * dark value at all.
 *
 * See {@link ThemeSelector} for why the active arm is `tint-selected
 * tint-interactive` rather than the accent fill, and why `aria-pressed` is here.
 */
/**
 * Tailwind generates utilities by scanning source for literal class names, so
 * an interpolated `grid-cols-${n}` would silently produce an unstyled grid.
 * These literals are what make a new family's column appear.
 */
const GRID_COLS: Record<number, string> = {
  1: 'grid-cols-1',
  2: 'grid-cols-2',
  3: 'grid-cols-3',
  4: 'grid-cols-4',
  5: 'grid-cols-5',
  6: 'grid-cols-6',
};

const FAMILIES: { id: ThemeFamily; label: string; swatch: string }[] = THEME_FAMILY_IDS.map(
  (id) => ({
    id,
    label: GENERATED_THEMES[id].label,
    swatch: GENERATED_THEMES[id].swatch,
  })
);

const ThemeFamilySelector: React.FC<ThemeFamilySelectorProps> = ({
  className = '',
  horizontal = false,
}) => {
  const { themeFamily, setThemeFamily } = useTheme();

  return (
    <div className={`${!horizontal ? 'px-1 py-2 space-y-2' : ''} ${className}`}>
      <div
        className={`${horizontal ? 'flex' : `grid ${GRID_COLS[FAMILIES.length] ?? 'grid-cols-3'}`} gap-1 ${!horizontal ? 'px-3' : ''}`}
      >
        {FAMILIES.map((family) => {
          const active = themeFamily === family.id;
          return (
            <Button
              key={family.id}
              data-testid={`theme-family-${family.id}-button`}
              onClick={() => setThemeFamily(family.id)}
              aria-pressed={active}
              variant="ghost"
              size="sm"
              className={cn(
                'border border-border-default transition-colors',
                active
                  ? 'tint-selected tint-interactive font-medium text-text-default'
                  : 'text-text-muted hover:text-text-default'
              )}
            >
              <span
                aria-hidden
                className="br-swatch-ring h-2.5 w-2.5 flex-none rounded-full"
                style={{ backgroundColor: family.swatch }}
              />
              <span>{family.label}</span>
            </Button>
          );
        })}
      </div>
    </div>
  );
};

export default ThemeFamilySelector;
