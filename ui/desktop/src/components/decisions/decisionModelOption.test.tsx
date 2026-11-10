import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import {
  DECISION_MODEL_OPTION_COPY,
  decisionModelKey,
  decisionModelOptions,
  decisionModelReason,
  formatDecisionModelOption,
  formatWindow,
  isDecisionModelOptionDisabled,
  riskyWindow,
} from './decisionModelOption';
import type { DecisionModelEntry } from '../../utils/sessionDecisions';

function entry(overrides: Partial<DecisionModelEntry> = {}): DecisionModelEntry {
  return {
    provider: 'ollama_decision',
    model: 'tev1:0.8b',
    displayName: 'tev1:0.8b',
    contextTokens: 2048,
    kinds: ['binary'],
    tier: 'private',
    adapter: null,
    local: true,
    unavailableReason: null,
    ...overrides,
  };
}

const local = entry();
const largeLocal = entry({ model: 'clef-flash:latest', contextTokens: 32_768 });
const hosted = entry({
  provider: 'baseten_decision',
  model: 'decision-models/joey-1',
  displayName: 'decision-models/joey-1',
  contextTokens: 16_000,
  tier: 'public',
  local: false,
});

const byName = (options: ReturnType<typeof decisionModelOptions>, name: string) =>
  options.find((option) => option.name === name)!;

describe('formatWindow', () => {
  it('writes tokens and thousands', () => {
    expect(formatWindow(512)).toBe('512 tokens');
    expect(formatWindow(2048)).toBe('2K tokens');
    expect(formatWindow(32_768)).toBe('33K tokens');
    expect(formatWindow(null)).toBeNull();
    expect(formatWindow(0)).toBeNull();
  });
  it('names the window in a row only under 8K', () => {
    expect(riskyWindow(2048)).toBe('2K tokens');
    expect(riskyWindow(7_999)).toBe('8K tokens');
    expect(riskyWindow(8_000)).toBeNull();
    expect(riskyWindow(null)).toBeNull();
  });
});

describe('decisionModelOptions (monitors spec 1.5)', () => {
  const options = decisionModelOptions({
    models: [local, largeLocal, hosted],
    chatTier: 'public',
    browser: false,
  });

  it('puts the chat default first, named by the hint without a stored model', () => {
    expect(options[0]).toMatchObject({
      value: '',
      name: 'Default model',
      secondary: "the chat's decision model",
      model: null,
      reason: null,
    });
  });

  it('names the stored default model by its alias', () => {
    const [first] = decisionModelOptions({
      models: [],
      chatTier: null,
      defaultModel: { provider: 'baseten_decision', model: 'decision-models/joey-1' },
      browser: false,
    });
    expect(first.secondary).not.toMatch(/decision-models\//);
    expect(first.secondary).toMatch(/joey-1/);
  });

  it('says where a local model runs, with the lock and the small window', () => {
    expect(byName(options, 'tev1:0.8b')).toMatchObject({
      value: 'ollama_decision/tev1:0.8b',
      secondary: 'this computer · 2K tokens',
      isPrivate: true,
      label: 'tev1:0.8b, Private model, this computer · 2K tokens',
      reason: null,
    });
    expect(byName(options, 'clef-flash:latest').secondary).toBe('this computer');
  });

  it('names a hosted model by its alias and Baseten, with no lock and no window', () => {
    const row = options.find((option) => option.model?.provider === 'baseten_decision')!;
    expect(row.name).not.toMatch(/decision-models\//);
    expect(row.secondary).toBe('Baseten');
    expect(row.isPrivate).toBe(false);
  });

  it('never prints a provider key, a slug or the words Private and Public', () => {
    for (const option of options) {
      const text = `${option.name} ${option.secondary ?? ''}`;
      expect(text).not.toMatch(/ollama_decision|baseten_decision|decision-models\//);
      expect(text).not.toMatch(/\b(Private|Public)\b/);
    }
  });

  it('says the computer Biorouter runs on in a serve browser', () => {
    const [, row] = decisionModelOptions({ models: [local], chatTier: null, browser: true });
    expect(row.secondary).toBe('the computer Biorouter runs on · 2K tokens');
  });

  it('refuses a public model in a private chat with the R4 line, overriding the daemon', () => {
    const rows = decisionModelOptions({
      models: [
        {
          ...hosted,
          unavailableReason:
            'Model `joey-1` is public and cannot read a private chat (decision_models lists them).',
        },
      ],
      chatTier: 'private',
      browser: false,
    });
    expect(rows[1].reason).toBe(DECISION_MODEL_OPTION_COPY.publicRefused);
    expect(rows[1].secondary).toBe("Public model: it can't read a private chat.");
    expect(isDecisionModelOptionDisabled(rows[1])).toBe(true);
  });

  it('shows any other reason from the daemon without backticks, and drops the lock', () => {
    const rows = decisionModelOptions({
      models: [{ ...local, unavailableReason: 'Grant `crew` does not allow `tev1:0.8b`.' }],
      chatTier: 'private',
      browser: false,
    });
    expect(rows[1]).toMatchObject({
      reason: 'Grant crew does not allow tev1:0.8b.',
      secondary: 'Grant crew does not allow tev1:0.8b.',
      isPrivate: false,
    });
  });

  it('disables the default row with the chat-wide refusal', () => {
    const [first] = decisionModelOptions({
      models: [],
      chatTier: 'private',
      chatRefusal: 'This chat works under a Crew grant that no longer stands.',
      browser: false,
    });
    expect(isDecisionModelOptionDisabled(first)).toBe(true);
    expect(first.secondary).toBe('This chat works under a Crew grant that no longer stands.');
  });

  it('lets a usable model through', () => {
    expect(decisionModelReason(local, 'private')).toBeNull();
    expect(decisionModelReason(hosted, 'public')).toBeNull();
    expect(isDecisionModelOptionDisabled(byName(options, 'tev1:0.8b'))).toBe(false);
  });

  it('keys models by provider and model', () => {
    expect(decisionModelKey(null)).toBe('');
    expect(decisionModelKey({ provider: 'p', model: 'm' })).toBe('p/m');
  });
});

describe('formatDecisionModelOption', () => {
  const [, row] = decisionModelOptions({ models: [local], chatTier: null, browser: false });
  const [, hostedRow] = decisionModelOptions({ models: [hosted], chatTier: null, browser: false });

  it('renders one line: the name, a hidden Private model after an aria-hidden lock, the place', () => {
    const { container } = render(<>{formatDecisionModelOption(row, { context: 'menu' })}</>);
    const option = screen.getByTestId('decision-model-option');
    expect(option.textContent).toBe('tev1:0.8b, Private model, this computer · 2K tokens');
    expect(option.className).toContain('flex-wrap');
    expect(option.className).toContain('gap-x-2');
    const svg = container.querySelector('svg')!;
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(screen.getByText('Private model,', { exact: false }).className).toBe('sr-only');
  });

  it('draws no lock for a public model', () => {
    const { container } = render(<>{formatDecisionModelOption(hostedRow, { context: 'menu' })}</>);
    expect(container.querySelector('svg')).toBeNull();
    expect(screen.getByTestId('decision-model-option').textContent).toMatch(/, Baseten$/);
  });

  it('shows the name alone as the selected value', () => {
    const { container } = render(<>{formatDecisionModelOption(row, { context: 'value' })}</>);
    expect(container.textContent).toBe('tev1:0.8b');
    expect(container.querySelector('svg')).toBeNull();
  });
});
