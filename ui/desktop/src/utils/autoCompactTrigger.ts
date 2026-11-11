/**
 * Where Biorouter really auto-compacts a chat, read the way the daemon reads it
 * (`auto_compact_trigger_tokens` in crates/biorouter/src/context_mgmt/mod.rs): at
 * the LOWER of the threshold share of the window (`BIOROUTER_AUTO_COMPACT_THRESHOLD`)
 * and an absolute token ceiling (`BIOROUTER_AUTO_COMPACT_MAX_TOKENS`, 200,000 unless
 * set; 0 removes it). On a window over 250k tokens the ceiling is what fires: a
 * 1,050,000-token model compacts at 200k, about 19% used, not at 80%.
 *
 * The ceiling never turns automatic compaction back on: with the threshold off,
 * nothing compacts.
 */

export const AUTO_COMPACT_MAX_TOKENS_KEY = 'BIOROUTER_AUTO_COMPACT_MAX_TOKENS';
export const DEFAULT_AUTO_COMPACT_MAX_TOKENS = 200_000;

/**
 * The ceiling a saved value makes the daemon use, or `null` for none. The daemon
 * reads a whole number of tokens and falls back to the default for anything it
 * cannot read as one, so a missing, fractional or non-numeric value is the
 * default here too, and 0 is "no ceiling".
 */
export function savedAutoCompactCeiling(v: unknown): number | null {
  let tokens: number | null = null;
  if (typeof v === 'number' && Number.isInteger(v) && v >= 0) {
    tokens = v;
  } else if (typeof v === 'string' && /^\d+$/.test(v.trim())) {
    tokens = Number(v.trim());
  }
  if (tokens === null) return DEFAULT_AUTO_COMPACT_MAX_TOKENS;
  return tokens > 0 ? tokens : null;
}

export interface AutoCompactTrigger {
  /** The share of the window, in percent (0 to 100, unrounded), where it fires. */
  pct: number;
  /** Tokens at which it fires. */
  tokens: number;
  /** The ceiling, not the share, is what fires first. */
  byCeiling: boolean;
}

/**
 * The effective trigger for a window of `windowTokens`, given the threshold as a
 * percent (`thresholdPct`, `null` when automatic compaction is off) and the
 * ceiling (`null` for none). `null` when nothing compacts or there is no window.
 */
export function effectiveAutoCompactTrigger(
  thresholdPct: number | null,
  windowTokens: number,
  ceiling: number | null
): AutoCompactTrigger | null {
  if (thresholdPct === null || !(windowTokens > 0)) return null;
  const byShare = (windowTokens * thresholdPct) / 100;
  if (ceiling !== null && ceiling < byShare) {
    return { pct: (ceiling / windowTokens) * 100, tokens: ceiling, byCeiling: true };
  }
  return { pct: thresholdPct, tokens: byShare, byCeiling: false };
}
