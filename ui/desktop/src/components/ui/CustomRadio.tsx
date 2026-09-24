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
        {/* §3.3: a 22px visual ring inside a 24px hit target, with a 10px inner
            dot and an 8px gap to the label. The outer box is the TARGET — it is
            deliberately 1px larger than what you can see on each side, because a
            selection control should be forgiving of a near miss without looking
            heavier for it. Input, ring and dot share this box so both siblings
            react to peer-checked. */}
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
          {/* `--border-emphasized` is the interactive-border token (§3.2) — ink at
              24%, derived per family — replacing a neutral that each family had to
              author and that drifted against the surface it sat on.

              `data-radio-ring` / `data-radio-dot` are selector hooks for main.css
              (Q2-11): the input above is sr-only, so its focus was drawn on an
              invisible pixel, and the dot is a fill that forced colours erase.
              main.css rings the RING when the input is :focus-visible and redraws
              ring and dot in system colours under forced colours. The rules key on
              these attributes and on the input being a preceding sibling, so keep
              the input first and all three in this one box. */}
          <span
            data-radio-ring=""
            aria-hidden="true"
            className="pointer-events-none absolute inset-[1px] rounded-full border-[1.5px] border-border-emphasized
                      transition-colors
                      peer-checked:border-border-accent"
          />
          <span
            data-radio-dot=""
            aria-hidden="true"
            className="pointer-events-none h-2.5 w-2.5 rounded-full bg-background-accent opacity-0
                      transition-opacity
                      peer-checked:opacity-100"
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
