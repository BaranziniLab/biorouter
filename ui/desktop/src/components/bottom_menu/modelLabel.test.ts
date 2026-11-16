import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  distinctModelLabels,
  distinguishModels,
  friendlyModelName,
  modelDisplayName,
  providerDisplayName,
  rememberProviderNames,
} from './modelLabel';
import displayNameTable from './modelDisplayName.cases.json';

describe('friendlyModelName', () => {
  it.each([
    ['gpt-5.6-sol-2026-07-09', 'gpt-5.6-sol'],
    ['gpt-5.5-2026-04-24', 'gpt-5.5'],
    ['claude-3-5-sonnet-20241022', 'claude-3-5-sonnet'],
    ['claude-opus-5', 'claude-opus-5'],
    ['gemma4-12b', 'gemma4-12b'],
    ['owner/repo:Q4_K_M', 'owner/repo:Q4_K_M'],
    ['gpt-4-0613', 'gpt-4-0613'],
    ['2026-07-09', '2026-07-09'],
    ['', ''],
  ])('%s -> %s', (id, name) => {
    expect(friendlyModelName(id)).toBe(name);
  });

  // V2-R6-2: a Bedrock inference-profile id is not a name. Region, vendor and
  // API version come off; the date strip then sees a trailing date again.
  it.each([
    ['us.anthropic.claude-sonnet-4-6', 'claude-sonnet-4-6'],
    ['anthropic.claude-3-5-sonnet-20240620-v1:0', 'claude-3-5-sonnet'],
    ['us.anthropic.claude-sonnet-4-5-20250929-v1:0', 'claude-sonnet-4-5'],
    ['us.anthropic.claude-opus-4-6-v1', 'claude-opus-4-6'],
    ['eu.anthropic.claude-haiku-4-5-20251001-v1:0', 'claude-haiku-4-5'],
    // The Claude 5.5 generation Versa API Bedrock offers (2026-11-12).
    ['us.anthropic.claude-opus-5-5', 'claude-opus-5-5'],
    ['us.anthropic.claude-sonnet-5-5', 'claude-sonnet-5-5'],
    ['us.anthropic.claude-haiku-5-5', 'claude-haiku-5-5'],
    ['apac.amazon.nova-pro-v1:0', 'nova-pro'],
    ['global.anthropic.claude-opus-5', 'claude-opus-5'],
    ['us-gov.anthropic.claude-sonnet-5', 'claude-sonnet-5'],
    ['meta.llama3-1-70b-instruct-v1:0', 'llama3-1-70b-instruct'],
    ['mistral.mistral-large-2407-v1:0', 'mistral-large-2407'],
    ['openai.gpt-oss-120b-1:0', 'gpt-oss-120b'],
    // A one-word name keeps its vendor rather than standing alone.
    ['us.deepseek.r1-v1:0', 'deepseek-r1'],
    ['  us.anthropic.claude-sonnet-4-6  ', 'claude-sonnet-4-6'],
  ])('bedrock %s -> %s', (id, name) => {
    expect(friendlyModelName(id)).toBe(name);
  });

  // Only Bedrock-shaped ids lose a vendor or a version: elsewhere a dot is
  // part of the name and `-v3` is a generation.
  it.each([
    ['qwen2.5-coder:7b', 'qwen2.5-coder:7b'],
    ['llama3.1:8b', 'llama3.1:8b'],
    ['deepseek-v3', 'deepseek-v3'],
    ['deepseek-v3.1', 'deepseek-v3.1'],
    ['gpt-4.1', 'gpt-4.1'],
    ['meta-llama/Llama-3.1-8B-Instruct', 'meta-llama/Llama-3.1-8B-Instruct'],
    ['claude-sonnet-4-6', 'claude-sonnet-4-6'],
  ])('not bedrock %s -> %s', (id, name) => {
    expect(friendlyModelName(id)).toBe(name);
  });
});

/**
 * One model, one name (Phase C, T1 F2). The same Versa API Bedrock model read
 * `us.anthropic.claude-haiku-5-5` in Settings and the model picker, `claude-haiku-5-5` on the
 * composer chip and "Claude Haiku 5.5" in the provider documentation. Every spelling of one model
 * now comes to the same name, whichever provider serves it.
 */
describe('modelDisplayName', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  // Every case of the table the daemon's `model_display_name` reads too
  // (`crates/biorouter/src/providers/model_display.rs`), so the two never name one model two ways.
  describe('modelDisplayName matches the shared case table', () => {
    const cases = displayNameTable.groups.flatMap((group) => group.cases);

    it('holds cases', () => {
      expect(cases.length).toBeGreaterThan(80);
      for (const entry of cases) expect(entry).toHaveLength(2);
    });

    it.each(cases)('%j -> %j', (id, name) => {
      expect(modelDisplayName(id)).toBe(name);
    });
  });

  it('leaves a name as it is, so asking twice changes nothing', () => {
    for (const id of ['us.anthropic.claude-haiku-5-5', 'gpt-5.6-sol-2026-07-09', 'qwen3.8:27b']) {
      const name = modelDisplayName(id);
      expect(modelDisplayName(name)).toBe(name);
    }
    expect(modelDisplayName('Lab Haiku / fast')).toBe('Lab Haiku / fast');
  });

  it("a deployment's alias wins", () => {
    vi.stubGlobal('window', {
      ...window,
      appConfig: {
        get: (key: string) =>
          key === 'BIOROUTER_PREDEFINED_MODELS'
            ? JSON.stringify([{ name: 'us.anthropic.claude-haiku-5-5', alias: 'Lab Haiku' }])
            : undefined,
      },
    });
    expect(modelDisplayName('us.anthropic.claude-haiku-5-5')).toBe('Lab Haiku');
  });
});

