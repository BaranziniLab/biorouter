import { Fragment, useEffect, useId, useRef, useState } from 'react';
import { useModelAndProvider } from '../ModelAndProviderContext';
import { Gauge } from '../icons/app-icons';
import { COST_CARD_COPY, FOOTER_COPY, USAGE_LIMIT_COPY } from './copy';
import './pickers.css';
import { formatUsd } from '../../utils/formatUsd';
import { fetchModelPricing } from '../../utils/pricing';
import { PricingData } from '../../api';
import {
  modelRowKey,
  type ModelCostRow,
  type SessionCostRow,
  type SessionCosts,
} from '../../hooks/useCostTracking';
import { modelLabel } from './ModelBreakdownTable';
import {
  FooterCard,
  FooterCardDetail,
  FooterCardList,
  FooterCardRoot,
  FooterCardRow,
  FooterCardSection,
  FooterCardTrigger,
  FooterCardValue,
} from './FooterCard';
import { FooterCardHelpSection } from './FooterCardHelpSection';
import { useFooterCard } from './useFooterCard';
import { isUnmeteredDecisionProvider } from '../decisions/decisionWire';
import { UsageLimitSection } from './UsageLimitSection';
import type { SessionUsageLimitController } from './useSessionUsageLimit';

interface CostTrackerProps {
  inputTokens?: number;
  outputTokens?: number;
  sessionCosts?: SessionCosts;
  modelCostRows?: ModelCostRow[];
  /**
   * Reads the chat's usage ledger again (`useCostTracking`'s `reload`). Called each time the card
   * opens, with the Limit section's own read, so the cost and the limit line count the same
   * requests: a side call made after the turn's last frame, such as the chat's title, is in both.
   */
  reloadCost?: () => void;
  /**
   * Price only the usage ledger, never the bare counters. A decision chat's
   * tokens belong to its decision model, and the fallback prices counters at
   * the app's current model, which is not the chat's: a local decision chat
   * showed "<$0.01" at GPT-5.5's rate for a request that costs nothing.
   */
  ledgerOnly?: boolean;
  /**
   * This chat's usage limit (`useSessionUsageLimit`, owned by `ChatInput`). Given, the card gains
   * its Limit section and the trigger is always on the line, even with no cost to show, in a new
   * chat, and with Show costs off (then it shows no dollar figure).
   */
  usageLimit?: SessionUsageLimitController;
  /** Opens Settings > App > Usage, where the monthly and default limits are set. */
  onOpenUsageSettings?: () => void;
}

export interface CostEstimate {
  amount: number | null;
  partial: boolean;
}

/**
 * A row of requests to a local decision model, which has no price because it
 * has no meter. It costs nothing, so it counts as $0 and leaves the chat's
 * total exact, as the usage report does.
 */
export function isUnmeteredDecisionRow(
  row: Pick<ModelCostRow, 'provider'> & Pick<SessionCostRow, 'totalCost'>
): boolean {
  return row.totalCost === null && isUnmeteredDecisionProvider(row.provider);
}

/**
 * The total of priced rows: the ledger's per-model rows, or the legacy per-model map's values. A
 * row with no price makes the total a conservative subtotal; no priced row at all leaves it
 * unavailable rather than $0. An unmetered decision model's row costs nothing and counts as $0.
 */
export function aggregateModelRowsCost(
  rows: (Pick<SessionCostRow, 'totalCost' | 'costIsPartial'> & Pick<ModelCostRow, 'provider'>)[]
): CostEstimate {
  let amount = 0;
  let hasKnownCost = false;
  let partial = false;
  for (const row of rows) {
    if (isUnmeteredDecisionRow(row)) {
      hasKnownCost = true;
    } else if (row.totalCost === null) {
      partial = true;
    } else {
      amount += row.totalCost;
      hasKnownCost = true;
    }
    partial ||= row.costIsPartial ?? false;
  }
  return { amount: hasKnownCost ? amount : null, partial };
}

/**
 * A cost as the footer, its card and the per-model rows print it: the shared dollar format
 * (`utils/formatUsd.ts`), so the chat's total and its one model's row read the same figure
 * ("$0.0085" in both, never "$0.01" beside "<$0.01"). Another currency symbol takes the same
 * digits.
 */
export function formatTooltipMoney(amount: number | null, currency = '$'): string {
  if (amount === null || !Number.isFinite(amount) || amount < 0) return 'Unavailable';
  const usd = formatUsd(amount);
  return currency === '$' ? usd : usd.replace('$', currency);
}

