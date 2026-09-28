import type { CrewWorkspaceUsage } from '../crewApi';

/** From this share of either budget, the host is told (M1). */
export const FULLNESS_WARN_PERCENT = 80;
/** From this share, the note says so more urgently. */
export const FULLNESS_URGENT_PERCENT = 95;

/**
 * How full a workspace is, for its host: `percent` of the budget ordinary changes may use, from
 * the fuller of the logical state and the audit journal, and where that leaves it.
 *
 * - `ok`: under 80%. The number is still shown in Workspace settings; nothing warns.
 * - `warn`: 80% or more.
 * - `urgent`: 95% or more.
 * - `full`: ordinary changes are refused now; only the host's removals and privacy changes work.
 */
export interface WorkspaceFullness {
  percent: number;
  level: 'ok' | 'warn' | 'urgent' | 'full';
}

/**
 * The share of one budget in use, or `null` when the budget says nothing usable. Ordinary changes
 * stop at the limit less the headroom the broker keeps for the host's removals and policy changes
 * (`commit` in `biorouter-crew/src/broker.rs`), so the share is of that, not of the whole limit:
 * measured against the whole limit, a workspace that refuses every post would read 94% full.
 */
function share(bytes: number, limit: number, headroom = 0): number | null {
  const room = limit - headroom;
  if (!Number.isFinite(room) || room <= 0 || !Number.isFinite(bytes) || bytes < 0) return null;
  return bytes / room;
}

/**
 * The host's `usage` (W2-BRK-7) as one reading (W2-UIW-20), or `null` when there is none to give:
 * a member's snapshot, an older broker that sends no usage, or budgets that make no sense. Only
 * the logical state and the journal count: when either is full every ordinary change is refused,
 * while a full attachment space only refuses new attachments.
 */
export function workspaceFullness(
  usage: CrewWorkspaceUsage | null | undefined
): WorkspaceFullness | null {
  if (!usage) return null;
  const shares = [
    share(usage.state_bytes, usage.state_limit, usage.state_admin_headroom),
    share(usage.journal_bytes, usage.journal_limit, usage.journal_admin_headroom),
  ].filter((value): value is number => value !== null);
  if (shares.length === 0) return null;
  const fullest = Math.max(...shares);
  // Floored, so "100% full" is never said of a workspace that still takes a post.
  const percent = Math.min(100, Math.floor(fullest * 100));
  const level =
    fullest >= 1
      ? 'full'
      : percent >= FULLNESS_URGENT_PERCENT
        ? 'urgent'
        : percent >= FULLNESS_WARN_PERCENT
          ? 'warn'
          : 'ok';
  return { percent: level === 'full' ? 100 : percent, level };
}

/** Whether the host should be told: 80% or more of either budget. */
export function fullnessNeedsAttention(fullness: WorkspaceFullness | null): boolean {
  return fullness !== null && fullness.level !== 'ok';
}

const RANK: Record<WorkspaceFullness['level'], number> = { ok: 0, warn: 1, urgent: 2, full: 3 };

/** Whether `after` crossed into a more pressing level than `before`: said once, when it does. */
export function fullnessRose(
  before: WorkspaceFullness | null,
  after: WorkspaceFullness | null
): boolean {
  if (!before || !after) return false;
  return RANK[after.level] > RANK[before.level];
}
