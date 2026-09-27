import { sanitizeDisplayText } from './displayText';

/**
 * How an institution ID is shown (ui-redesign-spec, "Privacy and institution").
 *
 * A Crew workspace or connection stores an institution as a short canonical ID
 * (`ucsf`), the one `biorouter_crew::is_canonical_institution_id` accepts. It is
 * shown **as the ID** — `Privacy: Private · ucsf` — with no casing guesswork:
 * upper-casing `ucsf` happens to be right, upper-casing `sdsc-west` is not, and a
 * wrong guess on a privacy surface reads as a different institution. The only
 * prettier form is a name the model registry itself publishes for the same ID
 * (a configured provider's affiliation), the precedent `institutionLabel` in
 * `privacy/providerAffiliation.ts` set for the provider catalog.
 *
 * Confirmations that set an institution permanently show the raw ID in mono
 * (`InstitutionName` with `raw`), because that is the exact value being written.
 */

/**
 * The HTML `pattern` for an institution ID field: the same rule as
 * `is_canonical_institution_id` (1–64 of `a-z 0-9 _ -`, starting with a letter
 * or digit). `pattern` is anchored by the browser.
 *
 * ⚠ The `-` in the second class is escaped, and must stay escaped. Browsers
 * (Chromium since 112, and jsdom) compile a `pattern` attribute with the `v`
 * flag, under which a bare `-` inside a class is a SYNTAX ERROR — and a pattern
 * that fails to compile is ignored without a word, so the field would accept
 * `UCSF` and the refusal would only come from the daemon. Without a flag, as
 * {@link isInstitutionId} compiles it, `\-` is the same literal `-`.
 */
export const INSTITUTION_ID_PATTERN = '[a-z0-9][a-z0-9_\\-]{0,63}';

const CANONICAL_INSTITUTION_ID = new RegExp(`^${INSTITUTION_ID_PATTERN}$`);

/** Whether a value is a canonical institution ID, exactly as the daemon and broker judge it. */
export function isInstitutionId(value: unknown): value is string {
  return typeof value === 'string' && CANONICAL_INSTITUTION_ID.test(value);
}

/** An institution a configured provider's affiliation names, with the registry's display name. */
export interface KnownInstitution {
  id: string;
  display_name?: string | null;
}

/**
 * The institution ID as stored, trimmed and stripped of anything that cannot
 * be displayed, or `null` when there is none. A non-canonical legacy value is
 * still shown rather than hidden — an unexpected institution is exactly what a
 * person needs to see — but never as anything other than itself.
 */
export function institutionId(id: string | null | undefined): string | null {
  const value = sanitizeDisplayText(id);
  return value ? value : null;
}

/**
 * How to write an institution in running text: the registry's name for that
 * exact ID when a known institution publishes one, otherwise the ID itself.
 * `null` when no institution is set, so each surface chooses its own words for
 * that case instead of inheriting a placeholder.
 */
export function institutionLabel(
  id: string | null | undefined,
  known?: readonly KnownInstitution[] | null
): string | null {
  const value = institutionId(id);
  if (value === null) return null;
  const published = Array.isArray(known)
    ? known.find((institution) => institution?.id === value)?.display_name
    : null;
  const name = sanitizeDisplayText(published);
  return name || value;
}