describe('distinctModelLabels', () => {
  const row = (provider: string | null, id: string) => ({
    name: modelDisplayName(id),
    provider,
    id,
  });

  it('names each model alone where no other row reads the same', () => {
    expect(
      distinctModelLabels([row('openai', 'gpt-5.5-2026-04-24'), row('anthropic', 'claude-fable-5')])
    ).toEqual(['GPT-5.5', 'Claude Fable 5']);
  });

  it('adds the provider where one model came through two providers', () => {
    expect(
      distinctModelLabels([
        row('openai', 'gpt-5'),
        row('openrouter', 'openai/gpt-5'),
        row('anthropic', 'claude-fable-5'),
      ])
    ).toEqual(['GPT-5 · openai', 'GPT-5 · openrouter', 'Claude Fable 5']);
  });

  describe("with the provider list's names", () => {
    afterEach(() => rememberProviderNames([]));

    // W1-04: the second step wrote the provider's KEY, so a person read "GPT-5 · openai" in the
    // cost card while Settings named the same provider "OpenAI".
    it("two rows that read alike add the provider's display name", () => {
      rememberProviderNames([
        { name: 'openai', metadata: { display_name: 'OpenAI' } },
        { name: 'openrouter', metadata: { display_name: 'OpenRouter' } },
        { name: 'versa_bedrock', metadata: { display_name: 'Versa API Bedrock' } },
        { name: 'aws_bedrock', metadata: { display_name: 'Amazon Bedrock' } },
      ]);
      expect(
        distinctModelLabels([
          row('openai', 'gpt-5'),
          row('openrouter', 'openai/gpt-5'),
          row('versa_bedrock', 'us.anthropic.claude-haiku-5-5'),
          row('aws_bedrock', 'global.anthropic.claude-haiku-5-5'),
          row('anthropic', 'claude-fable-5'),
        ])
      ).toEqual([
        'GPT-5 · OpenAI',
        'GPT-5 · OpenRouter',
        'Claude Haiku 5.5 · Versa API Bedrock',
        'Claude Haiku 5.5 · Amazon Bedrock',
        'Claude Fable 5',
      ]);
      // The id still follows where one provider served two snapshots.
      expect(
        distinctModelLabels([
          row('openai', 'gpt-5.5-2026-04-24'),
          row('openai', 'gpt-5.5-2026-07-09'),
        ])
      ).toEqual(['GPT-5.5 · OpenAI · gpt-5.5-2026-04-24', 'GPT-5.5 · OpenAI · gpt-5.5-2026-07-09']);
    });

    it('two providers the list names alike are told apart by the id', () => {
      rememberProviderNames([
        { name: 'lab_a', metadata: { display_name: 'Lab' } },
        { name: 'lab_b', metadata: { display_name: 'Lab' } },
      ]);
      // What reads the same gets more, so the raw id follows the shared name.
      expect(distinctModelLabels([row('lab_a', 'gpt-5'), row('lab_b', 'openai/gpt-5')])).toEqual([
        'GPT-5 · Lab · gpt-5',
        'GPT-5 · Lab · openai/gpt-5',
      ]);
    });

    it('a list that replaces the names forgets the old ones', () => {
      rememberProviderNames([{ name: 'openai', metadata: { display_name: 'OpenAI' } }]);
      expect(providerDisplayName('openai')).toBe('OpenAI');
      rememberProviderNames([{ name: 'anthropic', metadata: { display_name: ' Anthropic ' } }]);
      expect(providerDisplayName('openai')).toBeNull();
      expect(providerDisplayName('anthropic')).toBe('Anthropic');
      expect(providerDisplayName(null)).toBeNull();
    });
  });

  it('adds the raw id where one provider served two snapshots of one model', () => {
    // Two dated snapshots, each priced on its own: the display name drops the date.
    expect(
      distinctModelLabels([
        row('openai', 'gpt-5.5-2026-04-24'),
        row('openai', 'gpt-5.5-2026-07-09'),
        row('openrouter', 'openai/gpt-5.5'),
      ])
    ).toEqual([
      'GPT-5.5 · openai · gpt-5.5-2026-04-24',
      'GPT-5.5 · openai · gpt-5.5-2026-07-09',
      'GPT-5.5 · openrouter',
    ]);
    // Two Bedrock inference profiles of one model, priced differently: it drops the region.
    expect(
      distinctModelLabels([
        row('aws_bedrock', 'us.anthropic.claude-haiku-5-5'),
        row('aws_bedrock', 'global.anthropic.claude-haiku-5-5'),
      ])
    ).toEqual([
      'Claude Haiku 5.5 · aws_bedrock · us.anthropic.claude-haiku-5-5',
      'Claude Haiku 5.5 · aws_bedrock · global.anthropic.claude-haiku-5-5',
    ]);
  });

  it('adds the raw id where the rows have no provider to tell them apart', () => {
    expect(distinguishModels([row(null, 'gpt-5.5-2026-04-24'), row(null, 'gpt-5.5')])).toEqual([
      { provider: false, id: true },
      { provider: false, id: true },
    ]);
  });

  it('ends with every row of a list keyed by provider and id reading apart', () => {
    const rows = [
      row('openai', 'gpt-5.5-2026-04-24'),
      row('openai', 'gpt-5.5-2026-07-09'),
      row('openai', 'gpt-5.5'),
      row('openrouter', 'openai/gpt-5.5'),
      row(null, 'gpt-5.5'),
      row('aws_bedrock', 'us.anthropic.claude-haiku-5-5'),
      row('aws_bedrock', 'global.anthropic.claude-haiku-5-5'),
      row('anthropic', 'claude-haiku-5-5'),
    ];
    const labels = distinctModelLabels(rows);
    expect(new Set(labels).size).toBe(rows.length);
  });
});
