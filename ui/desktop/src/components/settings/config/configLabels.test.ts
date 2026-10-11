import { describe, expect, it } from 'vitest';
import { configLabels } from '../../../utils/configUtils';
import { configFieldLabel, sentenceCase } from './configLabels';

// V4-13: the editor's labels were Title Case where the key was a known one ("Lead Provider"), a
// lower-case run where it was not ("computer use capabilities version"), and a mix of both in one
// list, with two of them wrapping. They are one name each now, in sentence case.

describe('configFieldLabel', () => {
  it.each([
    // The four the vision pass flagged, and one more of the same shape.
    ['BIOROUTER_AUTO_COMPACT_THRESHOLD', 'Auto compact threshold'],
    ['computer_use_capabilities_version', 'Computer use capabilities version'],
    ['AWS_ENDPOINT_URL_BEDROCK', 'AWS endpoint URL Bedrock'],
    ['context_about_biorouter', 'Context about Biorouter'],
    ['brsdk_tracing', 'App SDK tracing'],
    ['BIOROUTER_MAX_TURNS', 'Max turns'],
    ['WORKSPACE_ANNOUNCE_ONLY', 'Workspace announce only'],
    ['VERSA_AZURE_TIMEOUT', 'Versa Azure timeout'],
  ])('names %s %s', (key, label) => {
    expect(configFieldLabel(key)).toBe(label);
  });

  it.each([
    ['BIOROUTER_PROVIDER', 'Provider'],
    ['BIOROUTER_MODE', 'Mode'],
    ['BIOROUTER_LEAD_PROVIDER', 'Lead provider'],
    ['BIOROUTER_TOOLSHIM_OLLAMA_MODEL', 'Tool shim Ollama model'],
    ['BIOROUTER_CLI_MIN_PRIORITY', 'CLI min priority'],
    ['BIOROUTER_ALLOWLIST', 'Allow list'],
    ['BIOROUTER_WORKFLOW_GITHUB_REPO', 'Workflow GitHub repo'],
    ['OPENAI_HOST', 'OpenAI host'],
    ['XIAOMI_MIMO_HOST', 'Xiaomi MiMo host (region endpoint)'],
    ['ZAI_HOST', 'z.ai host'],
    ['GCP_PROJECT_ID', 'GCP project ID'],
    ['AZURE_OPENAI_API_VERSION', 'Azure OpenAI API version'],
    ['LLAMACPP_PORT', 'Llama Server port'],
    ['CLAUDE_CODE_COMMAND', 'Claude CLI command'],
  ])('keeps the known name for %s in sentence case: %s', (key, label) => {
    expect(configFieldLabel(key)).toBe(label);
  });

  it('never leaves a known name in Title Case', () => {
    for (const key of Object.keys(configLabels)) {
      const label = configFieldLabel(key);
      // A later word that is a plain capitalised word (not a product or acronym) is lower case.
      const later = label.split(' ').slice(1);
      for (const word of later) {
        if (
          /^(Ollama|Llama|Server|Azure|Xiaomi|Databricks|Snowflake|Anthropic|Google|Groq)$/.test(
            word
          )
        )
          continue;
        expect(word, `${key} -> ${label}`).not.toMatch(/^[A-Z][a-z]+$/);
      }
    }
  });

  it('starts every name with a capital and never with the brand prefix', () => {
    for (const key of [
      'BIOROUTER_AUTO_COMPACT_THRESHOLD',
      'computer_use_capabilities_version',
      'brsdk_pii_guardrail',
      'tunnel_port',
    ]) {
      const label = configFieldLabel(key);
      expect(label).toMatch(/^[A-Z]/);
      expect(label).not.toMatch(/^Biorouter /);
    }
  });

  it('keeps a lone brand word as it is', () => {
    expect(configFieldLabel('BIOROUTER')).toBe('Biorouter');
  });

  it('is short enough to sit on one line in the label column', () => {
    // The column is 240px at 14px Arial: about 7px per character, so 34 characters is the line.
    for (const key of [
      'computer_use_capabilities_version',
      'BIOROUTER_AUTO_COMPACT_THRESHOLD',
      'AWS_ENDPOINT_URL_BEDROCK',
    ]) {
      expect(configFieldLabel(key).length).toBeLessThanOrEqual(34);
    }
  });
});

describe('sentenceCase', () => {
  it('lowers a later plain word and keeps the first word, acronyms and mixed-case names', () => {
    expect(sentenceCase('Lead Provider')).toBe('Lead provider');
    expect(sentenceCase('CLI Min Priority')).toBe('CLI min priority');
    expect(sentenceCase('Workflow GitHub Repo')).toBe('Workflow GitHub repo');
    expect(sentenceCase('Azure OpenAI Deployment Name')).toBe('Azure OpenAI deployment name');
  });

  it('keeps a product name that is a plain capitalised word', () => {
    expect(sentenceCase('Tool Shim Ollama Model')).toBe('Tool shim Ollama model');
    expect(sentenceCase('Llama Server Context Size')).toBe('Llama Server context size');
  });

  it('leaves a name that is already in sentence case alone', () => {
    expect(sentenceCase('Provider')).toBe('Provider');
    expect(sentenceCase('Auto compact threshold')).toBe('Auto compact threshold');
  });
});
