/**
 * The rule a new schedule's name has to satisfy, stated at the field instead of
 * discovered by submitting.
 *
 * ⚠ Mirrored from `validate_schedule_id` in `crates/biorouter/src/scheduler.rs`,
 * which is a **security** boundary rather than a tidiness rule — the name is
 * interpolated into a filename, so the daemon's copy is the authority and stays
 * the authority. This one is a courtesy.
 *
 * That makes the drift direction the safe one. If the daemon's rule tightens and
 * this one does not, the request is refused and the user reads the daemon's own
 * sentence, which `createSchedule` now carries through instead of collapsing
 * into "Unexpected response format". The reverse — loosening the daemon's rule
 * so it matches something written here — is never the fix.
 *
 * Kept free of React and of the API client so the rule can be driven directly by
 * a test rather than only through a rendered dialog.
 */

/** `MAX_SCHEDULE_ID_LEN` in `scheduler.rs`. A filename still has to fit. */
export const MAX_SCHEDULE_NAME_LENGTH = 64;

/**
 * What is wrong with `name` as a schedule name, or `null` when nothing is.
 *
 * Takes the name exactly as it will be sent — trimming, if any, belongs to the
 * caller, so that this cannot report a problem with a string the daemon never
 * sees.
 */
export function scheduleNameProblem(name: string): string | null {
  if (name.length === 0) {
    return 'Enter a name for this schedule.';
  }
  if (name.length > MAX_SCHEDULE_NAME_LENGTH) {
    return `The name must be ${MAX_SCHEDULE_NAME_LENGTH} characters or fewer.`;
  }
  if (!/^[A-Za-z0-9_-]+$/.test(name)) {
    return "The name may only contain letters, digits, '-' and '_'.";
  }
  return null;
}