export function formatCostEstimate(estimate: CostEstimate, currency = '$'): string {
  return formatTooltipMoney(estimate.amount, currency);
}

/**
 * Whether the footer shows a figure at all (spec 3.7): only a cost above $0.
 * "$0.00" and "Unavailable" are not worth a place on the line; the card is
 * still one hover away once there is something to break down.
 */
export function costIsWorthShowing(estimate: CostEstimate): boolean {
  return estimate.amount !== null && Number.isFinite(estimate.amount) && estimate.amount > 0;
}

/**
 * Everything the cost card shows, whichever of the three data paths produced it, so the three
 * draw ONE card: the ledger's per-model rows, the legacy per-model map, or the counters priced at
 * the current model's rate.
 *
 * The tokens are the ledger's four disjoint buckets, so on the ledger path they add up to the
 * billed total the summary's Tokens value (`mostCompleteBilledTokens`) and Settings > App > Usage
 * show: a card that listed input and output alone read "22,788 input · 382 output" beside a
 * Tokens value of 89k, the other 65,408 being cache reads.
 */
export interface CostSummary {
  estimate: CostEstimate;
  currency: string;
  inputTokens: number;
  outputTokens: number;
  /**
   * Prompt tokens read from the cache, summed over the rows that recorded them; `null` where no
   * row did. The counters the live price reads never split them out, so that path says nothing.
   */
  cacheReadTokens: number | null;
  /** Prompt tokens written to the cache, the same way. */
  cacheCreationTokens: number | null;
  /** The ledger's rows, one per model, for the card's By model section; empty on the other paths. */
  models: ModelCostRow[];
}

/**
 * One cache bucket summed over the rows that recorded it, or `null` when none did. A row whose
 * history predates cache accounting adds nothing rather than a guess, so the sum is then a lower
 * bound; such a row's cost is partial too, and the card says the total is an estimate.
 */
export function recordedTokenSum(values: (number | null | undefined)[]): number | null {
  let sum: number | null = null;
  for (const value of values) {
    if (typeof value === 'number' && Number.isFinite(value) && value >= 0) {
      sum = (sum ?? 0) + value;
    }
  }
  return sum;
}

/**
 * Which data the footer prices, in order: the ledger's rows, else the legacy map, else the
 * counters at the current model's rate (`pricing`, `null` while it loads or when the model has
 * no price). The first path that has rows decides, even when its total is not worth showing:
 * a ledger that priced the chat at nothing never falls back to pricing the counters. `null`
 * draws nothing on the line.
 */
export function costSummary({
  modelCostRows,
  sessionCosts,
  inputTokens,
  outputTokens,
  pricing,
}: {
  modelCostRows?: ModelCostRow[];
  sessionCosts?: SessionCosts;
  inputTokens: number;
  outputTokens: number;
  pricing: PricingData | null;
}): CostSummary | null {
  if (modelCostRows && modelCostRows.length > 0) {
    const estimate = aggregateModelRowsCost(modelCostRows);
    if (!costIsWorthShowing(estimate)) return null;
    return {
      estimate,
      currency: '$',
      inputTokens: modelCostRows.reduce((sum, row) => sum + row.inputTokens, 0),
      outputTokens: modelCostRows.reduce((sum, row) => sum + row.outputTokens, 0),
      cacheReadTokens: recordedTokenSum(modelCostRows.map((row) => row.cacheReadTokens)),
      cacheCreationTokens: recordedTokenSum(modelCostRows.map((row) => row.cacheCreationTokens)),
      models: modelCostRows,
    };
  }

  const legacyRows = sessionCosts ? Object.values(sessionCosts) : [];
  if (legacyRows.length > 0) {
    const estimate = aggregateModelRowsCost(legacyRows);
    if (!costIsWorthShowing(estimate)) return null;
    return {
      estimate,
      currency: '$',
      inputTokens: legacyRows.reduce((sum, row) => sum + row.inputTokens, 0),
      outputTokens: legacyRows.reduce((sum, row) => sum + row.outputTokens, 0),
      // The map's rows carry the cache buckets only where their writer set them.
      cacheReadTokens: recordedTokenSum(legacyRows.map((row) => row.cacheReadTokens)),
      cacheCreationTokens: recordedTokenSum(legacyRows.map((row) => row.cacheCreationTokens)),
      models: [],
    };
  }

  if (!pricing) return null;
  const estimate = {
    amount:
      inputTokens * (pricing.input_token_cost ?? 0) +
      outputTokens * (pricing.output_token_cost ?? 0),
    partial: true,
  };
  if (!costIsWorthShowing(estimate)) return null;
  return {
    estimate,
    currency: pricing.currency,
    inputTokens,
    outputTokens,
    // The counters hold input and output alone: nothing to say about the cache.
    cacheReadTokens: null,
    cacheCreationTokens: null,
    models: [],
  };
}

