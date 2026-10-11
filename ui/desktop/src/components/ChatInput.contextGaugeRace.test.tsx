/**
 * The context gauge after a model switch: the window it shows must be the
 * window of the model the chat is on NOW.
 *
 * Each switch starts a new read of the window, and the reads are not ordered:
 * an Ollama chat's read waits on the daemon's `/api/show` probe of
 * `OLLAMA_HOST`, up to 15 s against a host that drops packets, while a Llama
 * Server or cloud model's read answers at once. With no guard the old model's
 * answer landed last, and the gauge measured the new model's usage against the
 * old model's window.
 */
import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, waitFor } from '@testing-library/react';
import type * as Alerts from './alerts';

const gauge = vi.hoisted(() => ({
  liveContextWindow: vi.fn(),
  addAlert: vi.fn(),
  clearAlerts: vi.fn(),
  // One array for every render: a new `[]` each time is a new dependency for
  // the composer's effects that read it, which re-render without end.
  alerts: [] as unknown[],
}));

vi.mock('../utils/liveContextWindow', () => ({
  liveContextWindow: (...args: unknown[]) => gauge.liveContextWindow(...args),
}));
// The real alert types; only the hook is replaced, so every alert the composer
// raises is recorded with the window it was given.
vi.mock('./alerts', async (importOriginal) => ({
  ...(await importOriginal<typeof Alerts>()),
  useAlerts: () => ({
    alerts: gauge.alerts,
    addAlert: gauge.addAlert,
    clearAlerts: gauge.clearAlerts,
  }),
}));

// The scaffold the sibling ChatInput suites use.
vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    getProviders: vi.fn(async () => []),
    read: vi.fn(async () => null),
  }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: vi.fn(async () => ({ model: 'qwen3.8:27b', provider: 'ollama' })),
    currentModel: 'qwen3.8:27b',
    currentProvider: 'ollama',
    modelConfigStatus: 'ready',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./settings/models/bottom_bar/ModelsBottomBar', () => ({ default: () => null }));
vi.mock('./bottom_menu/BottomMenuExtensionSelection', () => ({
  BottomMenuExtensionSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuSkillSelection', () => ({
  BottomMenuSkillSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuKnowledgeSelection', () => ({
  BottomMenuKnowledgeSelection: () => null,
}));
vi.mock('./bottom_menu/BottomMenuReasoningEffort', () => ({
  BottomMenuReasoningEffort: () => null,
}));
vi.mock('./bottom_menu/CostTracker', () => ({
  CostTracker: () => null,
}));
vi.mock('./MessageQueue', () => ({
  default: () => null,
}));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: { privacy_tier: 'public' } })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(window, {
    appConfig: { get: () => '/default/workdir' },
    electron: {
      directoryChooser: vi.fn(),
      addRecentDir: vi.fn(),
      logInfo: vi.fn(),
      getPathForFile: vi.fn(() => ''),
      on: vi.fn(),
      off: vi.fn(),
    },
  });
});

const composer = (
  effectiveModel: { provider: string; model: string },
  chatState: ChatState = ChatState.Idle
) => (
  <ChatInput
    sessionId="chat-a"
    handleSubmit={vi.fn()}
    chatState={chatState}
    onStop={vi.fn()}
    initialValue=""
    setView={vi.fn()}
    totalTokens={0}
    accumulatedInputTokens={0}
    accumulatedOutputTokens={0}
    droppedFiles={[]}
    onFilesProcessed={vi.fn()}
    messagesLength={0}
    disableAnimation={false}
    toolCount={0}
    onWorkingDirChange={vi.fn()}
    effectiveModel={effectiveModel}
  />
);

/** Every window the gauge has been shown, in order. */
const gaugeWindows = () =>
  gauge.addAlert.mock.calls
    .map(([alert]) => alert as { message?: string; progress?: { total: number } })
    .filter((alert) => alert.message === 'Context window')
    .map((alert) => alert.progress?.total);

describe('the context gauge across a model switch', () => {
  it('refreshes the local window after a turn lowers Ollama’s automatic context', async () => {
    gauge.liveContextWindow.mockResolvedValue(131_072);
    const model = { provider: 'ollama', model: 'qwen3.8:27b' };
    const view = render(composer(model));
    await waitFor(() => expect(gaugeWindows()).toContain(131_072));

    view.rerender(composer(model, ChatState.Thinking));
    view.rerender(composer(model, ChatState.Streaming));
    expect(gauge.liveContextWindow).toHaveBeenCalledTimes(1);

    gauge.liveContextWindow.mockResolvedValue(65_536);
    view.rerender(composer(model, ChatState.Idle));
    await waitFor(() => expect(gaugeWindows().slice(-1)).toEqual([65_536]));
    expect(gauge.liveContextWindow).toHaveBeenCalledTimes(2);
  });

  it('does not re-read a hosted model’s window after a turn', async () => {
    gauge.liveContextWindow.mockResolvedValue(131_072);
    const model = { provider: 'anthropic', model: 'claude-opus-5' };
    const view = render(composer(model));
    await waitFor(() => expect(gaugeWindows()).toContain(131_072));

    view.rerender(composer(model, ChatState.Streaming));
    view.rerender(composer(model, ChatState.Idle));
    expect(gauge.liveContextWindow).toHaveBeenCalledTimes(1);
  });

  it('keeps the new model’s window when the old model’s answer arrives last', async () => {
    let answerOllama!: (window: number | null) => void;
    gauge.liveContextWindow.mockImplementation((provider: string) =>
      provider === 'ollama'
        ? new Promise<number | null>((resolve) => {
            answerOllama = resolve;
          })
        : Promise.resolve(131_072)
    );

    const view = render(composer({ provider: 'ollama', model: 'qwen3.8:27b' }));
    await waitFor(() => expect(gauge.liveContextWindow).toHaveBeenCalledTimes(1));

    // The switch, while the Ollama probe is still out.
    view.rerender(composer({ provider: 'llamacpp', model: 'qwen3.8-27b' }));
    await waitFor(() => expect(gaugeWindows()).toContain(131_072));

    // Now the stale Ollama answer lands.
    await act(async () => {
      answerOllama(65_536);
    });

    const windows = gaugeWindows();
    expect(windows).not.toContain(65_536);
    expect(windows[windows.length - 1]).toBe(131_072);
  });
});
