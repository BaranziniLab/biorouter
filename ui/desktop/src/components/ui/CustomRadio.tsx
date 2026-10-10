/**
 * CustomRadio - A reusable radio button component with dark mode support
 * @param {Object} props - Component props
 * @param {string} props.id - Unique identifier for the radio input
 * @param {string} props.name - Name attribute for the radio input
 * @param {string} props.value - Value of the radio input
 * @param {boolean} props.checked - Whether the radio is checked
 * @param {function} props.onChange - Function to call when radio selection changes
 * @param {boolean} [props.disabled] - Whether the radio is disabled
 * @param {React.ReactNode} [props.label] - Primary label content
 * @param {React.ReactNode} [props.secondaryLabel] - Secondary/subtitle label content
 * @param {React.ReactNode} [props.rightContent] - Optional content to display on the right side
 * @param {string} [props.className] - Additional CSS classes for the main container
 * @returns {JSX.Element}
 */
const CustomRadio = ({
  id,
  name,
  value,
  checked,
  onChange,
  disabled = false,
  label = null,
  secondaryLabel = null,
  rightContent = null,
  className = '',
}: {
  id: string;
  name: string;
  value: string;
  checked: boolean;
  onChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  disabled?: boolean;
  label?: React.ReactNode;
  secondaryLabel?: React.ReactNode;
  rightContent?: React.ReactNode;
  className?: string;
}) => {
  return (
    <label
      htmlFor={id}
      className={`flex min-h-8 justify-between items-center py-2 cursor-pointer ${disabled ? 'opacity-50 cursor-not-allowed' : ''} ${className}`}
    >
      <div className="flex items-center">
        {/* Spec 2.6: a 16px ring inside a 24px hit target, a 6px dot that scales from 0.6 to
            1 over `--dur-fast`, and an 8px gap to the label; the same box as the Checkbox, so
            the two share an optical axis. Size and motion are authored in main.css
            (`.br-radio-ring`, `.br-radio-dot`). Input, ring and dot share this box so both
            siblings react to the input's state. */}
        <span className="relative mr-2 inline-flex h-6 w-6 shrink-0 items-center justify-center">
          <input
            type="radio"
            id={id}
            name={name}
            value={value}
            checked={checked}
            onChange={onChange}
            disabled={disabled}
            className="peer sr-only"
          />
          {/* The ring's COLOUR is authored in main.css, not written here (QA Q3-58).
              It used to be `border-border-emphasized` — ink at 24% — which measured
              1.6:1 on white: under the 3:1 a control's boundary owes, so an
              unchecked radio was a faint smudge. main.css now paints it
              `--text-muted` at rest (≥ 4.5:1 on every ground, `check-contrast.mjs`)
              and `--border-accent` when checked, unlayered so no utility can pull
              it back to the faint token, and `focusFallback.test.ts` measures it
              in all six scopes.

              `data-radio-ring` / `data-radio-dot` are selector hooks for main.css
              (Q2-11): the input above is sr-only, so its focus was drawn on an
              invisible pixel, and the dot is a fill that forced colours erase.
              main.css colours the ring, rings the RING when the input is
              :focus-visible, and redraws ring and dot in system colours under
              forced colours. The rules key on these attributes and on the input
              being a preceding sibling, so keep the input first and all three in
              this one box. */}
          <span
            data-radio-ring=""
            aria-hidden="true"
            className="br-radio-ring pointer-events-none absolute rounded-full border-[1.5px]"
          />
          <span
            data-radio-dot=""
            aria-hidden="true"
            className="br-radio-dot pointer-events-none rounded-full"
          />
        </span>

        {(label || secondaryLabel) && (
          <div>
            {label && <p className="text-label text-text-default">{label}</p>}
            {secondaryLabel && <p className="text-supporting text-text-muted">{secondaryLabel}</p>}
          </div>
        )}
      </div>

      {rightContent && (
        <div className="flex items-center text-body text-text-muted">{rightContent}</div>
      )}
    </label>
  );
};

export default CustomRadio;
