/**
 * Shared pieces of the usage limit controls: the composer's cost card (per chat) and
 * Settings > App > Usage (monthly and default per chat). The daemon owns the limits
 * (`crates/biorouter/src/agents/usage_limit.rs`, docs/agent-loop/usage-limits.md); these only
 * parse what a person types, format an amount, and tell every open composer when Settings
 * changed a limit so its paused state is read again.
 */

import { formatUsd } from '../../utils/formatUsd';

/** The unit a person sets a limit in. */
export type LimitUnit = 'usd' | 'tokens';

/**
 * The result of reading a limit field. `precision` is a dollar amount with more than
 * {@link MAX_USD_DECIMALS} decimals, which is refused rather than rounded.
 */
export type ParsedLimit = { ok: true; value: number } | { ok: false; reason?: 'precision' };

/** The most decimals a dollar limit keeps: $0.0125 is a limit, $0.01255 is refused. */
export const MAX_USD_DECIMALS = 4;

const MULTIPLIERS: Record<string, number> = { k: 1_000, m: 1_000_000, b: 1_000_000_000 };

/** A number whose commas are all well-formed thousands groups: "1,000", "$1,250.50", "1,500k". */
const THOUSANDS_GROUPED = /^\$?\d{1,3}(,\d{3})+(\.\d*)?[kmb]?$/i;

/**
 * Reads a limit as typed: "$5", "5.50", "0.013" and "1,000" for dollars; "250000", "250,000",
 * "250k" and "1.5m" for tokens. A dollar amount must be above 0 and is kept exactly as typed, to
 * at most four decimals (more is refused, never rounded, so a sub-cent limit is stored as the
 * person wrote it); a token count is rounded to a whole number and must be at least 1. Anything
 * else is not a limit.
 *
 * A comma is read only as a thousands separator ("1,000", "250,000"). Any other comma is refused
 * rather than dropped: "0,50" written with a decimal comma would otherwise become $50, a limit a
 * hundred times what the person meant.
 */
export function parseLimitInput(text: string, unit: LimitUnit): ParsedLimit {
  const compact = text.trim().replace(/[_\s]/g, '');
  if (compact.includes(',') && !THOUSANDS_GROUPED.test(compact)) return { ok: false };
  let cleaned = compact.replace(/,/g, '');
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
  const decimals = cleaned.includes('.') ? cleaned.length - cleaned.indexOf('.') - 1 : 0;
  if (decimals > MAX_USD_DECIMALS) return { ok: false, reason: 'precision' };
  return { ok: true, value: parsed };
}

/** "$5.00", "$1,250.00", "$0.013": the shared dollar format (`utils/formatUsd.ts`). */
export function formatLimitUsd(amount: number): string {
  return formatUsd(amount);
}

/** "250,000 tokens", "1 token". */
export function formatLimitTokens(count: number): string {
  const whole = Math.round(count);
  return `${whole.toLocaleString()} ${whole === 1 ? 'token' : 'tokens'}`;
}

/** What a limit field is seeded with: the stored amount as a person would type it. */
export function limitInputValue(amount: number | null | undefined, unit: LimitUnit): string {
  if (amount == null || !Number.isFinite(amount) || amount <= 0) return '';
  // A dollar limit is stored as typed (at most four decimals), so it is shown back the same way.
  return unit === 'usd'
    ? String(Number(amount.toFixed(MAX_USD_DECIMALS)))
    : String(Math.round(amount));
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
