const scaled = (value) => `calc(${value} * var(--app-font-scale, 1.07))`;
const absoluteLength = /^\d*\.?\d+(px|rem)$/;
const spacingCalculation = /^calc\(var\(--spacing\)\s*\*\s*[\d.]+\)$/;

/** Scale typography only: layout dimensions and Electron's zoom stay independent. */
export function fontScale() {
  return {
    postcssPlugin: 'biorouter-font-scale',
    Declaration(declaration) {
      const { prop, value } = declaration;
      if (value.includes('--app-font-scale')) return;
      const typography =
        prop === 'font-size' ||
        prop === 'line-height' ||
        prop === '--tw-leading' ||
        /^--text-[\w-]+$/.test(prop);
      // Relative em/unitless values already inherit the parent's scaled font.
      // Token references are scaled at their definition, never again at use sites.
      if (
        typography &&
        (absoluteLength.test(value.trim()) || spacingCalculation.test(value.trim()))
      ) {
        declaration.value = scaled(value);
      } else if (prop === 'font') {
        // Preserve the family and optional style/weight; scale the shorthand's
        // absolute size and leading without touching relative KaTeX fonts.
        declaration.value = value.replace(
          /(^|\s)(\d*\.?\d+(?:px|rem))(?:\s*\/\s*(\d*\.?\d+(?:px|rem|em|%)?))?(?=\s)/,
          (_match, prefix, size, leading) =>
            `${prefix}${scaled(size)}${leading ? `/${absoluteLength.test(leading) ? scaled(leading) : leading}` : ''}`
        );
      }
    },
  };
}
