/**
 * A chat whose main model is a decision model: the composer shows the decision
 * question above its text box, inside the one card, and the text box asks for
 * the text to decide on. Any other chat's composer is unchanged.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, within } from '@testing-library/react';

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
const model = vi.hoisted(() => ({ provider: 'ollama_decision', name: 'tev1:0.8b' }));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: null, provider: null })),
    currentModel: model.name,
    currentProvider: model.provider,
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
vi.mock('./bottom_menu/ToolsChip', () => ({
  ToolsChip: () => <button data-testid="tools-chip-stub">Tools</button>,
}));
vi.mock('./bottom_menu/ModelEffortChip', () => ({
  ModelEffortChip: () => <button data-testid="model-chip-stub">model</button>,
}));
vi.mock('./bottom_menu/ComposerFooter', () => ({
  ComposerFooter: () => <div data-testid="composer-footer" />,
}));
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('./decisions/decisionChatApi', () => ({
  fetchDecisionChatQuestion: vi.fn(async () => null),
  saveDecisionChatQuestion: vi.fn(async () => ({ ok: true })),
}));
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'synthetic-proof' }),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import { COMPOSER_COPY } from './composer/copy';
import { DECISION_CHAT_PLACEHOLDER } from './decisions/DecisionComposer';

beforeEach(() => {
  vi.clearAllMocks();
  model.provider = 'ollama_decision';
  model.name = 'tev1:0.8b';
  Object.assign(window, {
    appConfig: { get: () => '/Users/wgu/Desktop' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(),
      on: vi.fn(),
      off: vi.fn(),
      saveDataUrlToTemp: vi.fn(),
      readTempImageAsBase64: vi.fn(),
      deleteTempFile: vi.fn(),
    },
  });
});

function renderComposer() {
  return render(
    <ChatInput
      sessionId="chat-1"
      handleSubmit={vi.fn()}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={0}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={0}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
}

const card = () => document.querySelector('.biorouter-composer-card') as HTMLElement;

describe('the composer of a decision chat', () => {
  it('puts the decision question inside the card, above the text box', async () => {
    renderComposer();
    const question = await within(card()).findByTestId('decision-composer');
    const text = within(card()).getByTestId('chat-input');
    expect(question.compareDocumentPosition(text) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(text).toHaveAttribute('placeholder', DECISION_CHAT_PLACEHOLDER);
    // The card keeps its one shape: nothing above it.
    expect(card().previousElementSibling).toBeNull();
  });

  it('leaves every other chat’s composer as it was', () => {
    model.provider = 'versa_azure';
    model.name = 'gpt-5.6-sol';
    renderComposer();
    expect(screen.queryByTestId('decision-composer')).toBeNull();
    expect(screen.getByTestId('chat-input')).toHaveAttribute(
      'placeholder',
      COMPOSER_COPY.placeholderEmpty
    );
  });
});
