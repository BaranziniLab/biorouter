/**
 * The model's name as a person says it (spec 3.7): the release-date stamp a
 * provider appends comes off, so `gpt-5.6-sol-2026-07-09` reads `gpt-5.6-sol`
 * and `claude-3-5-sonnet-20241022` reads `claude-3-5-sonnet`. The full id stays
 * in the chip's tooltip, its accessible name and the menu.
 *
 * Only a trailing date is removed, and only when something is left in front of
 * it: a name that IS a date, or has none, comes back unchanged.
 */
const TRAILING_DATE = /-(?:\d{4}-\d{2}-\d{2}|\d{8})$/;

export function friendlyModelName(id: string): string {
  const trimmed = id.trim();
  const stripped = trimmed.replace(TRAILING_DATE, '');
  return stripped.length > 0 ? stripped : trimmed;
}
