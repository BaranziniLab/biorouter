import { useCallback, useEffect, useRef, useState } from 'react';
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
  /** The daemon-priced cost when the row has one, else the pricing table's; null when unknown. */
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

/** The three lifetime counters of a chat's live `tokenState`. */
export interface LiveTokenCounters {
  accumulatedInputTokens?: number | null;
  accumulatedOutputTokens?: number | null;
  accumulatedTotalTokens?: number | null;
}

interface UseCostTrackingProps {
  session?: Session | null;
  /**
   * This chat's live counters: the stream's `tokenState`. In a live chat the
   * `session` row is the one the chat was opened with, and its counters never
   * move (a decision chat's reply moves them before the row is read again), so
   * without these the ledger was read once on open and the footer's cost stayed
   * where it was until a reload. They move when a turn records usage (once per
   * provider call, and on the turn's end), never per streamed token: every
   * `Message` frame carries a copy of the last counters (BR-52).
   */
  liveTokens?: LiveTokenCounters | null;
  /**
   * Whether a turn is running in this chat (`isRunningState` of its `chatState`). When a turn
   * ends the ledger is read again on `LATE_USAGE_READS_MS`, and while the chat is idle a return
   * to the window reads it once more: side calls the daemon bills after the turn moved nothing
   * this hook watches.
   */
  turnRunning?: boolean;
}

/**
 * When the ledger is read again after a turn in an open chat ends, in milliseconds from the end.
 * The daemon bills side calls after the turn's last frame: an eager or background compaction of a
 * long prompt, or one that failed and still recorded its usage (`record_unapplied_compaction_usage`,
 * which does not stamp the chat's row). None of them moves a counter this hook watches, so the
 * footer's cost and the summary's Tokens kept the turn's figures until a reload (2026-11-13, a
 * Claude Haiku 5.5 chat on Versa Bedrock: $0.127 shown, about $0.19 more billed). Five reads reach
 * past a compaction of a long prompt; a new turn, a switch of chat or an unmount cancels the rest.
 */
export const LATE_USAGE_READS_MS: readonly number[] = [10_000, 30_000, 90_000, 180_000, 300_000];

/**
 * A return to the window reads the ledger once: its `focus` and `visibilitychange` arrive
 * together, so a second one within this gap asks for nothing.
 */
export const RETURN_READ_GAP_MS = 2_000;

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
 * The cost the daemon priced for a row (`ModelUsageRow.cost` and `costIsPartial`), or `null`
 * when the row carries none. The daemon prices each request on its own (the cache buckets at the
 * cache rate, a request over a long-context threshold at the long-context rate), which a flat
 * per-token price here cannot. Read through a narrow guard rather than the generated type, so a
 * daemon that predates the fields, or a row it left unpriced, falls back to the client price.
 */
export function serverPricedCost(row: unknown): { cost: number; partial: boolean } | null {
  if (!row || typeof row !== 'object') return null;
  const { cost, costIsPartial } = row as { cost?: unknown; costIsPartial?: unknown };
  if (typeof cost !== 'number' || !Number.isFinite(cost) || cost < 0) return null;
  return { cost, partial: costIsPartial === true };
}

/**
 * Price the real per-model rows the backend recorded: the daemon's own price where the row
 * carries one (`serverPricedCost`), else the client price. `pricingFor` returns the
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
      const server = serverPricedCost(row);
      if (server) {
        totalCost = server.cost;
        costIsPartial = server.partial;
      } else if (provider && model) {
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

/** A chat's rows before its ledger answers: one array, so the prop stays stable. */
const NO_ROWS: ModelCostRow[] = [];

/** Every token the rows count, in all four buckets. A chat's ledger only grows while it lives. */
export function ledgerTokenCount(rows: ModelCostRow[]): number {
  return rows.reduce(
    (sum, row) =>
      sum +
      row.inputTokens +
      row.outputTokens +
      (row.cacheReadTokens ?? 0) +
      (row.cacheCreationTokens ?? 0),
    0
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
 *
 * `ledgerSettled` says whether THIS chat's ledger has answered (with rows, or
 * by failing). Until it has, `modelRows` is empty, and a reader that falls back
 * to the accumulated counters shows a figure the ledger then replaces: the
 * summary's Tokens value and the footer cost read "2.2M" and "$11.75", then
 * "N/A" and nothing, on every open (vision V2-R5-5). So a reader waits for it.
 * The answer is kept with the chat it was read for, so one chat's ledger never
 * stands in for another's, and a refresh of the same chat (a turn moved its
 * counters) keeps the answer it has until the new one lands. A refresh that
 * fails keeps it too: only a chat's first read settles with no rows.
 *
 * The ledger is re-read when the chat's counters move, on its row or live
 * (`liveTokens`). The row's counters move on an open or a resume; in a live
 * chat only the stream's do, once per provider call that records usage and on
 * the turn's end, each after the daemon wrote the turn's ledger row. They are
 * compared by value, so the per-token frames that copy them ask for nothing.
 * A read that a newer one has superseded never lands (`cancelled`).
 *
 * `reload` reads it again on demand. The cost card calls it each time it opens,
 * beside its Limit section's own read, so the card's cost and its limit line
 * count the same requests: a side call made after the turn's last frame (the
 * chat's title, a knowledge macro) is in both, not in the limit alone.
 *
 * A side call billed later still (a background compaction) reaches the footer's
 * cost and the summary's Tokens without the card: after a turn ends the ledger
 * is read again on `LATE_USAGE_READS_MS`, and once more when the window comes
 * back while the chat is idle. Nothing is read on a timer while a turn runs.
 * A read that counts fewer tokens than the rows held never replaces them, so a
 * stale answer can never lower the figures.
 */
