/**
 * A file name cut in its middle rather than at its end (FILES2-N1), so the extension, which says
 * what the file is, is always what stays in sight. A name that fits is left whole.
 *
 * Pure: `measure` gives a text's drawn width, so the arithmetic is tested without a layout engine.
 */

/** The ellipsis that stands for what was cut. */
export const MIDDLE_ELLIPSIS = '…';

/** The longest ending kept whole as an extension, dot included (`.tar.gz` is two). */
const MAX_EXTENSION_CHARACTERS = 12;

/**
 * The name's ending that must stay in sight: from its last dot, when that is not its first
 * character and the ending is short enough to be an extension, else nothing. Counted in
 * characters, never code units, so a character outside the Basic Multilingual Plane is never
 * split.
 */
export function extensionOf(name: string): string {
  const characters = Array.from(name);
  const dot = characters.lastIndexOf('.');
  if (dot <= 0 || characters.length - dot > MAX_EXTENSION_CHARACTERS) return '';
  return characters.slice(dot).join('');
}

/**
 * `name` as wide as `width` at most: whole when it fits; otherwise as much of its start as fits,
 * an ellipsis, and its end, the extension whole and a few characters before it. When not even the
 * ellipsis and the extension fit, they are returned all the same: the stylesheet cuts what is left.
 */
export function middleTruncate(
  name: string,
  width: number,
  measure: (text: string) => number
): string {
  if (!(width > 0) || measure(name) <= width) return name;
  const characters = Array.from(name);
  const extension = Array.from(extensionOf(name));
  // A few characters before the extension stay with it, so `report-v2.pdf` does not read `….pdf`.
  const tailLength = Math.min(characters.length, extension.length + 4);
  const tail = characters.slice(characters.length - tailLength).join('');
  const head = characters.slice(0, characters.length - tailLength);
  const fits = (count: number) =>
    measure(`${head.slice(0, count).join('')}${MIDDLE_ELLIPSIS}${tail}`) <= width;
  let low = 0;
  let high = head.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (fits(middle)) low = middle;
    else high = middle - 1;
  }
  return `${head.slice(0, low).join('')}${MIDDLE_ELLIPSIS}${tail}`;
}
