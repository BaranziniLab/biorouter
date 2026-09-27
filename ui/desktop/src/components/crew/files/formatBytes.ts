const UNITS = ['KB', 'MB', 'GB', 'TB'] as const;

/**
 * A file size a person reads at a glance, in 1024 units: "103 bytes", "55 KB", "1.5 MB".
 *
 * Below 10 of a unit one decimal is kept (a trailing ".0" is dropped), from 10 up the number
 * is whole. A value that would round up to 1024 of a unit is written in the next unit, so the
 * row never reads "1024 KB". The exact byte count is not lost: the transfer routes carry it,
 * and nothing is decided from this string.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '';
  if (bytes < 1024) {
    const whole = Math.round(bytes);
    return `${whole} ${whole === 1 ? 'byte' : 'bytes'}`;
  }
  let value = bytes / 1024;
  let unit = 0;
  const shown = (amount: number) =>
    amount < 10 ? Math.round(amount * 10) / 10 : Math.round(amount);
  while (unit < UNITS.length - 1 && shown(value) >= 1024) {
    value /= 1024;
    unit += 1;
  }
  const amount = shown(value);
  return `${Number.isInteger(amount) ? amount : amount.toFixed(1)} ${UNITS[unit]}`;
}