export const useCostTracking = ({
  session,
  liveTokens,
  turnRunning = false,
}: UseCostTrackingProps) => {
  const [ledger, setLedger] = useState<{ sessionId: string; rows: ModelCostRow[] } | null>(null);
  const [reads, setReads] = useState(0);
  const reload = useCallback(() => setReads((count) => count + 1), []);

  const sessionId = session?.id;
  // Some providers report only the split and others only the total. Observe all
  // three counters so either shape refreshes the authoritative ledger.
  const accumulatedInput = session?.accumulated_input_tokens;
  const accumulatedOutput = session?.accumulated_output_tokens;
  const accumulatedTotal = session?.accumulated_total_tokens;
  const liveInput = liveTokens?.accumulatedInputTokens;
  const liveOutput = liveTokens?.accumulatedOutputTokens;
  const liveTotal = liveTokens?.accumulatedTotalTokens;

  useEffect(() => {
    if (!sessionId) return;
    let cancelled = false;

    const load = async () => {
      try {
        // With the user's proof: a private chat's usage is refused to a caller
        // without it (issue #56, QA 2026-09-10).
        const response = await getSessionUsage({
          path: { session_id: sessionId },
          headers: await userActionHeaders(),
          throwOnError: false,
        });
        // A refused or failed read is not an answer with no rows.
        if (!response.data) throw new Error('the usage ledger did not answer');
        const priced = await buildModelCostRows(response.data.models ?? [], fetchModelPricing);
        if (!cancelled) {
          // The ledger only grows while the chat lives: an answer that counts fewer tokens than
          // the one held is stale, and never lowers the figures.
          setLedger((held) =>
            held?.sessionId === sessionId && ledgerTokenCount(priced) < ledgerTokenCount(held.rows)
              ? held
              : { sessionId, rows: priced }
          );
        }
      } catch {
        if (!cancelled) {
          setLedger((held) => (held?.sessionId === sessionId ? held : { sessionId, rows: [] }));
        }
      }
    };

    load();
    return () => {
      cancelled = true;
    };
  }, [
    sessionId,
    accumulatedInput,
    accumulatedOutput,
    accumulatedTotal,
    liveInput,
    liveOutput,
    liveTotal,
    reads,
  ]);

  // After a turn ends in this chat, read the ledger again on a short back-off: a side call billed
  // after the turn's last frame moves no counter above. The ref says which chat this hook saw a
  // turn run in; it is not cleared when the schedule starts, so React's development double run of
  // an effect schedules again what its cleanup cancelled. A new turn, a switch of chat or an
  // unmount cancels what is left of the schedule.
  const turnSeenIn = useRef<string | null>(null);
  useEffect(() => {
    if (!sessionId) return;
    if (turnRunning) {
      turnSeenIn.current = sessionId;
      return;
    }
    if (turnSeenIn.current !== sessionId) {
      turnSeenIn.current = null;
      return;
    }
    const timers = LATE_USAGE_READS_MS.map((delay) => setTimeout(reload, delay));
    return () => {
      for (const timer of timers) clearTimeout(timer);
    };
  }, [sessionId, turnRunning, reload]);

  // A return to the window while the chat is idle reads it once more, for a side call billed
  // after the schedule above ended or while the window was away.
  useEffect(() => {
    if (!sessionId || turnRunning) return;
    let lastRead = Number.NEGATIVE_INFINITY;
    const readOnReturn = () => {
      if (document.visibilityState === 'hidden') return;
      const now = Date.now();
      if (now - lastRead < RETURN_READ_GAP_MS) return;
      lastRead = now;
      reload();
    };
    window.addEventListener('focus', readOnReturn);
    document.addEventListener('visibilitychange', readOnReturn);
    return () => {
      window.removeEventListener('focus', readOnReturn);
      document.removeEventListener('visibilitychange', readOnReturn);
    };
  }, [sessionId, turnRunning, reload]);

  const ledgerIsThisChats = !!sessionId && ledger?.sessionId === sessionId;
  const modelRows = ledgerIsThisChats ? ledger.rows : NO_ROWS;

  // Back-compat shape for older CostTracker callers. Unknown and partial rows
  // stay in modelRows instead of being forged into exact legacy costs here.
  const sessionCosts = buildLegacySessionCosts(modelRows);

  return {
    sessionCosts,
    modelRows,
    /** No chat, or this chat's ledger has answered. */
    ledgerSettled: !sessionId || ledgerIsThisChats,
    /** Reads this chat's ledger again (the cost card, on open). Stable. */
    reload,
  };
};
