import { useEffect, useState } from 'react';
import { useModelAndProvider } from '../ModelAndProviderContext';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { FOOTER_COPY } from './copy';
import './pickers.css';
import { fetchModelPricing } from '../../utils/pricing';
import { PricingData } from '../../api';
import type { ModelCostRow, SessionCostRow, SessionCosts } from '../../hooks/useCostTracking';

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

export function aggregateModelRowsCost(rows: ModelCostRow[]): CostEstimate {
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

function aggregateSessionCosts(rows: SessionCostRow[]): CostEstimate {
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

export function sessionTokensSummary(inputTokens: number, outputTokens: number): string {
  return `Input: ${inputTokens.toLocaleString()} tokens\nOutput: ${outputTokens.toLocaleString()} tokens`;
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
 * "$0.00" and "Unavailable" are not worth a place on the line; the breakdown
 * is still one hover away once there is something to break down.
 */
export function costIsWorthShowing(estimate: CostEstimate): boolean {
  return estimate.amount !== null && Number.isFinite(estimate.amount) && estimate.amount > 0;
}

/** The footer's figure: sans with tabular numbers (`.br-footline__item`). */
function CostTrigger({ estimate, currency = '$' }: { estimate: CostEstimate; currency?: string }) {
  const label = formatCostEstimate(estimate, currency);
  return (
    <TooltipTrigger asChild>
      <button
        type="button"
        data-testid="chat-cost"
        className="br-footline__item"
        aria-label={
          estimate.amount === null
            ? FOOTER_COPY.costUnavailable
            : FOOTER_COPY.cost(label, estimate.partial)
        }
      >
        {label}
      </button>
    </TooltipTrigger>
  );
}

export function costEstimateSummary(estimate: CostEstimate, currency = '$') {
  if (estimate.amount === null) return 'Total cost unavailable';
  const label = formatCostEstimate(estimate, currency);
  return estimate.partial
    ? `Estimated total: ${label}\nConservative estimate based on available token and pricing data.`
    : `Total cost: ${label}`;
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

  if (modelCostRows && modelCostRows.length > 0) {
    const estimate = aggregateModelRowsCost(modelCostRows);
    if (!costIsWorthShowing(estimate)) return null;
    const inputTotal = modelCostRows.reduce((sum, row) => sum + row.inputTokens, 0);
    const outputTotal = modelCostRows.reduce((sum, row) => sum + row.outputTokens, 0);
    return (
      <Tooltip>
        <CostTrigger estimate={estimate} />
        <TooltipContent className="whitespace-pre-line">
          {`${sessionTokensSummary(inputTotal, outputTotal)}\n${costEstimateSummary(estimate)}`}
        </TooltipContent>
      </Tooltip>
    );
  }

  const legacyRows = sessionCosts ? Object.values(sessionCosts) : [];
  if (legacyRows.length > 0) {
    const estimate = aggregateSessionCosts(legacyRows);
    if (!costIsWorthShowing(estimate)) return null;
    const totals = legacyRows.reduce(
      (sum, row) => ({
        input: sum.input + row.inputTokens,
        output: sum.output + row.outputTokens,
      }),
      { input: 0, output: 0 }
    );
    return (
      <Tooltip>
        <CostTrigger estimate={estimate} />
        <TooltipContent className="whitespace-pre-line">
          {`${sessionTokensSummary(totals.input, totals.output)}\n${costEstimateSummary(estimate)}`}
        </TooltipContent>
      </Tooltip>
    );
  }

  if (!currentModel || !currentProvider) return null;

  // Loading, or no price for this model: nothing to show on the line.
  if (isLoading || !costInfo) return null;

  const freshSubtotal =
    inputTokens * (costInfo.input_token_cost ?? 0) +
    outputTokens * (costInfo.output_token_cost ?? 0);
  const estimate = { amount: freshSubtotal, partial: true };
  if (!costIsWorthShowing(estimate)) return null;
  return (
    <Tooltip>
      <CostTrigger estimate={estimate} currency={costInfo.currency} />
      <TooltipContent className="whitespace-pre-line">
        {`${sessionTokensSummary(inputTokens, outputTokens)}\n${costEstimateSummary(estimate, costInfo.currency)}`}
      </TooltipContent>
    </Tooltip>
  );
}
