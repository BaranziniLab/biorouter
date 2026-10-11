import React from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';

// The composer decides which context window the footer ring reads, and the
// ring draws only against a window marked loaded (V2-R4-1). So "loaded" has to
// mean "loaded for the model this chat is bound to NOW": two lookups can be in
// flight at once (the composer mounts before the chat's row names its binding,
// or the person changes model twice quickly), and whichever finished LAST used
// to win, however old its question was. This file drives that race through the
// real ComposerFooter and the real ring, with the lookups held open by hand.

const mocks = vi.hoisted(() => ({
  getProviders: vi.fn(),
  read: vi.fn(),
  getCurrentModelAndProvider: vi.fn(),
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ getProviders: mocks.getProviders, read: mocks.read }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: mocks.getCurrentModelAndProvider,
    currentModel: 'app-model',
    currentProvider: 'app-provider',
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
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});
vi.mock('../api', () => ({
  getSession: vi.fn(async () => ({ data: null })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

import ChatInput from './ChatInput';
import { ChatState } from '../types/chatState';
import type { PinnedModelView } from '../hooks/chatStreamStore';

type Providers = Array<{
  name: string;
  metadata: { known_models: Array<{ name: string; context_limit: number }> };
}>;

/** One lookup the test holds open until it decides how it ends. */
function deferred() {
  let resolve!: (providers: Providers) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<Providers>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// Two models with windows far enough apart to read at a glance in the ring's
// name: "of 1M" against "of 200k".
const PROVIDERS: Providers = [
  {
    name: 'app-provider',
    metadata: { known_models: [{ name: 'app-model', context_limit: 1_000_000 }] },
  },
  {
    name: 'pinned-provider',
    metadata: { known_models: [{ name: 'pinned-model', context_limit: 200_000 }] },
  },
];
const PINNED: PinnedModelView = { provider: 'pinned-provider', model: 'pinned-model' };

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
  mocks.read.mockResolvedValue(null);
  // The chat has no pin yet, so the first lookup measures the app's selection.
  mocks.getCurrentModelAndProvider.mockResolvedValue({
    model: 'app-model',
    provider: 'app-provider',
  });
  Object.assign(window, {
    appConfig: { get: () => undefined },
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

function chat(effectiveModel?: PinnedModelView) {
  return (
    <ChatInput
      sessionId="session-1"
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
      effectiveModel={effectiveModel}
    />
  );
}

/** Mount on the app's model, then pin the chat to another while the first lookup is still open. */
async function mountThenPin() {
  const first = deferred();
  const second = deferred();
  mocks.getProviders.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

  const view = render(chat());
  // The first run has reached its provider lookup before the binding arrives.
  await waitFor(() => expect(mocks.getProviders).toHaveBeenCalledTimes(1));
  view.rerender(chat(PINNED));
  await waitFor(() => expect(mocks.getProviders).toHaveBeenCalledTimes(2));
  return { first, second };
}

const ring = () => screen.queryByTestId('context-window-indicator');

describe('ChatInput context window across overlapping lookups', () => {
  it('draws no ring while the lookups are open', async () => {
    await mountThenPin();
    expect(ring()).toBeNull();
  });

  it('keeps the newest lookup when an older one finishes after it', async () => {
    const { first, second } = await mountThenPin();

    await act(async () => second.resolve(PROVIDERS));
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 200k$/));

    // The first run was about the app's model, which this chat is no longer bound to.
    await act(async () => first.resolve(PROVIDERS));
    expect(ring()).toHaveAccessibleName(/of 200k$/);
    expect(ring()).not.toHaveAccessibleName(/of 1M/);
  });

  it('does not let an older lookup that fails put the default window back', async () => {
    const { first, second } = await mountThenPin();

    await act(async () => second.resolve(PROVIDERS));
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 200k$/));

    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    await act(async () => first.reject(new Error('offline')));
    logged.mockRestore();

    expect(ring()).toHaveAccessibleName(/of 200k$/);
  });

  it('still takes the newest lookup when the lookups finish in order', async () => {
    const { first, second } = await mountThenPin();

    await act(async () => first.resolve(PROVIDERS));
    // The older answer must not stand in for the chat's model while the newest is open.
    expect(ring()).toBeNull();

    await act(async () => second.resolve(PROVIDERS));
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 200k$/));
  });
});
