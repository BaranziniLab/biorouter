/**
 * A TypeScript port of the naming design's normalization functions
 * (naming-design.md, "Normalization and keys"), used ONLY as the renderer's
 * fallback for detecting colliding display names when the daemon predates the
 * projected `labels`.
 *
 * ```text
 * clean(s)        = NFC(s) → trim → collapse White_Space runs to ' '
 * strip_ignorable = remove Default_Ignorable_Code_Point characters
 * name_key(s)     = strip_ignorable(NFKC(s)) → to_lowercase() → NFKC
 *                   → map ' ', '-', '_', '.' to '-' → collapse '-' runs → trim '-'
 * ```
 *
 * ⚠ **The daemon's answer wins whenever it exists.** Its keys add the UTS #39
 * confusable skeleton (S2b), which this port does not attempt: `anaIysis` and
 * `analysis` have different `nameKey`s here and the same skeleton there. A key
 * computed here is display-only and must never address, authorize or dedupe
 * anything — names are resolved by the daemon (`POST /crew/resolve`), from the
 * caller's own snapshot.
 *
 * `toLowerCase` is the Unicode default mapping, as Rust's `to_lowercase` is, not
 * full case folding: `ß` and `ss` stay distinct in both, deliberately.
 */

const WHITE_SPACE_RUN = /\p{White_Space}+/gu;
const EDGE_WHITE_SPACE = /^\p{White_Space}+|\p{White_Space}+$/gu;
const DEFAULT_IGNORABLE = /\p{Default_Ignorable_Code_Point}/gu;
const KEY_SEPARATOR = /[ \-_.]/g;
const DASH_RUN = /-+/g;
const EDGE_DASH = /^-+|-+$/g;

/**
 * `clean(s)`: NFC, trim, collapse `White_Space` runs to one U+0020.
 *
 * The trim is `White_Space`, not ECMAScript's `trim()`, which also strips U+FEFF
 * and so would disagree with Rust's `str::trim` on the edge.
 */
export function cleanName(value: string): string {
  return value.normalize('NFC').replace(EDGE_WHITE_SPACE, '').replace(WHITE_SPACE_RUN, ' ');
}

/** `strip_ignorable(s)`: remove every `Default_Ignorable_Code_Point` character. */
export function stripIgnorable(value: string): string {
  return value.replace(DEFAULT_IGNORABLE, '');
}

/**
 * `name_key(s)`. `"Analysis Lab"`, `"analysis-lab"`, `"ANALYSIS_LAB"`,
 * `"Analysis.Lab"`, fullwidth `"Ａｎａｌｙｓｉｓ Ｌａｂ"` and `"Analysis Lab"` followed
 * by U+FE0F all have the key `analysis-lab`.
 */
export function nameKey(value: string): string {
  return stripIgnorable(value.normalize('NFKC'))
    .toLowerCase()
    .normalize('NFKC')
    .replace(KEY_SEPARATOR, '-')
    .replace(DASH_RUN, '-')
    .replace(EDGE_DASH, '');
}

/**
 * The key two display names collide on: `name_key(clean(s))`. The broker cleans
 * a display name before it stores one, so a legacy nickname is cleaned here the
 * same way first — a tab or a newline then separates words exactly as a space
 * does, instead of making two lookalike names look distinct.
 */
export function displayNameKey(value: string): string {
  return nameKey(cleanName(value));
}

const HYPHENATED_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SIMPLE_UUID = /^[0-9a-f]{32}$/i;
const BRACED_UUID = /^\{[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\}$/i;
const URN_UUID = /^urn:uuid:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const HEX_64 = /^[0-9a-f]{64}$/i;

/**
 * Whether text reads as a machine identifier: anything that parses as a UUID (the
 * hyphenated, simple, braced and URN forms) or 64 hex characters — the shapes
 * the broker refuses as a name (naming design D4). The display layer uses it to
 * keep a legacy name that looks like an ID out of the default path.
 */
export function isMachineIdShaped(value: string): boolean {
  const text = value.trim();
  return (
    HYPHENATED_UUID.test(text) ||
    SIMPLE_UUID.test(text) ||
    BRACED_UUID.test(text) ||
    URN_UUID.test(text) ||
    HEX_64.test(text)
  );
}
