/**
 * Shared pieces of the usage limit controls: the composer's cost card (per chat) and
 * Settings > App > Usage (monthly and default per chat). The daemon owns the limits
 * (`crates/biorouter/src/agents/usage_limit.rs`, docs/agent-loop/usage-limits.md); these only
 * parse what a person types, format an amount, and tell every open composer when Settings
 * changed a limit so its paused state is read again.
 */

/** The unit a person sets a limit in. */
export type LimitUnit = 'usd' | 'tokens';

/** The result of reading a limit field. */
export type ParsedLimit = { ok: true; value: number } | { ok: false };

const MULTIPLIERS: Record<string, number> = { k: 1_000, m: 1_000_000, b: 1_000_000_000 };

/**
 * Reads a limit as typed: "$5", "5.50" and "1,000" for dollars; "250000", "250,000", "250k" and
 * "1.5m" for tokens. A dollar amount must be above 0; a token count is rounded to a whole number
 * and must be at least 1. Anything else is not a limit.
 */
export function parseLimitInput(text: string, unit: LimitUnit): ParsedLimit {
  let cleaned = text.trim().replace(/[,_\s]/g, '');
  if (unit === 'usd') cleaned = cleaned.replace(/^\$/, '');
  let multiplier = 1;
  if (unit === 'tokens') {
    const suffix = cleaned.slice(-1).toLowerCase();
    if (suffix in MULTIPLIERS) {
      multiplier = MULTIPLIERS[suffix];
      cleaned = cleaned.slice(0, -1);
    }
  }
  if (!/^\d*\.?\d+$/.test(cleaned) && !/^\d+\.$/.test(cleaned)) return { ok: false };
  const parsed = Number(cleaned) * multiplier;
  if (!Number.isFinite(parsed) || parsed <= 0) return { ok: false };
  if (unit === 'tokens') {
    const whole = Math.round(parsed);
    return whole >= 1 ? { ok: true, value: whole } : { ok: false };
  }
  return { ok: true, value: Math.round(parsed * 100) / 100 || parsed };
}

/** "$5.00", "$1,250.00", "<$0.01". */
export function formatLimitUsd(amount: number): string {
  if (amount > 0 && amount < 0.01) return '<$0.01';
  return `$${amount.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

/** "250,000 tokens", "1 token". */
export function formatLimitTokens(count: number): string {
  const whole = Math.round(count);
  return `${whole.toLocaleString()} ${whole === 1 ? 'token' : 'tokens'}`;
}

/** What a limit field is seeded with: the stored amount as a person would type it. */
export function limitInputValue(amount: number | null | undefined, unit: LimitUnit): string {
  if (amount == null || !Number.isFinite(amount) || amount <= 0) return '';
  return unit === 'usd' ? String(Math.round(amount * 100) / 100) : String(Math.round(amount));
}

/**
 * Said on `window` when Settings changes the monthly or default per-chat limit, so every open
 * composer reads its chat's limit again: the change can pause or resume a chat the person is not
 * looking at. `detail.sessionId` is set when one chat's own limit changed.
 */
export const USAGE_LIMITS_CHANGED_EVENT = 'biorouter:usage-limits-changed';

export interface UsageLimitsChangedDetail {
  sessionId?: string;
  /** Who said it, so the composer that made the change does not read it back again. */
  source?: unknown;
}

export function announceUsageLimitsChanged(sessionId?: string, source?: unknown): void {
  if (typeof window === 'undefined') return;
  const detail: UsageLimitsChangedDetail = { sessionId, source };
  window.dispatchEvent(new CustomEvent(USAGE_LIMITS_CHANGED_EVENT, { detail }));
}
