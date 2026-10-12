/**
 * The relative time a list row shows at rest (spec 3.10: "a meta slot holding the relative
 * time, which gives way to the actions on hover or focus"). Codex-compact: "now", "5m", "3h",
 * "4d", then a short date ("Oct 7", with the year only when it is not this year). The full date
 * goes in the time's tooltip (`rowTimeTitle`). Sans face, tabular figures: never mono.
 *
 * Shared by the Workflows and Built apps rows. Pure, so it is tested without a DOM.
 */
export function rowTime(when: Date | null | undefined, now: Date = new Date()): string | null {
  if (!when || Number.isNaN(when.getTime())) return null;
  const seconds = Math.max(0, Math.round((now.getTime() - when.getTime()) / 1000));
  if (seconds < 60) return 'now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  return when.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    ...(when.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
  });
}

/** The full date for a row time's tooltip ("Oct 7, 2026, 2:00 PM"). */
export function rowTimeTitle(when: Date | null | undefined): string | null {
  if (!when || Number.isNaN(when.getTime())) return null;
  return when.toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

/** A Unix-seconds timestamp as a Date, or null. */
export const fromUnixSeconds = (secs?: number | null): Date | null =>
  secs ? new Date(secs * 1000) : null;
