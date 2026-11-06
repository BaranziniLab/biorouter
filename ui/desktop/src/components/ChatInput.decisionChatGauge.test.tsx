/**
 * A chat whose main model is a decision model has no context ring: each
 * message is asked on its own, nothing adds to the chat's token total, and the
 * ring's Compact button would send `/compact` to a model that cannot summarize.
 * The composer neither draws the ring nor looks a window up for it. Driven
 * through the real ComposerFooter and the real ring.
 */
import React from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  getCurrentModelAndProvider: vi.fn(),
  liveContextWindow: vi.fn(),
  model: { provider: 'ollama_decision', name: 'tev1:0.8b' },
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders, read: vi.fn(async () => null) }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: mocks.getCurrentModelAndProvider,
    currentModel: mocks.model.name,
    currentProvider: mocks.model.provider,
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../utils/liveContextWindow', () => ({ liveContextWindow: mocks.liveContextWindow }));
vi.mock('../hooks/useDiverge', () => ({ useDiverge: () => ({ diverge: vi.fn() }) }));
vi.mock('./bottom_menu/ToolsChip', () => ({ ToolsChip: () => null }));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
vi.mock('./bottom_menu/CostTracker', () => ({
  CostTracker: () => <span data-testid="cost-tracker-stub" />,
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
import { COST_TRACKING_ENABLED } from '../updates';

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});
afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  vi.clearAllMocks();
  mocks.liveContextWindow.mockResolvedValue(null);
  mocks.getProviders.mockResolvedValue([
    {
      name: 'versa_azure',
      metadata: { known_models: [{ name: 'gpt-5.6-sol', context_limit: 1_000_000 }] },
    },
  ]);
  Object.assign(window, {
    appConfig: { get: () => undefined },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
      saveDataUrlToTemp: vi.fn(),
      readTempImageAsBase64: vi.fn(),
      deleteTempFile: vi.fn(),
    },
  });
});

function bindTo(provider: string, name: string) {
  mocks.model.provider = provider;
  mocks.model.name = name;
  mocks.getCurrentModelAndProvider.mockResolvedValue({ model: name, provider });
}

function renderComposer() {
  return render(
    <ChatInput
      sessionId="chat-1"
      handleSubmit={vi.fn()}
      chatState={ChatState.Idle}
      onStop={vi.fn()}
      initialValue=""
      setView={vi.fn()}
      totalTokens={50_000}
      accumulatedInputTokens={0}
      accumulatedOutputTokens={0}
      droppedFiles={[]}
      onFilesProcessed={vi.fn()}
      messagesLength={2}
      toolCount={0}
      onWorkingDirChange={vi.fn()}
    />
  );
}

const ring = () => screen.queryByTestId('context-window-indicator');

describe('the context ring of a decision chat', () => {
  it('draws no ring and looks no window up', async () => {
    bindTo('ollama_decision', 'tev1:0.8b');
    renderComposer();
    await waitFor(() => expect(mocks.getCurrentModelAndProvider).toHaveBeenCalled());
    // Let the lookup run to its end before reading what it did.
    await waitFor(() => expect(screen.getByTestId('composer-footer')).toBeInTheDocument());
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(ring()).toBeNull();
    expect(mocks.liveContextWindow).not.toHaveBeenCalled();
    expect(mocks.getProviders).not.toHaveBeenCalled();
    // The rest of the footer line is the chat's and stays.
    if (COST_TRACKING_ENABLED) {
      expect(screen.getByTestId('cost-tracker-stub')).toBeInTheDocument();
    }
  });

  it('still draws the ring for a chat model', async () => {
    bindTo('versa_azure', 'gpt-5.6-sol');
    renderComposer();
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 1M$/));
    expect(mocks.liveContextWindow).toHaveBeenCalledWith('versa_azure', 'gpt-5.6-sol');
  });
});
