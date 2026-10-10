import { GENERATED_THEMES, THEME_FAMILY_IDS } from '../../styles/themes.generated';
import { SegmentedControl, type SegmentedOption } from '../ui/segmented-control';
import { useTheme, type ThemeFamily } from '../../contexts/ThemeContext';
import { appearanceCopy } from '../settings/app/copy';

export interface ThemeFamilySelectorProps {
  id?: string;
  className?: string;
  /** Settings > App > Color palette names it through its row label (`SettingRow`). */
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
  'aria-label'?: string;
}

/**
 * The theme *family* (Parchment / Alma Mater / Roche Limit), the second axis beside the
 * light/dark {@link ThemeSelector}. Both are the one `SegmentedControl` (spec 2.6).
 *
 * ⚠ **The swatch is ALWAYS `family.swatch`**, the family's accent, in every state. It once
 * switched to `currentColor` while active, which discarded the family's identity at exactly the
 * moment you picked it. Its ring is the authored `.br-swatch-ring`, so the mark sits on a known
 * ground in both modes.
 *
 * The options derive from the generated registry, so a new family appears here without an edit.
 */
const FAMILIES: SegmentedOption<ThemeFamily>[] = THEME_FAMILY_IDS.map((id) => ({
  value: id,
  label: GENERATED_THEMES[id].label,
  swatch: GENERATED_THEMES[id].swatch,
  testId: `theme-family-${id}-button`,
}));

export default function ThemeFamilySelector({
  id,
  className,
  'aria-labelledby': ariaLabelledBy,
  'aria-describedby': ariaDescribedBy,
  'aria-label': ariaLabel,
}: ThemeFamilySelectorProps) {
  const { themeFamily, setThemeFamily } = useTheme();
  return (
    <SegmentedControl<ThemeFamily>
      id={id}
      className={className}
      options={FAMILIES}
      value={themeFamily}
      onValueChange={setThemeFamily}
      aria-labelledby={ariaLabelledBy}
      aria-describedby={ariaDescribedBy}
      aria-label={ariaLabelledBy ? undefined : (ariaLabel ?? appearanceCopy.palette)}
    />
  );
}
