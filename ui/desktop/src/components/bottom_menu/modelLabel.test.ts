import { describe, expect, it } from 'vitest';
import { friendlyModelName } from './modelLabel';

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
