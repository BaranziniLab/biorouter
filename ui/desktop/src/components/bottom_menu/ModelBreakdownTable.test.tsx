import { describe, it, expect } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import { ModelBreakdownTable, modelLabel } from './ModelBreakdownTable';
import { MODEL_NOT_RECORDED } from './copy';
import { homeUsageCopy } from '../usage/copy';
import { UNKNOWN_MODEL_LABEL, type ModelCostRow } from '../../hooks/useCostTracking';

// A fixed payload mirroring the backend rollup: two real models the user
// switched between mid-thread, plus the null-model bucket (turns saved without
// a model), whose price is unknown.
const rows: ModelCostRow[] = [
  {
    provider: 'openai',
    model: 'gpt-5',
    inputTokens: 300,
    outputTokens: 70,
    cacheReadTokens: 500,
    cacheCreationTokens: 100,
    totalTokens: 970,
    turns: 2,
    totalCost: 1.23,
    costIsPartial: true,
  },
  {
    provider: 'anthropic',
    model: 'claude-fable-5',
    inputTokens: 10,
    outputTokens: 5,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalTokens: 15,
    turns: 1,
    totalCost: 0.0004,
    costIsPartial: false,
  },
  {
    provider: undefined,
    model: undefined,
    inputTokens: 1,
    outputTokens: 2,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    totalTokens: 3,
    turns: 1,
    totalCost: null,
    costIsPartial: false,
  },
];

describe('modelLabel', () => {
  it('joins provider and model, and names the null bucket in sentence case', () => {
    expect(modelLabel({ provider: 'openai', model: 'gpt-5' })).toBe('openai/gpt-5');
    expect(modelLabel({ provider: undefined, model: 'gpt-5' })).toBe('gpt-5');
    // Home's usage card's name for the bucket, not a second one (usage/copy.test.ts).
    expect(MODEL_NOT_RECORDED).toBe(homeUsageCopy.notRecorded);
    expect(modelLabel({ provider: undefined, model: undefined })).toBe(MODEL_NOT_RECORDED);
    // A provider that reported no model is still the same bucket, and says so.
    expect(modelLabel({ provider: 'openai', model: undefined })).toBe(MODEL_NOT_RECORDED);
    // The key token is never what a person reads.
    expect(modelLabel({ provider: undefined, model: undefined })).not.toBe(UNKNOWN_MODEL_LABEL);
  });

  it('names a decision model as one, by its alias', () => {
    expect(
      modelLabel({
        provider: 'baseten_decision',
        model: 'decision-models/baseten-joey-2b-latest',
      })
    ).toBe('Decision model: joey-1');
    expect(modelLabel({ provider: 'ollama_decision', model: 'tev1:0.8b' })).toBe(
      'Decision model: tev1:0.8b'
    );
  });
});

describe('ModelBreakdownTable decision rows', () => {
  it('counts a decision model in requests, not turns', () => {
    render(
      <ModelBreakdownTable
        rows={[
          {
            provider: 'baseten_decision',
            model: 'decision-models/baseten-mammoth-27b-latest',
            inputTokens: 1_200_000,
            outputTokens: 600,
            cacheReadTokens: 0,
            cacheCreationTokens: 0,
            totalTokens: 1_200_600,
            turns: 600,
            totalCost: 0.12,
            costIsPartial: false,
          },
        ]}
      />
    );
    const row = screen.getByText('Decision model: mammoth-1').closest('tr')!;
    const cells = within(row).getAllByRole('cell');
    expect(cells[1].textContent).toBe('600 requests');
    expect(cells[7].textContent).toBe('$0.12');
  });

  it('says a local decision model is not metered, and an unpriced hosted one is unavailable', () => {
    const local: ModelCostRow = {
      provider: 'ollama_decision',
      model: 'tev1:0.8b',
      inputTokens: 40_000,
      outputTokens: 20,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      totalTokens: 40_020,
      turns: 20,
      totalCost: null,
      costIsPartial: false,
    };
    const hosted: ModelCostRow = {
      ...local,
      provider: 'baseten_decision',
      model: 'decision-models/baseten-unlisted-99b-latest',
    };
    render(<ModelBreakdownTable rows={[local, hosted]} />);
    const localCells = within(
      screen.getByText('Decision model: tev1:0.8b').closest('tr')!
    ).getAllByRole('cell');
    expect(localCells[7].textContent).toBe('Not metered');
    const hostedRow = screen.getByText('Decision model: baseten-unlisted-99b-latest');
    expect(within(hostedRow.closest('tr')!).getAllByRole('cell')[7].textContent).toBe(
      'Unavailable'
    );
  });
});

