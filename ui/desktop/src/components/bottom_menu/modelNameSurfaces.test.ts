import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

import { modelDisplayName } from './modelLabel';

/**
 * One model, one name, on every surface (Phase C, T1 F2; the owner's consistency rule).
 *
 * The same Versa API Bedrock model read `us.anthropic.claude-haiku-5-5` in Settings › Models and
 * the Choose model dialog, `claude-haiku-5-5` on the composer's chip and "Claude Haiku 5.5" in the
 * provider documentation. Each surface had picked its own helper (`friendlyModelName`, the raw id,
 * `shortModelLabel`), which is how they drifted. Now every surface asks `modelDisplayName`, and
 * this file holds that: a surface that names a model through anything else fails here, and so
 * does a surface on the list below that stops asking.
 */
const SRC = join(__dirname, '../..');

/** The surfaces a person reads a chat model's name on. */
const SURFACES = [
  'components/settings/models/bottom_bar/ModelsBottomBar.tsx', // composer chip and its menu
  'components/settings/models/subcomponents/SwitchModelModal.tsx', // Choose model dialog
  'components/settings/models/defaultModelValue.ts', // Settings › Models › Model
  'components/settings/models/ModelsSection.tsx',
  'components/crew/pane/presentation.ts', // default worker, Crew tasks, Crew agent
  'components/crew/pane/CrewModelPicker.tsx', // their model list
  'components/BaseChat.tsx', // chat summary rail, monitors dialog
  'components/decisions/DraftMonitorsDialog.tsx',
  'components/decisions/DecideToolSummary.tsx', // a decide call's models, escalation and vote
  'components/knowledge/IngestPanel/IngestModelPicker.tsx',
  'components/knowledge/IngestPanel/IngestPanel.tsx',
  'components/sessions/HomeUsageCard.tsx',
  'components/bottom_menu/ModelBreakdownTable.tsx', // cost card
  'components/settings/usage/UsagePanel.tsx',
  'utils/sessionSubagents.ts', // agents rail, peek card, History badges
  'components/settings/privacy/PrivacyPanel.tsx',
  'components/privacy/pinnedModel.ts',
  'components/privacy/useConfirmNewChatModel.ts',
  'components/ChatInput.tsx',
  'components/ModelAndProviderContext.tsx',
  'components/copilot/CopilotControl.tsx',
  'components/onboarding/CommercialSetupCard.tsx',
  'components/workflows/shared/WorkflowFormFields.tsx',
  'components/applications/ApplicationsView.tsx', // an app's meta line
  'utils/sessionDecisions.ts', // every decision model label, and a gate's judge
];

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

describe('every surface names a model by its display name', () => {
  it.each(SURFACES)('%s asks modelDisplayName', (file) => {
    expect(readFileSync(join(SRC, file), 'utf8')).toMatch(/\bmodelDisplayName\(/);
  });

  it('no surface names a model with friendlyModelName, the short id, directly', () => {
    const callers = sourceFiles(SRC)
      .filter((path) => /\bfriendlyModelName\(/.test(readFileSync(path, 'utf8')))
      .map((path) => relative(SRC, path).split(sep).join('/'));
    // The helper's own module, where `modelDisplayName` reads it.
    expect(callers).toEqual(['components/bottom_menu/modelLabel.ts']);
  });

  it("no surface names a model by a deployment's alias alone, without the display name", () => {
    // `getModelDisplayName` answers the predefined alias or the raw id, so a surface that asked it
    // directly showed `us.anthropic.claude-haiku-5-5` wherever no alias was set.
    const callers = sourceFiles(SRC)
      .filter((path) => /\bgetModelDisplayName\(/.test(readFileSync(path, 'utf8')))
      .map((path) => relative(SRC, path).split(sep).join('/'));
    expect(callers.sort()).toEqual([
      // The context's Settings answer, which `ModelsSection` passes through `modelDisplayName`.
      'components/ModelAndProviderContext.tsx',
      'components/bottom_menu/modelLabel.ts',
      'components/settings/models/predefinedModelsUtils.ts',
    ]);
  });

  it('reads one Bedrock model the same way through every provider that serves it', () => {
    const names = new Set(
      [
        'us.anthropic.claude-haiku-5-5', // Versa API Bedrock, Settings and the picker before
        'claude-haiku-5-5', // the chip before; Anthropic and Claude Code
        'anthropic/claude-haiku-5.5', // OpenRouter
        'global.anthropic.claude-haiku-5-5', // Amazon Bedrock's global profile
      ].map(modelDisplayName)
    );
    expect([...names]).toEqual(['Claude Haiku 5.5']);
  });

  it.each([
    // Every provider's spelling of one model, provider by provider (T1 F2: "every provider").
    [
      'Claude Sonnet 4.6',
      [
        'us.anthropic.claude-sonnet-4-6', // Versa API Bedrock, Amazon Bedrock
        'claude-sonnet-4-6', // Anthropic, Claude Code
        'anthropic/claude-sonnet-4.6', // OpenRouter
        'databricks-claude-sonnet-4-6', // Databricks
        'claude-sonnet-4-6@20260101', // Vertex AI
      ],
    ],
    [
      'GPT-5.6 Sol',
      [
        'gpt-5.6-sol-2026-07-09', // Versa API Azure
        'gpt-5.6-sol', // OpenAI
        'databricks-gpt-5-6-sol', // Databricks
      ],
    ],
    ['GPT-6 Astra', ['gpt-6-astra', 'openai/gpt-6-astra', 'databricks-gpt-6-astra']], // Codex too
    ['Gemini 3.1 Pro', ['gemini-3.1-pro', 'databricks-gemini-3-1-pro']],
    // The local models Biorouter recommends, as Ollama tags them and as Llama Server names them.
    ['Qwen3.8 27B', ['qwen3.8:27b', 'qwen3.8-27b']],
    ['Gemma 4 31B', ['gemma4:31b', 'gemma4-31b']],
    // A model no family rule knows still reads the same through every provider: its short id.
    ['gpt-oss-120b', ['openai/gpt-oss-120b', 'openai.gpt-oss-120b-1:0', 'gpt-oss-120b']],
    ['GLM-5.3', ['zai-org/GLM-5.3']], // Baseten
    ['clef-flash:latest', ['hf.co/acme/clef-flash:latest', 'clef-flash:latest']], // Ollama
  ])('reads %s the same through every provider', (name, ids) => {
    expect(ids.map(modelDisplayName)).toEqual(ids.map(() => name));
  });
});
