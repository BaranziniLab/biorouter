import { afterEach, describe, expect, it, vi } from 'vitest';
import { friendlyModelName, modelDisplayName } from './modelLabel';

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

  it.each([
    // One model, every provider's spelling of it.
    ['us.anthropic.claude-haiku-5-5', 'Claude Haiku 5.5'],
    ['global.anthropic.claude-haiku-5-5', 'Claude Haiku 5.5'],
    ['claude-haiku-5-5', 'Claude Haiku 5.5'],
    ['anthropic/claude-haiku-5.5', 'Claude Haiku 5.5'],
    ['us.anthropic.claude-opus-5-5', 'Claude Opus 5.5'],
    ['us.anthropic.claude-sonnet-5-5', 'Claude Sonnet 5.5'],
    ['us.anthropic.claude-sonnet-5', 'Claude Sonnet 5'],
    ['us.anthropic.claude-opus-4-6-v1', 'Claude Opus 4.6'],
    ['us.anthropic.claude-opus-4-5-20251101-v1:0', 'Claude Opus 4.5'],
    ['claude-sonnet-4-5-20250929', 'Claude Sonnet 4.5'],
    ['claude-fable-5-1', 'Claude Fable 5.1'],
    ['claude-3-5-sonnet-20241022', 'Claude 3.5 Sonnet'],
    ['claude-3-haiku', 'Claude 3 Haiku'],
    // GPT, as OpenAI writes it.
    ['gpt-5.6-sol-2026-07-09', 'GPT-5.6 Sol'],
    ['gpt-5.6-sol', 'GPT-5.6 Sol'],
    ['gpt-6-astra', 'GPT-6 Astra'],
    ['gpt-5.4-mini-2026-03-17', 'GPT-5.4 mini'],
    ['gpt-5-nano-2025-08-07', 'GPT-5 nano'],
    ['gpt-4o-2024-11-20', 'GPT-4o'],
    ['gpt-5.5-2026-04-24', 'GPT-5.5'],
    ['gpt-5.3-codex', 'GPT-5.3-Codex'],
    ['gpt-5.1-codex-max', 'GPT-5.1-Codex-Max'],
    ['gemini-3-pro-preview', 'Gemini 3 Pro Preview'],
    ['gemini-2.5-flash', 'Gemini 2.5 Flash'],
    // Databricks' endpoint names, its dashes read as the vendor's dots.
    ['databricks-claude-opus-4-8', 'Claude Opus 4.8'],
    ['databricks-gpt-5-4-mini', 'GPT-5.4 mini'],
    ['databricks-gpt-5-5-pro', 'GPT-5.5 Pro'],
    ['databricks-gemini-3-5-flash-lite', 'Gemini 3.5 Flash Lite'],
    // Vertex AI's `@date`.
    ['claude-opus-4-5@20251101', 'Claude Opus 4.5'],
    // The two local models, as Llama Server's catalog and Ollama's recommendation name them.
    ['qwen3.8-27b', 'Qwen3.8 27B'],
    ['qwen3.8:27b', 'Qwen3.8 27B'],
    ['gemma4-31b', 'Gemma 4 31B'],
    ['gemma4:31b', 'Gemma 4 31B'],
    ['qwen3:1.7b', 'Qwen3 1.7B'],
    ['accounts/fireworks/models/qwen3-235b', 'Qwen3 235B'],
  ])('%s -> %s', (id, name) => {
    expect(modelDisplayName(id)).toBe(name);
  });

  // A family whose naming is not regular keeps the short id rather than a guess: no date, no
  // Bedrock profile, no `vendor/` path. An Ollama tag keeps its `:latest`.
  it.each([
    ['gpt-oss-120b', 'gpt-oss-120b'],
    ['openai/gpt-oss-120b', 'gpt-oss-120b'],
    ['o4-mini-2025-04-16', 'o4-mini'],
    ['zai-org/GLM-5.3', 'GLM-5.3'],
    ['deepseek-ai/DeepSeek-V4.1-Flash', 'DeepSeek-V4.1-Flash'],
    ['accounts/fireworks/models/kimi-k2-instruct', 'kimi-k2-instruct'],
    ['qwen3.8:latest', 'qwen3.8:latest'],
    ['qwen2.5-coder:7b', 'qwen2.5-coder:7b'],
    ['qwen3.8:27b-q4_K_M', 'qwen3.8:27b-q4_K_M'],
    ['gemma4:31b-cloud', 'gemma4:31b-cloud'],
    ['tev1:0.8b', 'tev1:0.8b'],
    ['databricks-meta-llama-3-3-70b-instruct', 'databricks-meta-llama-3-3-70b-instruct'],
    // OpenAI's own dash is a date, not a minor version: never "GPT-4.0613".
    ['gpt-4-0613', 'gpt-4-0613'],
    ['gemini-2.5-pro-preview-06-05', 'gemini-2.5-pro-preview-06-05'],
    ['claude-opus-5-5[1m]', 'claude-opus-5-5[1m]'],
    ['opus', 'opus'],
    ['', ''],
  ])('%s keeps %s', (id, name) => {
    expect(modelDisplayName(id)).toBe(name);
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
