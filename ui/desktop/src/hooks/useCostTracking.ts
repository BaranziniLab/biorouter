import { useEffect, useState } from 'react';
import { fetchModelPricing } from '../utils/pricing';
import { getSessionUsage, ModelUsageRow, Session } from '../api';
import { userActionHeaders } from '../utils/userAction';
import { billedTokens } from '../utils/usageAccounting';

export interface ModelCostRow {
  /** Provider that served the turns, or undefined for the unknown bucket. */
  provider?: string;
  /** Model that served the turns, or undefined for the unknown bucket. */
  model?: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number | null;
  cacheCreationTokens: number | null;
  totalTokens: number | null;
  turns: number;
  /** Client-side cost from the pricing table, or null when pricing is unknown. */
  totalCost: number | null;
  /** Historical usage or pricing cannot certify the known subtotal as exact. */
  costIsPartial: boolean;
}

export interface SessionCostRow {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens?: number;
  cacheCreationTokens?: number;
  totalCost: number | null;
  costIsPartial?: boolean;
}

export type SessionCosts = Record<string, SessionCostRow>;

type LegacySessionCosts = Record<
  string,
  { inputTokens: number; outputTokens: number; totalCost: number }
>;

interface UseCostTrackingProps {
  session?: Session | null;
}

/**
 * The label a `null` model_id row (turns recorded before model attribution, or
 * providers that reported no model) shows in the breakdown.
 */
export const UNKNOWN_MODEL_LABEL = 'unknown';

/** Stable key for a `(provider, model)` group, matching the old cost map keys. */
export function modelRowKey(provider?: string | null, model?: string | null): string {
  return `${provider ?? UNKNOWN_MODEL_LABEL}/${model ?? UNKNOWN_MODEL_LABEL}`;
}

export function buildLegacySessionCosts(rows: ModelCostRow[]): LegacySessionCosts {
  const sessionCosts: LegacySessionCosts = {};
  for (const row of rows) {
    if (row.totalCost === null || row.costIsPartial) continue;
    sessionCosts[modelRowKey(row.provider, row.model)] = {
      inputTokens: row.inputTokens,
      outputTokens: row.outputTokens,
      totalCost: row.totalCost,
    };
  }
  return sessionCosts;
}

/**
 * Price the real per-model rows the backend recorded. `pricingFor` returns the
 * per-token input/output cost for a `(provider, model)` pair, or null when the
 * pricing table has no entry — that row's cost stays null (token-only), never
 * zero, so an unknown price is visibly distinct from a genuinely free model.
 */
export async function buildModelCostRows(
  rows: ModelUsageRow[],
  pricingFor: (
    provider: string,
    model: string
  ) => Promise<{
    input_token_cost?: number | null;
    output_token_cost?: number | null;
    cache_read_cost?: number | null;
    cache_write_cost?: number | null;
  } | null>
): Promise<ModelCostRow[]> {
  return Promise.all(
    rows.map(async (row) => {
      const provider = row.provider ?? undefined;
      const model = row.modelId ?? undefined;
      const exactTotal = billedTokens(row);
      let totalCost: number | null = null;
      let costIsPartial = false;
      if (provider && model) {
        const pricing = await pricingFor(provider, model);
        if (pricing) {
          const buckets = [
            [row.inputTokens, pricing.input_token_cost],
            [row.outputTokens, pricing.output_token_cost],
            [row.cacheReadTokens, pricing.cache_read_cost],
            [row.cacheCreationTokens, pricing.cache_write_cost],
          ] as const;
          let knownSubtotal = 0;
          let hasKnownRate = false;
          costIsPartial = exactTotal === null;
          for (const [tokens, rate] of buckets) {
            if (typeof tokens !== 'number' || !Number.isFinite(tokens) || tokens < 0) {
              costIsPartial = true;
              continue;
            }
            if (typeof rate === 'number' && Number.isFinite(rate) && rate >= 0) {
              knownSubtotal += tokens * rate;
              hasKnownRate = true;
            } else if (tokens > 0) {
              costIsPartial = true;
            }
          }
          if (exactTotal === 0) {
            totalCost = 0;
            costIsPartial = false;
          } else if (hasKnownRate && (!costIsPartial || knownSubtotal > 0)) {
            totalCost = knownSubtotal;
          }
        }
      }
      return {
        provider,
        model,
        inputTokens: row.inputTokens,
        outputTokens: row.outputTokens,
        cacheReadTokens: row.cacheReadTokens ?? null,
        cacheCreationTokens: row.cacheCreationTokens ?? null,
        totalTokens: exactTotal,
        turns: row.turns,
        totalCost,
        costIsPartial,
      };
    })
  );
}

/**
 * Per-model cost tracking backed by the real `token_events` ledger.
 *
 * The previous implementation *guessed* the split by watching model-change
 * events in the renderer and attributing the whole current session total to
 * whichever model was active at the moment of the switch — it never saw turns
 * from before the app opened and mis-attributed everything after a mid-thread
 * switch (Issue #1). This fetches the authoritative `(model, provider)` rollup
 * from `GET /sessions/{id}/usage` instead and only prices it client-side.
 */
export const useCostTracking = ({ session }: UseCostTrackingProps) => {
  const [modelRows, setModelRows] = useState<ModelCostRow[]>([]);

  const sessionId = session?.id;
  // Some providers report only the split and others only the total. Observe all
  // three counters so either shape refreshes the authoritative ledger.
  const accumulatedInput = session?.accumulated_input_tokens;
  const accumulatedOutput = session?.accumulated_output_tokens;
  const accumulatedTotal = session?.accumulated_total_tokens;

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      if (!sessionId) {
        setModelRows([]);
        return;
      }
      try {
        // With the user's proof: a private chat's usage is refused to a caller
        // without it (issue #56, QA 2026-09-10).
        const response = await getSessionUsage({
          path: { session_id: sessionId },
          headers: await userActionHeaders(),
          throwOnError: false,
        });
        const rows = response.data?.models ?? [];
        const priced = await buildModelCostRows(rows, fetchModelPricing);
        if (!cancelled) {
          setModelRows(priced);
        }
      } catch {
        if (!cancelled) {
          setModelRows([]);
        }
      }
    };

    load();
    return () => {
      cancelled = true;
    };
  }, [sessionId, accumulatedInput, accumulatedOutput, accumulatedTotal]);

  // Back-compat shape for older CostTracker callers. Unknown and partial rows
  // stay in modelRows instead of being forged into exact legacy costs here.
  const sessionCosts = buildLegacySessionCosts(modelRows);

  return {
    sessionCosts,
    modelRows,
  };
};
