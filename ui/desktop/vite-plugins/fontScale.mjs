import { list } from 'postcss';

const scaled = (value) => `calc(${value} * var(--app-font-scale, 1))`;
const absoluteLength = /^\d*\.?\d+(px|rem)$/;
const spacingCalculation = /^calc\(var\(--spacing\)\s*\*\s*[\d.]+\)$/;

const spaceReverseCalculation =
  /^calc\(calc\(var\(--spacing\)\s*\*\s*[\d.]+\)\s*\*\s*(?:var\(--tw-space-[xy]-reverse\)|calc\(1\s*-\s*var\(--tw-space-[xy]-reverse\)\))\)$/;
const controlTokens = new Set([
  '--control-sm',
  '--control-md',
  '--control-lg',
  '--control-compact',
  '--row-height',
  '--row-height-rail',
  '--md-code-pad',
]);
const spacingProperty =
  /^(?:padding|margin)(?:-(?:top|right|bottom|left|block|inline)(?:-(?:start|end))?)?$|^(?:gap|row-gap|column-gap)$/;

function intrinsicGeometry(declaration) {
  for (let parent = declaration.parent; parent; parent = parent.parent) {
    if (parent.type === 'rule' && /\.(?:katex|xterm|monaco)(?:[-\s.#:[>]|$)/.test(parent.selector))
      return true;
  }
  return false;
}

/** Text and its breathing room scale together; viewport and panel geometry stay fixed. */
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
      } else if (
        (spacingProperty.test(prop) || controlTokens.has(prop)) &&
        !intrinsicGeometry(declaration)
      ) {
        declaration.value = list
          .space(value)
          .map((part) =>
            absoluteLength.test(part) ||
            spacingCalculation.test(part) ||
            spaceReverseCalculation.test(part)
              ? scaled(part)
              : part
          )
          .join(' ');
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
