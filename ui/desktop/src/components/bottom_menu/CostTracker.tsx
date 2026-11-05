import { useEffect, useId, useState } from 'react';
import { useModelAndProvider } from '../ModelAndProviderContext';
import { COST_CARD_COPY, FOOTER_COPY } from './copy';
import './pickers.css';
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
  FooterCardNote,
  FooterCardRoot,
  FooterCardRow,
  FooterCardSection,
  FooterCardTrigger,
  FooterCardValue,
} from './FooterCard';
import { useFooterCard } from './useFooterCard';

interface CostTrackerProps {
  inputTokens?: number;
  outputTokens?: number;
  sessionCosts?: SessionCosts;
  modelCostRows?: ModelCostRow[];
}

export interface CostEstimate {
  amount: number | null;
  partial: boolean;
}

/**
 * The total of priced rows: the ledger's per-model rows, or the legacy per-model map's values. A
 * row with no price makes the total a conservative subtotal; no priced row at all leaves it
 * unavailable rather than $0.
 */
export function aggregateModelRowsCost(
  rows: Pick<SessionCostRow, 'totalCost' | 'costIsPartial'>[]
): CostEstimate {
  let amount = 0;
  let hasKnownCost = false;
  let partial = false;
  for (const row of rows) {
    if (row.totalCost === null) {
      partial = true;
    } else {
      amount += row.totalCost;
      hasKnownCost = true;
    }
    partial ||= row.costIsPartial ?? false;
  }
  return { amount: hasKnownCost ? amount : null, partial };
}

export function formatTooltipMoney(amount: number | null, currency = '$'): string {
  if (amount === null || !Number.isFinite(amount) || amount < 0) return 'Unavailable';
  if (amount > 0 && amount < 0.01) return `<${currency}0.01`;
  return `${currency}${amount.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
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
 */
export interface CostSummary {
  estimate: CostEstimate;
  currency: string;
  inputTokens: number;
  outputTokens: number;
  /** The ledger's rows, one per model, for the card's By model section; empty on the other paths. */
  models: ModelCostRow[];
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
  return { estimate, currency: pricing.currency, inputTokens, outputTokens, models: [] };
}

/**
 * The footer's cost: the figure on the line (sans with tabular numbers, `.br-footline__item`) and
 * the card it opens, one of the footer's three `FooterCard`s, which open and close alike
 * (`useFooterCard`): the chat's total and its tokens, then the cost per model.
 */
function CostFigure({ summary }: { summary: CostSummary }) {
  const card = useFooterCard();
  const cardId = useId();
  const byModelId = useId();
  const { estimate, currency, models } = summary;
  const label = formatCostEstimate(estimate, currency);
  return (
    <FooterCardRoot card={card}>
      <FooterCardTrigger
        card={card}
        cardId={cardId}
        data-testid="chat-cost"
        aria-label={
          estimate.amount === null
            ? FOOTER_COPY.costUnavailable
            : FOOTER_COPY.cost(label, estimate.partial)
        }
      >
        {label}
      </FooterCardTrigger>
      <FooterCard
        card={card}
        id={cardId}
        label={COST_CARD_COPY.cardLabel}
        align="end"
        testId="cost-card"
      >
        <FooterCardSection label={COST_CARD_COPY.section}>
          <FooterCardValue>{label}</FooterCardValue>
          <FooterCardDetail>
            {COST_CARD_COPY.tokens(summary.inputTokens, summary.outputTokens)}
          </FooterCardDetail>
          {estimate.partial && <FooterCardNote>{COST_CARD_COPY.partialNote}</FooterCardNote>}
        </FooterCardSection>
        {models.length > 0 && (
          <FooterCardSection label={COST_CARD_COPY.byModel} labelId={byModelId}>
            <FooterCardList labelledBy={byModelId}>
              {models.map((row, index) => (
                <FooterCardRow key={`${modelRowKey(row.provider, row.model)}:${index}`}>
                  <FooterCardValue>{modelLabel(row)}</FooterCardValue>
                  <FooterCardDetail>
                    {COST_CARD_COPY.modelDetail(
                      formatTooltipMoney(row.totalCost, currency),
                      row.inputTokens,
                      row.outputTokens
                    )}
                  </FooterCardDetail>
                </FooterCardRow>
              ))}
            </FooterCardList>
          </FooterCardSection>
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

  if (!showPricing) return null;

  const summary = costSummary({
    modelCostRows,
    sessionCosts,
    inputTokens,
    outputTokens,
    // Loading, or no model to price the counters at: nothing from this path.
    pricing: currentModel && currentProvider && !isLoading ? costInfo : null,
  });
  // One element at one place for every path, so a card the person pinned stays pinned when the
  // ledger's answer replaces the live price.
  return summary ? <CostFigure summary={summary} /> : null;
}
