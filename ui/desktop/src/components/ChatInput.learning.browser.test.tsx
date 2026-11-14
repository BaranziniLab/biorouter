// @vitest-environment node
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser } from 'playwright';
import { expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const stubs: Record<string, string> = {
  api: `export const getSession = async () => ({ data: null }); export const startAgent = async () => { throw new Error('Unexpected chat creation'); }; export const postDictionaryObservations = async (options) => { window.observations.push(options); return {}; }; export const getSessionUsageLimit = async () => ({ data: null }); export const getUsageSummary = async () => ({ data: null }); export const putSessionUsageLimit = async () => ({ data: null });`,
  ConfigContext: `export const useConfig = () => ({ getProviders: async () => [], read: async () => null });`,
  ModelAndProviderContext: `export const useModelAndProvider = () => ({ currentModel: 'model', currentProvider: 'openai', modelConfigStatus: 'ready', currentModelSupportsVision: false, getCurrentModelAndProvider: async () => ({ model: null, provider: null }) });`,
  useDiverge: `export const useDiverge = () => ({ diverge: () => {} });`,
  userAction: `export const USER_ACTION_REFUSAL_MARKER = "is the user's decision, not yours"; export const userActionHeaders = async () => ({ 'X-User-Action': 'proof' });`,
  liveContextWindow: `export const liveContextWindow = async () => null;`,
  ComposerChips: `export const ComposerChips = () => null;`,
  toolsLists: `export const useToolsLists = () => ({ shown: null, clearQueries: () => {} }); export const ToolsListPanel = () => null;`,
  ModelEffortChip: `export const ModelEffortChip = () => null;`,
  ComposerFooter: `export const ComposerFooter = () => null;`,
  ComposerPlusMenu: `export const ComposerPlusMenu = () => null;`,
  useSessionWorkingDir: `export const useSessionWorkingDir = () => ({ workingDir: '/work', setWorkingDir: () => {} });`,
  ContextWindowIndicator: `export const contextWindowModelKey = () => '';`,
  MessageQueue: `export default () => null; export const canSteerMessage = () => false;`,
  MentionPopover: `export default () => null;`,
  DictationButton: `export const DictationButton = () => null; export const DictationHoldReason = () => null; export const DictationNotices = () => null;`,
  useDictationComposer: `export const dictationContext = () => ''; export const useDictationComposer = () => ({ available: false, holdsComposer: false, handleKeyDown: () => false, noteInterest: () => {} });`,
  dictationSession: `export const isDictationInserting = () => false; export const noteDictationEdit = () => {};`,
  dictationApi: `export const streamDecisionModels = (models) => models.map(({ provider, model, adapter }) => (adapter ? { provider, model, adapter } : { provider, model }));`,
  useBoundProviderTier: `export const readResolvedProviderTier = async () => 'public';`,
  chatStreamStore: `export const isRunningState = () => false;`,
  ChatCrewAccessBar: `export const useCrewComposerHold = () => null;`,
};

const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      continue;
    }
  }
  return null;
};

it('a programmatic replacement in the real composer posts no stale correction in Chromium', async () => {
  const bundled = await build({
    stdin: {
      resolveDir: here,
      loader: 'tsx',
      contents: `
        import * as React from 'react';
        import { createRoot } from 'react-dom/client';
        import ChatInput from './ChatInput';
        import { saveComposerDraft, recordComposerDictation } from '../utils/composerDrafts';
        window.observations = [];
        window.electron = { platform: 'darwin', logInfo: () => {}, on: () => {}, off: () => {} };
        window.appConfig = { get: () => undefined };
        const source = 'Check the a pass one gene in the lung';
        const draft = recordComposerDictation([], '', { start: 0, end: 0 }, source, { recordId: 'record', raw: 'raw-only-in-memory', changes: [], model: 'small', biasScope: 'public' });
        saveComposerDraft('tab:browser-learning', { ...draft, images: [], files: [] });
        const root = createRoot(document.getElementById('root'));
        window.renderComposer = (initialValue = '') => root.render(<ChatInput
          draftKey="tab:browser-learning" sessionId="chat" initialValue={initialValue}
          handleSubmit={async () => true} chatState="idle" onStop={() => {}}
          setView={() => {}} totalTokens={0} accumulatedInputTokens={0}
          accumulatedOutputTokens={0} droppedFiles={[]} onFilesProcessed={() => {}}
          messagesLength={1} toolCount={0} onWorkingDirChange={() => {}}
        />);
        window.renderComposer();
      `,
    },
    bundle: true,
    write: false,
    platform: 'browser',
    format: 'iife',
    // A stylesheet a component imports (a decision chat's question editor, for one)
    // is irrelevant to what this test checks, and an IIFE bundle has no CSS output.
    loader: { '.css': 'empty' },
    define: { 'process.env.NODE_ENV': '"test"' },
    plugins: [
      {
        name: 'learning-test-boundaries',
        setup(plugin) {
          plugin.onResolve({ filter: /.*/ }, (args) => {
            const name = args.path.split('/').pop() ?? '';
            if (stubs[name]) return { path: name, namespace: 'learning-stub' };
            return undefined;
          });
          plugin.onLoad({ filter: /.*/, namespace: 'learning-stub' }, (args) => ({
            contents: stubs[args.path],
            loader: 'js',
          }));
        },
      },
    ],
  });
  const browser = await launchChromium();
  if (!browser) throw new Error('No Chromium or Chrome browser could launch');
  try {
    for (const programmatic of [false, true]) {
      const page = await browser.newPage();
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.stack ?? error.message));
      page.on('console', (message) => {
        if (message.type() === 'error') errors.push(message.text());
      });
      await page.route('**/*', (route) => route.abort());
      await page.route('https://learning.test/**', (route) =>
        route.fulfill({ contentType: 'text/html', body: '<div id="root"></div>' })
      );
      await page.goto('https://learning.test/');
      await page.addScriptTag({ content: bundled.outputFiles[0].text });
      const field = page.getByTestId('chat-input');
      try {
        await expect
          .poll(() => field.inputValue({ timeout: 4_000 }), { timeout: 5_000 })
          .toBe('Check the a pass one gene in the lung');
      } catch {
        const stage = await Promise.race([
          page.evaluate(() => ({
            rootPresent: document.getElementById('root') !== null,
            rootChildren: document.getElementById('root')?.childElementCount ?? 0,
            textareas: document.querySelectorAll('textarea').length,
            renderComposerPresent:
              typeof (window as unknown as { renderComposer?: unknown }).renderComposer ===
              'function',
          })),
          new Promise<{ timedOut: true }>((resolve) =>
            setTimeout(() => resolve({ timedOut: true }), 1_000)
          ),
        ]);
        throw new Error(
          `Composer did not render: ${JSON.stringify(stage)}; errors: ${errors.join(' | ')}`
        );
      }
      await field.fill('Check the EPAS1 gene in the lung');
      if (programmatic) {
        await page.evaluate(() =>
          (window as unknown as { renderComposer(text: string): void }).renderComposer(
            'Check the HIF2-alpha gene in the lung'
          )
        );
        await expect.poll(() => field.inputValue()).toBe('Check the HIF2-alpha gene in the lung');
      }
      await field.press('Enter');
      await expect.poll(() => field.inputValue()).toBe('');
      await expect
        .poll(() =>
          page.evaluate(
            () => (window as unknown as { observations: unknown[] }).observations.length
          )
        )
        .toBe(programmatic ? 0 : 1);
      expect(errors).toEqual([]);
      await page.close();
    }
  } finally {
    await browser.close();
  }
});