/**
 * A detail of one or two lines (`COST_CARD_COPY.tokens` and `modelDetail`) as ONE muted
 * paragraph, so a second line keeps the detail's own 16px rhythm instead of taking the section's
 * 4px gap and reading as a separate fact. Selected and copied, the lines come out on two lines.
 */
function TokenDetail({ lines }: { lines: string[] }) {
  if (lines.length === 1) return <FooterCardDetail>{lines[0]}</FooterCardDetail>;
  return (
    <FooterCardDetail>
      {lines.map((line, index) => (
        <Fragment key={index}>
          {index > 0 && <br />}
          <span>{line}</span>
        </Fragment>
      ))}
    </FooterCardDetail>
  );
}

/** The trigger's limit mark: `paused` while a limit holds the chat, `warn` at 80%. */
export function limitState(
  usageLimit: SessionUsageLimitController | undefined
): 'paused' | 'warn' | undefined {
  const state = usageLimit?.state;
  if (!state) return undefined;
  if (state.paused) return 'paused';
  return state.warn ? 'warn' : undefined;
}

/** The trigger's accessible name: the cost, or what the item is, then the limit's state. */
export function costTriggerLabel(
  summary: CostSummary | null,
  limit: 'paused' | 'warn' | undefined
): string {
  const base = summary
    ? summary.estimate.amount === null
      ? FOOTER_COPY.costUnavailable
      : FOOTER_COPY.cost(
          formatCostEstimate(summary.estimate, summary.currency),
          summary.estimate.partial
        )
    : USAGE_LIMIT_COPY.triggerLabel;
  if (limit === 'paused') return `${base}, ${USAGE_LIMIT_COPY.triggerPaused}`;
  if (limit === 'warn') return `${base}, ${USAGE_LIMIT_COPY.triggerWarn}`;
  return base;
}

/**
 * The footer's cost: the figure on the line (sans with tabular numbers, `.br-footline__item`) and
 * the card it opens, one of the footer's three `FooterCard`s, which open and close alike
 * (`useFooterCard`): the chat's total and its tokens, then the cost and tokens per model, then
 * the chat's Limit (when `usageLimit` is given).
 *
 * With no cost worth showing (`summary` is `null`: $0, unavailable, a new chat, or Show costs
 * off) and a limit to manage, the trigger is the gauge glyph alone (`chat-usage`), so the Limit
 * section is always one click away. It is the same element at the same place either way, so a
 * card the person kept stays kept when the first cost arrives.
 */
