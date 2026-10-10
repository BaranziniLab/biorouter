import { FONT_SIZES, useFontSize, type FontSize } from '../../../hooks/useFontSize';
import { SegmentedControl, type SegmentedOption } from '../../ui/segmented-control';
import { appearanceCopy } from './copy';

export interface FontSizeSelectorProps {
  id?: string;
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
}

const OPTIONS: SegmentedOption<FontSize>[] = FONT_SIZES.map((size) => ({
  value: size,
  label: appearanceCopy.textSizeOptions[size],
  testId: `text-size-${size}-button`,
}));

/**
 * Settings > App > Appearance > Text size. The one `SegmentedControl` (spec 2.6), replacing the
 * native OS radios this used to be: the only native radio left in Settings, and a white disc in
 * dark mode.
 */
export default function FontSizeSelector({
  id,
  'aria-labelledby': ariaLabelledBy,
  'aria-describedby': ariaDescribedBy,
}: FontSizeSelectorProps) {
  const { fontSize, setFontSize } = useFontSize();
  return (
    <SegmentedControl<FontSize>
      id={id}
      options={OPTIONS}
      value={fontSize}
      onValueChange={setFontSize}
      aria-labelledby={ariaLabelledBy}
      aria-describedby={ariaDescribedBy}
      aria-label={ariaLabelledBy ? undefined : appearanceCopy.textSize}
    />
  );
}
