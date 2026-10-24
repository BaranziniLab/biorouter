/**
 * How the two Approvals settings read off the daemon's config, in one place: the section reads
 * them when it mounts, and `warmSettings.ts` reads them while Settings opens, and the two must
 * mean the same thing by "nothing saved".
 */

export const APPROVAL_MODE_KEY = 'BIOROUTER_MODE';
export const MAX_TURNS_KEY = 'BIOROUTER_MAX_TURNS';

/** What `BIOROUTER_MODE` means when nothing is saved: the daemon's own default. */
export const DEFAULT_MODE = 'auto';

/** The saved approval mode. Nothing saved is the daemon's default, Autonomous. */
export function approvalModeFromConfig(value: unknown): string {
  return typeof value === 'string' && value ? value : DEFAULT_MODE;
}

/**
 * The saved Max turns exactly as stored, or `null` when none is saved (the field then shows the
 * agent's default). Zero means Unlimited; invalid saved numbers remain visible for correction.
 */
export function maxTurnsFromConfig(value: unknown): number | null {
  const stored = typeof value === 'string' && value.trim() !== '' ? Number(value) : value;
  return typeof stored === 'number' && Number.isFinite(stored) ? stored : null;
}
