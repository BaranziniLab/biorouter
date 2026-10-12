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
import { afterAll, beforeAll, describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';

// The gauge is the footer's context ring (V2-R4-1). This file drives it through
// the real ComposerFooter and the real ring, and reads the window from the
// ring's name ("100% context left · 0 of 131.1k").
const gauge = vi.hoisted(() => ({
  liveContextWindow: vi.fn(),
}));

vi.mock('../utils/liveContextWindow', () => ({
  liveContextWindow: (...args: unknown[]) => gauge.liveContextWindow(...args),
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
vi.mock('./bottom_menu/ToolsChip', () => ({ ToolsChip: () => null }));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
vi.mock('./bottom_menu/CostTracker', () => ({ CostTracker: () => null }));
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

afterAll(() => {
  vi.unstubAllGlobals();
});

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
    toolCount={0}
    onWorkingDirChange={vi.fn()}
    effectiveModel={effectiveModel}
  />
);

const ring = () => screen.queryByTestId('context-window-indicator');
const WINDOW_131K = /of 131\.1k$/;
const WINDOW_65K = /of 65\.5k$/;

describe('the context gauge across a model switch', () => {
  it('refreshes the local window after a turn lowers Ollama’s automatic context', async () => {
    gauge.liveContextWindow.mockResolvedValue(131_072);
    const model = { provider: 'ollama', model: 'qwen3.8:27b' };
    const view = render(composer(model));
    await waitFor(() => expect(ring()).toHaveAccessibleName(WINDOW_131K));

    view.rerender(composer(model, ChatState.Thinking));
    view.rerender(composer(model, ChatState.Streaming));
    expect(gauge.liveContextWindow).toHaveBeenCalledTimes(1);

    gauge.liveContextWindow.mockResolvedValue(65_536);
    view.rerender(composer(model, ChatState.Idle));
    await waitFor(() => expect(ring()).toHaveAccessibleName(WINDOW_65K));
    expect(gauge.liveContextWindow).toHaveBeenCalledTimes(2);
  });

  it('does not re-read a hosted model’s window after a turn', async () => {
    gauge.liveContextWindow.mockResolvedValue(131_072);
    const model = { provider: 'anthropic', model: 'claude-opus-5' };
    const view = render(composer(model));
    await waitFor(() => expect(ring()).toHaveAccessibleName(WINDOW_131K));

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
    await waitFor(() => expect(ring()).toHaveAccessibleName(WINDOW_131K));

    // Now the stale Ollama answer lands.
    await act(async () => {
      answerOllama(65_536);
    });

    expect(ring()).toHaveAccessibleName(WINDOW_131K);
    expect(ring()).not.toHaveAccessibleName(WINDOW_65K);
  });

  // The same model read twice: the read the chat started when it opened is
  // still out when a turn ends and the re-read after it answers. Both answers
  // are for this chat's model, so only the order decides, and the older one
  // must not put the window the turn moved away from back on the ring.
  it('keeps the re-read window when an older read of the same model answers last', async () => {
    const answers: Array<(window: number | null) => void> = [];
    gauge.liveContextWindow.mockImplementation(
      () =>
        new Promise<number | null>((resolve) => {
          answers.push(resolve);
        })
    );
    const model = { provider: 'ollama', model: 'qwen3.8:27b' };

    const view = render(composer(model, ChatState.Streaming));
    await waitFor(() => expect(gauge.liveContextWindow).toHaveBeenCalledTimes(1));

    view.rerender(composer(model, ChatState.Idle));
    await waitFor(() => expect(gauge.liveContextWindow).toHaveBeenCalledTimes(2));

    await act(async () => {
      answers[1](65_536);
    });
    await waitFor(() => expect(ring()).toHaveAccessibleName(WINDOW_65K));

    await act(async () => {
      answers[0](131_072);
    });
    expect(ring()).toHaveAccessibleName(WINDOW_65K);
  });
});
