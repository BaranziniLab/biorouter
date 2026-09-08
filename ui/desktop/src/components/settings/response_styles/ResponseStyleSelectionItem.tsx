import { useEffect, useState } from 'react';

export interface ResponseStyle {
  key: string;
  label: string;
  description: string;
}

export const all_response_styles: ResponseStyle[] = [
  {
    key: 'detailed',
    label: 'Detailed',
    description: 'Tool calls are by default shown open to expose details',
  },
  {
    key: 'concise',
    label: 'Concise',
    description: 'Tool calls are by default closed and only show the tool used',
  },
];

interface ResponseStyleSelectionItemProps {
  currentStyle: string;
  style: ResponseStyle;
  showDescription: boolean;
  handleStyleChange: (newStyle: string) => void;
}

export function ResponseStyleSelectionItem({
  currentStyle,
  style,
  showDescription,
  handleStyleChange,
}: ResponseStyleSelectionItemProps) {
  const [checked, setChecked] = useState(currentStyle === style.key);

  useEffect(() => {
    setChecked(currentStyle === style.key);
  }, [currentStyle, style.key]);

  // The single-child wrapper this used to return is gone, and that is what
  // restores the list's hairlines: with one row per wrapper EVERY row was
  // `:last-child` of its own box, so `.biorouter-settings-row:last-child`
  // suppressed all of them and the two styles ran together.
  //
  // ⚠ **No state-dependent fill.** The radio states the selection; the row's
  // `bg-background-medium/70` said it again in a weaker language, and the
  // unlayered hover rule beat it, so pointing at the selected style visibly
  // un-selected it.
  return (
    <div
      className="biorouter-settings-row group flex min-w-0 cursor-pointer items-center justify-between gap-3 px-3 py-2.5 text-text-default"
      onClick={() => handleStyleChange(style.key)}
    >
      <div className="min-w-0 flex-1">
        <p className="text-label text-text-default">{style.label}</p>
        {showDescription && (
          <p className="mt-0.5 max-w-md text-supporting text-text-muted">{style.description}</p>
        )}
      </div>

      {/* `CustomRadio`'s construction (§3.3) — see the note in
          `ModeSelectionItem`, including why the input stays inside this span. */}
      <span className="relative inline-flex h-6 w-6 shrink-0 items-center justify-center">
        <input
          type="radio"
          name="responseStyles"
          value={style.key}
          checked={checked}
          onChange={() => handleStyleChange(style.key)}
          className="peer sr-only"
        />
        <span
          className="pointer-events-none absolute inset-[1px] rounded-full border-[1.5px] border-border-emphasized
                     transition-colors
                     peer-checked:border-border-accent"
        />
        <span
          className="pointer-events-none h-2.5 w-2.5 rounded-full bg-background-accent opacity-0
                     transition-opacity
                     peer-checked:opacity-100"
        />
      </span>
    </div>
  );
}