function CostFigure({
  summary,
  usageLimit,
  showCosts,
  priced,
  reloadCost,
  onOpenUsageSettings,
}: {
  summary: CostSummary | null;
  usageLimit?: SessionUsageLimitController;
  showCosts: boolean;
  priced: boolean;
  reloadCost?: () => void;
  onOpenUsageSettings?: () => void;
}) {
  const card = useFooterCard();
  const cardId = useId();
  const byModelId = useId();
  const limit = limitState(usageLimit);
  const label = summary ? formatCostEstimate(summary.estimate, summary.currency) : null;

  // The card reads the limit and the cost afresh each time it opens, together, so the two count
  // the same requests (a title's side call lands after the turn's last frame).
  const reload = usageLimit?.reload;
  useEffect(() => {
    if (!card.open) return;
    reload?.();
    reloadCost?.();
  }, [card.open, reload, reloadCost]);

  // The composer notice's "Change limit" keeps the card open and moves focus into it.
  const openRequest = usageLimit?.openCardRequest ?? 0;
  const handledRequest = useRef(openRequest);
  const { pinned, togglePin } = card;
  useEffect(() => {
    if (openRequest === handledRequest.current) return;
    handledRequest.current = openRequest;
    if (!pinned) togglePin({ moveFocus: true });
  }, [openRequest, pinned, togglePin]);

  return (
    <FooterCardRoot card={card}>
      <FooterCardTrigger
        card={card}
        cardId={cardId}
        data-testid={summary ? 'chat-cost' : 'chat-usage'}
        data-limit-state={limit}
        aria-label={costTriggerLabel(summary, limit)}
      >
        {label ?? <Gauge aria-hidden />}
      </FooterCardTrigger>
      <FooterCard
        card={card}
        id={cardId}
        label={COST_CARD_COPY.cardLabel}
        align="end"
        testId="cost-card"
      >
        {summary && label !== null && (
          // A partial total says so in its label, "Estimated cost of this chat", and why it is an
          // estimate in the InfoTip beside it, rather than in a note under the figures.
          <FooterCardHelpSection
            label={
              summary.estimate.partial ? COST_CARD_COPY.sectionEstimated : COST_CARD_COPY.section
            }
            about={COST_CARD_COPY.aboutEstimate}
            help={summary.estimate.partial ? COST_CARD_COPY.partialNote : undefined}
          >
            <FooterCardValue>{label}</FooterCardValue>
            <TokenDetail lines={COST_CARD_COPY.tokens(summary)} />
          </FooterCardHelpSection>
        )}
        {summary && summary.models.length > 0 && (
          <FooterCardSection label={COST_CARD_COPY.byModel} labelId={byModelId}>
            <FooterCardList labelledBy={byModelId}>
              {summary.models.map((row, index) => (
                <FooterCardRow key={`${modelRowKey(row.provider, row.model)}:${index}`}>
                  <FooterCardValue>{modelLabel(row)}</FooterCardValue>
                  <TokenDetail
                    lines={COST_CARD_COPY.modelDetail(
                      formatTooltipMoney(row.totalCost, summary.currency),
                      row
                    )}
                  />
                </FooterCardRow>
              ))}
            </FooterCardList>
          </FooterCardSection>
        )}
        {usageLimit && (
          <UsageLimitSection
            controller={usageLimit}
            open={card.open}
            showCosts={showCosts}
            priced={priced}
            onOpenUsageSettings={onOpenUsageSettings}
            contentRef={card.contentRef}
          />
        )}
      </FooterCard>
    </FooterCardRoot>
  );
}

export function CostTracker({
  inputTokens = 0,
  outputTokens = 0,
  sessionCosts,
  modelCostRows,
  reloadCost,
  ledgerOnly = false,
  usageLimit,
  onOpenUsageSettings,
}: CostTrackerProps) {
  const { currentModel, currentProvider } = useModelAndProvider();
  const [costInfo, setCostInfo] = useState<PricingData | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [showPricing, setShowPricing] = useState(true);

  useEffect(() => {
    const checkPricingSetting = () => {
      setShowPricing(localStorage.getItem('show_pricing') !== 'false');
    };

    checkPricingSetting();
    window.addEventListener('storage', checkPricingSetting);
    return () => window.removeEventListener('storage', checkPricingSetting);
  }, []);

  useEffect(() => {
    const loadCostInfo = async () => {
      if (!currentModel || !currentProvider) {
        setCostInfo(null);
        setIsLoading(false);
        return;
      }

      setIsLoading(true);
      try {
        const costData = await fetchModelPricing(currentProvider, currentModel);
        setCostInfo(costData);
      } catch {
        setCostInfo(null);
      } finally {
        setIsLoading(false);
      }
    };

    loadCostInfo();
  }, [currentModel, currentProvider]);

  if (!showPricing && !usageLimit) return null;

  const summary = !showPricing
    ? null
    : costSummary({
        modelCostRows,
        sessionCosts,
        inputTokens,
        outputTokens,
        // Loading, no model to price the counters at, or a chat whose counters are not the current
        // model's (`ledgerOnly`): nothing from this path.
        pricing: !ledgerOnly && currentModel && currentProvider && !isLoading ? costInfo : null,
      });
  // A new limit starts in dollars when something the chat runs on has a price, whether or not
  // Show costs is on: that switch hides the running cost, not the dollar limit.
  const priced =
    (!!costInfo &&
      ((costInfo.input_token_cost ?? 0) > 0 || (costInfo.output_token_cost ?? 0) > 0)) ||
    (modelCostRows ?? []).some((row) => row.totalCost !== null && row.totalCost > 0);
  // One element at one place for every path, so a card the person pinned stays pinned when the
  // ledger's answer replaces the live price.
  if (!summary && !usageLimit) return null;
  return (
    <CostFigure
      summary={summary}
      usageLimit={usageLimit}
      showCosts={showPricing}
      priced={priced}
      reloadCost={reloadCost}
      onOpenUsageSettings={onOpenUsageSettings}
    />
  );
}
