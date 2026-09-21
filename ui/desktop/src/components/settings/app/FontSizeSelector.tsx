import { FONT_SIZES, useFontSize } from '../../../hooks/useFontSize';

export default function FontSizeSelector() {
  const { fontSize, setFontSize } = useFontSize();
  return (
    <fieldset className="biorouter-settings-section">
      <legend className="biorouter-settings-section-header text-caps text-text-muted">
        Font size
      </legend>
      <p className="mb-3 text-supporting text-text-muted">Adjust text throughout Biorouter</p>
      <div className="biorouter-settings-control-strip">
        {FONT_SIZES.map((size) => (
          <label
            key={size}
            className="flex cursor-pointer items-center gap-2 rounded-element border border-border-default px-3 py-2 text-label text-text-default has-[:checked]:bg-background-muted"
          >
            <input
              type="radio"
              name="app-font-size"
              value={size}
              checked={fontSize === size}
              onChange={() => setFontSize(size)}
              className="accent-text-default"
            />
            {size[0].toUpperCase() + size.slice(1)}
          </label>
        ))}
      </div>
    </fieldset>
  );
}
