import type { ModelCostRow } from '../../hooks/useCostTracking';
import { modelRowKey } from '../../hooks/useCostTracking';
import { billedTokens, knownBilledTokens } from '../../utils/usageAccounting';
import { decisionModelName } from '../../utils/usageStats';
import { isDecisionProvider, pluralize } from '../decisions/decisionWire';
import { MODEL_NOT_RECORDED } from './copy';
import { modelDisplayName } from './modelLabel';
import { formatTooltipMoney, isUnmeteredDecisionRow } from './CostTracker';

interface ModelBreakdownTableProps {
  rows: ModelCostRow[];
  currency?: string;
}

/**
 * Label a row's model for a person: its display name ("Claude Haiku 5.5", the name the composer's
 * chip and every other surface show; T1 F2), or `MODEL_NOT_RECORDED` for the bucket of turns saved
 * without a model. A label is copy, never a key: key a row by `modelRowKey`. A decision model's
 * row is named as one (`Decision model: joey-1`), since its Baseten slug says little. Pass
 * `withProvider` when two rows would otherwise read the same (one model through two providers).
 */
export function modelLabel(
  row: Pick<ModelCostRow, 'provider' | 'model'>,
  withProvider = false
): string {
  if (!row.model) {
    return MODEL_NOT_RECORDED;
  }
  if (isDecisionProvider(row.provider)) {
    return `Decision model: ${decisionModelName(row.model)}`;
  }
  const name = modelDisplayName(row.model);
  return withProvider && row.provider ? `${name} · ${row.provider}` : name;
}

/** Whether each row's label needs its provider to tell it from another row's. */
export function labelsNeedProvider(rows: Pick<ModelCostRow, 'provider' | 'model'>[]): boolean[] {
  const counts = new Map<string, number>();
  const names = rows.map((row) => modelLabel(row));
  for (const name of names) counts.set(name, (counts.get(name) ?? 0) + 1);
  return names.map((name) => (counts.get(name) ?? 0) > 1);
}

/**
 * The Turns cell. A decision model's count is of requests (a monitor check, a
 * chunk of a `decide` batch), which are not turns, so it says so.
 */
function turnsCell(row: Pick<ModelCostRow, 'provider' | 'turns'>): string {
  return isDecisionProvider(row.provider)
    ? pluralize(row.turns, 'request')
    : row.turns.toLocaleString();
}

/**
 * The Cost cell. A local decision model has no meter, so its row reads "Not
 * metered" (the Usage panel's word) rather than "Unavailable".
 */
function costCell(row: Pick<ModelCostRow, 'provider' | 'totalCost'>, currency: string): string {
  return isUnmeteredDecisionRow(row) ? 'Not metered' : formatTooltipMoney(row.totalCost, currency);
}

function tokenCell(tokens: number | null): string {
  return tokens === null ? 'Not recorded' : tokens.toLocaleString();
}

function billedTokenCell(row: ModelCostRow): string {
  const exact = billedTokens(row);
  if (exact !== null) return tokenCell(exact);
  const subtotal = knownBilledTokens(row);
  return subtotal > 0 ? tokenCell(subtotal) : 'Unavailable';
}

/**
 * Per-model usage breakdown for the cost popover, rendered from the real
 * `token_events` rollup (Issue #1). Rows arrive already priced; a `null` cost
 * is explicitly unavailable so an unknown price never masquerades as free.
 */
export function ModelBreakdownTable({ rows, currency = '$' }: ModelBreakdownTableProps) {
  if (rows.length === 0) {
    return null;
  }
  const withProvider = labelsNeedProvider(rows);

  return (
    <table
      className="w-full border-collapse text-left text-supporting"
      data-testid="model-breakdown-table"
    >
      <thead>
        <tr className="h-8 border-b border-border-subtle text-text-muted">
          <th className="pr-3 font-medium">Model</th>
          <th className="px-2 text-right font-medium">Turns</th>
          <th className="px-2 text-right font-medium">Fresh in</th>
          <th className="px-2 text-right font-medium">Cache read</th>
          <th className="px-2 text-right font-medium">Cache write</th>
          <th className="px-2 text-right font-medium">Out</th>
          <th className="px-2 text-right font-medium">Billed</th>
          <th className="pl-2 text-right font-medium">Cost</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row, index) => (
          <tr
            key={modelRowKey(row.provider, row.model)}
            className="h-10 border-b border-border-subtle text-text-default last:border-b-0"
          >
            <td className="pr-3">{modelLabel(row, withProvider[index])}</td>
            <td className="px-2 text-right tabular-nums">{turnsCell(row)}</td>
            <td className="px-2 text-right tabular-nums">{row.inputTokens.toLocaleString()}</td>
            <td className="px-2 text-right tabular-nums">{tokenCell(row.cacheReadTokens)}</td>
            <td className="px-2 text-right tabular-nums">{tokenCell(row.cacheCreationTokens)}</td>
            <td className="px-2 text-right tabular-nums">{row.outputTokens.toLocaleString()}</td>
            <td className="px-2 text-right tabular-nums">{billedTokenCell(row)}</td>
            <td className="pl-2 text-right tabular-nums">{costCell(row, currency)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
