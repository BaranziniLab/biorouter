// ui/desktop/src/components/knowledge/kbFormat.ts
import type { KbFormat, Manifest } from '../../api/types.gen';

/**
 * The schema generation at which `Manifest.format` starts meaning anything.
 *
 * Mirrors `CURRENT_SCHEMA_VERSION` in
 * `crates/biorouter-mcp/src/knowledge/types.rs`. It is duplicated here because
 * the constant is not on the wire — only its consequence is, as the
 * `schema_version` a base reports — and the renderer needs the same rule the
 * daemon's `Manifest::profile()` applies.
 */
export const CURRENT_KB_SCHEMA_VERSION = 3;

/** What the UI says a base's format is, including the state where there isn't one. */
export type KbFormatLabel = 'OKF' | 'BioOKF' | 'Legacy';

/**
 * The base's profile, or `null` for a base written before the OKF generation.
 *
 * ⚠ **Reading `manifest.format` on its own is the DR-6 trap, and it is the one
 * an implementer falls into first.** The field is `#[serde(default)]` on the
 * Rust side, so every `manifest.yaml` that predates it deserializes as `okf` —
 * and re-saving a legacy manifest genuinely writes `format: okf` into the file.
 * A check written against the field alone therefore reports every legacy base as
 * already-migrated. The daemon's own accessor folds `schema_version` in
 * (`Manifest::profile()`), and so does this one.
 */
export function kbProfile(base: Pick<Manifest, 'format' | 'schema_version'>): KbFormat | null {
  return base.schema_version >= CURRENT_KB_SCHEMA_VERSION ? (base.format ?? 'okf') : null;
}

/** The badge word for a base: `OKF`, `BioOKF`, or `Legacy` when it declares no profile. */
export function kbFormatLabel(base: Pick<Manifest, 'format' | 'schema_version'>): KbFormatLabel {
  const profile = kbProfile(base);
  if (profile === null) return 'Legacy';
  return profile === 'biookf' ? 'BioOKF' : 'OKF';
}

/**
 * The help a `Legacy` badge carries (an `InfoTip asChild` in the band), and nothing else does.
 *
 * A legacy base is not broken and is not a warning — DR-26 makes "no path to
 * OKF in this release" a deliberate decision, so the badge has to say the base
 * is fine as it is rather than implying an upgrade the user cannot perform.
 */
export const LEGACY_FORMAT_TITLE =
  'Created before the format chooser. It reads fine and is not validated.';

/** The id the daemon will *probably* derive from a name — a preview, never a fact. */
export function previewKbId(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .substring(0, 64);
}
