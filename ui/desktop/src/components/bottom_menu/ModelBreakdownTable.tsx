import type { ModelCostRow } from '../../hooks/useCostTracking';
import { modelRowKey } from '../../hooks/useCostTracking';
import { billedTokens, knownBilledTokens } from '../../utils/usageAccounting';
import { decisionModelName } from '../../utils/usageStats';
import { isDecisionProvider, pluralize } from '../decisions/decisionWire';
import { MODEL_NOT_RECORDED } from './copy';
import { distinctModelLabels, modelDisplayName, type ModelRowName } from './modelLabel';
import { formatTooltipMoney, isUnmeteredDecisionRow } from './CostTracker';

interface ModelBreakdownTableProps {
  rows: ModelCostRow[];
  currency?: string;
}

/**
 * Label a row's model for a person: its display name ("Claude Haiku 5.5", the name the composer's
 * chip and every other surface show; T1 F2), or `MODEL_NOT_RECORDED` for the bucket of turns saved
 * without a model. A label is copy, never a key: key a row by `modelRowKey`. A decision model's
 * row is named as one (`Decision model: joey-1`), since its Baseten slug says little. In a list,
 * label the rows with {@link modelLabels}, which tells apart rows that would read the same.
 */
export function modelLabel(row: Pick<ModelCostRow, 'provider' | 'model'>): string {
  return rowName(row).name;
}

function rowName(row: Pick<ModelCostRow, 'provider' | 'model'>): ModelRowName {
  if (!row.model) {
    return { name: MODEL_NOT_RECORDED, provider: row.provider };
  }
  if (isDecisionProvider(row.provider)) {
    // Its wire provider (`baseten_decision`) is no word for a person; its slug tells two apart.
    return { name: `Decision model: ${decisionModelName(row.model)}`, id: row.model };
  }
  return { name: modelDisplayName(row.model), provider: row.provider, id: row.model };
}

/**
 * Each row's label, told apart from every other row's (`distinctModelLabels`): the provider
 * follows a name two rows share, and the raw id follows when one provider served both, as two
 * snapshots of one model (`gpt-5.5-2026-04-24`, `gpt-5.5-2026-07-09`) or two Bedrock profiles
 * (`us.` and `global.`), each priced on its own.
 */
export function modelLabels(rows: readonly Pick<ModelCostRow, 'provider' | 'model'>[]): string[] {
  return distinctModelLabels(rows.map(rowName));
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
  const labels = modelLabels(rows);

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
            <td className="pr-3 [overflow-wrap:anywhere]">{labels[index]}</td>
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
