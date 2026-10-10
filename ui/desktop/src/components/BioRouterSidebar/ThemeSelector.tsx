import { Monitor, Moon, Sun } from '../icons/app-icons';
import { SegmentedControl, type SegmentedOption } from '../ui/segmented-control';
import { useTheme } from '../../contexts/ThemeContext';
import { appearanceCopy } from '../settings/app/copy';

type ThemePreference = 'light' | 'dark' | 'system';

export interface ThemeSelectorProps {
  id?: string;
  className?: string;
  /** Settings > App > Theme names it through its row label (`SettingRow` passes these). */
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
  'aria-label'?: string;
}

/**
 * Light / dark / system: one of the two theme axes, and the sibling of
 * {@link ThemeFamilySelector}. Both are the one `SegmentedControl` (spec 2.6), so
 * they read as siblings in Settings > App > Appearance.
 *
 * ⚠ **Selection is achromatic** (astryx A-06): the segmented thumb is a neutral
 * raised ground, never the accent. Coral is reserved for the one committing
 * action of a view.
 *
 * The test ids are read by `tests/e2e/app.spec.ts`; the state is `aria-checked`
 * on each `role="radio"` segment (it used to be `aria-pressed` on toggle buttons).
 */
export default function ThemeSelector({
  id,
  className,
  'aria-labelledby': ariaLabelledBy,
  'aria-describedby': ariaDescribedBy,
  'aria-label': ariaLabel,
}: ThemeSelectorProps) {
  const { userThemePreference, setUserThemePreference } = useTheme();
  const options: SegmentedOption<ThemePreference>[] = [
    {
      value: 'light',
      label: appearanceCopy.themeOptions.light,
      icon: Sun,
      testId: 'light-mode-button',
    },
    {
      value: 'dark',
      label: appearanceCopy.themeOptions.dark,
      icon: Moon,
      testId: 'dark-mode-button',
    },
    {
      value: 'system',
      label: appearanceCopy.themeOptions.system,
      icon: Monitor,
      testId: 'system-mode-button',
    },
  ];

  return (
    <SegmentedControl<ThemePreference>
      id={id}
      className={className}
      options={options}
      value={userThemePreference}
      onValueChange={setUserThemePreference}
      aria-labelledby={ariaLabelledBy}
      aria-describedby={ariaDescribedBy}
      aria-label={ariaLabelledBy ? undefined : (ariaLabel ?? appearanceCopy.theme)}
    />
  );
}