describe('ModelBreakdownTable', () => {
  it('renders one row per model with hand-checked token and cost cells', () => {
    render(<ModelBreakdownTable rows={rows} />);

    const table = screen.getByTestId('model-breakdown-table');
    const bodyRows = within(table).getAllByRole('row');
    // 1 header + 3 data rows.
    expect(bodyRows).toHaveLength(4);

    // gpt-5 row: every token bucket, cache-aware billed total, partial cost.
    const gptRow = screen.getByText('openai/gpt-5').closest('tr')!;
    const gptCells = within(gptRow).getAllByRole('cell');
    expect(gptCells.map((c) => c.textContent)).toEqual([
      'openai/gpt-5',
      '2',
      '300',
      '500',
      '100',
      '70',
      '970',
      '$1.23',
    ]);

    // A sub-cent cost keeps its digits, as the chat's total does (utils/formatUsd.ts).
    const claudeRow = screen.getByText('anthropic/claude-fable-5').closest('tr')!;
    expect(within(claudeRow).getAllByRole('cell')[7].textContent).toBe('$0.0004');

    // The bucket with no model: named in sentence case, null price explicitly unavailable.
    expect(screen.queryByText(UNKNOWN_MODEL_LABEL)).not.toBeInTheDocument();
    const unknownRow = screen.getByText(MODEL_NOT_RECORDED).closest('tr')!;
    const unknownCells = within(unknownRow).getAllByRole('cell');
    expect(unknownCells.map((c) => c.textContent)).toEqual([
      MODEL_NOT_RECORDED,
      '1',
      '1',
      '0',
      '0',
      '2',
      '3',
      'Unavailable',
    ]);
  });

  it('renders nothing for an empty payload', () => {
    const { container } = render(<ModelBreakdownTable rows={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it('formats large token counts with thousands separators', () => {
    render(
      <ModelBreakdownTable
        rows={[
          {
            provider: 'versa',
            model: 'gpt-5',
            inputTokens: 12_000_000,
            outputTokens: 1_300_000,
            cacheReadTokens: 0,
            cacheCreationTokens: 0,
            totalTokens: 13_300_000,
            turns: 61,
            totalCost: 42,
            costIsPartial: false,
          },
        ]}
      />
    );
    const row = screen.getByText('versa/gpt-5').closest('tr')!;
    const cells = within(row).getAllByRole('cell');
    expect(cells[2].textContent).toBe('12,000,000');
    expect(cells[5].textContent).toBe('1,300,000');
    expect(cells[6].textContent).toBe('13,300,000');
  });

  it('shows nullable cache buckets as unrecorded and measured buckets as conservative totals', () => {
    render(
      <ModelBreakdownTable
        rows={[
          {
            provider: 'anthropic',
            model: 'legacy-model',
            inputTokens: 100,
            outputTokens: 20,
            cacheReadTokens: null,
            cacheCreationTokens: 5,
            totalTokens: null,
            turns: 2,
            totalCost: 1.25,
            costIsPartial: true,
          },
        ]}
      />
    );

    const row = screen.getByText('anthropic/legacy-model').closest('tr')!;
    expect(
      within(row)
        .getAllByRole('cell')
        .map((cell) => cell.textContent)
    ).toEqual(['anthropic/legacy-model', '2', '100', 'Not recorded', '5', '20', '125', '$1.25']);
  });
});
