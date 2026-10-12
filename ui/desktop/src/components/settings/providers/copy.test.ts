import { describe, expect, it } from 'vitest';
import { institutionalCopy, llamaCopy } from '../../onboarding/copy';
import { providerRowLine, providerRowSummary, providerSetupCopy } from './copy';

/** Vision FR-R3-4: every provider row's second line is one face and ends the way a label does. */
describe('providerRowLine', () => {
  it('drops the closing period of a one-sentence description', () => {
    expect(providerRowLine('Run models through Amazon Bedrock.')).toBe(
      'Run models through Amazon Bedrock'
    );
    expect(
      providerRowLine(
        'Models through Azure OpenAI Service (uses Azure credential chain by default).'
      )
    ).toBe('Models through Azure OpenAI Service (uses Azure credential chain by default)');
  });

  it('leaves a line with no period alone, including a dotted name inside it', () => {
    expect(
      providerRowLine('Venice.ai models (Llama, DeepSeek, Mistral) with function calling')
    ).toBe('Venice.ai models (Llama, DeepSeek, Mistral) with function calling');
    expect(providerRowLine('Claude and other models from Anthropic')).toBe(
      'Claude and other models from Anthropic'
    );
  });

  it('keeps the punctuation of a description of several sentences', () => {
    const sagemaker =
      'Run Text Generation Inference models through Amazon SageMaker endpoints. Requires AWS credentials and a SageMaker endpoint URL.';
    expect(providerRowLine(sagemaker)).toBe(sagemaker);
  });

  it('draws a backticked command as plain text', () => {
    expect(providerRowLine('Uses your installed `claude` command.')).toBe(
      'Uses your installed claude command'
    );
  });
});

describe('providerRowSummary', () => {
  it('writes each summary as a row line: no closing period, no backtick, no dash, no "+"', () => {
    for (const line of Object.values(providerRowSummary)) {
      expect(line).not.toMatch(/[.`+]$|[`+—–]/);
      expect(line).toBe(providerRowLine(line));
    }
  });

  // Vision FR-R4-5: the rows sit under their institution's section label, so the line names only
  // the models and the credential ("UCSF · GPT models · API key" said UCSF three times in a row).
  it('names the two Versa gateways by models and credential, under their institution', () => {
    expect(providerRowSummary.versa_azure).toBe('GPT models · API key');
    expect(providerRowSummary.versa_bedrock).toBe('Claude models · access key and secret');
    for (const line of Object.values(providerRowSummary)) expect(line).not.toMatch(/UCSF/);
  });
});

/**
 * Vision FR-R4-5: one disclosure on the first-run path had three wordings ("Details" on Llama
 * Server, "Advanced" with "Endpoint, region" on Versa, "Show 4 options" in every Configure dialog).
 */
describe('the first-run path says one disclosure word, with a "·" summary', () => {
  it('names every disclosure "Advanced"', () => {
    expect(providerSetupCopy.advanced).toBe('Advanced');
    expect(llamaCopy.advanced).toBe(providerSetupCopy.advanced);
    expect(institutionalCopy.advanced).toBe(providerSetupCopy.advanced);
  });

  it('joins every summary with a middle dot, never a comma', () => {
    for (const summary of [institutionalCopy.azureSummary, institutionalCopy.bedrockSummary]) {
      expect(summary).toContain(' · ');
      expect(summary).not.toContain(',');
    }
    expect(providerSetupCopy.advancedSummary(['Organization', 'Timeout'])).toBe(
      'Organization · Timeout'
    );
  });
});
