/**
 * The footer's context ring after a full reload of an open chat.
 *
 * Measured live (2026-11-05, Versa Azure gpt-5.5, CDP network log): a reload
 * issues about a dozen `GET /config/providers` within its first 1.5 s, one per
 * consumer that asks while the app's catalog cache is still empty (the
 * `ConfigProvider` itself, the model chip, the privacy hooks, the disclosure
 * gate, this composer). Chromium serialises identical GETs to one URL, and the
 * daemon takes about 0.8 s to answer each, because it constructs every
 * configured provider. The composer used to force its OWN fetch and draw the
 * ring only when that fetch answered: it stood last in the line, and the ring
 * appeared 11 to 12 s after load (23.6 s in an earlier run), while the app's
 * own catalog, which already named the model's window, had landed at about 2 s.
 *
 * Here the composer runs inside the REAL `ConfigProvider` over a fake daemon
 * whose catalog fetches are held open by hand, so "the composer's own fetch is
 * still queued" is something the test controls rather than assumes.
 */
import React from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';

type Catalog = Array<{
  name: string;
  metadata: { known_models: Array<{ name: string; context_limit: number }> };
}>;

/** One catalog fetch the test holds open until it decides how it ends. */
interface HeldFetch {
  resolve: (catalog: Catalog) => void;
  reject: (error: Error) => void;
}

const daemon = vi.hoisted(() => ({
  fetches: [] as Array<{
    resolve: (catalog: unknown) => void;
    reject: (error: Error) => void;
  }>,
}));

// The app-wide selection, as `ModelAndProviderContext` publishes it: `null`
// until its own config read lands, which on a reload is AFTER the composer mounts.
const selection = vi.hoisted(() => ({
  model: null as string | null,
  provider: null as string | null,
}));

vi.mock('../api', () => ({
  // The ConfigProvider's mount reads.
  readAllConfig: vi.fn(async () => ({ data: { config: {} } })),
  readConfig: vi.fn(async () => ({ data: null })),
  removeConfig: vi.fn(async () => ({ data: {} })),
  upsertConfig: vi.fn(async () => ({ data: {} })),
  getExtensions: vi.fn(async () => ({
    data: { extensions: [], warnings: [] },
    response: { status: 200 },
  })),
  addExtension: vi.fn(async () => ({ data: {} })),
  removeExtension: vi.fn(async () => ({ data: {} })),
  getProviderModels: vi.fn(async () => ({ data: [] })),
  // The catalogue long poll parks for the whole test.
  catalogChanges: vi.fn(() => new Promise(() => {})),
  // Every `GET /config/providers`, whoever asks, waits for the test.
  providers: vi.fn(
    () =>
      new Promise((resolve, reject) => {
        daemon.fetches.push({ resolve: (catalog) => resolve({ data: catalog }), reject });
      })
  ),
  // The composer's own reads.
  getSession: vi.fn(async () => ({ data: { privacy_tier: 'public' } })),
  llamacppStatus: vi.fn(async () => ({ data: {} })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));

vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({
    getCurrentModelAndProvider: async () => ({
      model: 'gpt-5.5',
      provider: 'versa_azure',
    }),
    currentModel: selection.model,
    currentProvider: selection.provider,
    modelConfigStatus: selection.model ? 'ready' : 'loading',
    currentModelSupportsVision: false,
    currentModelSupportedInputMimeTypes: null,
  }),
}));
vi.mock('../hooks/useDiverge', () => ({
  useDiverge: () => ({ diverge: vi.fn() }),
}));
vi.mock('./bottom_menu/toolsLists', () => ({
  useToolsLists: () => ({ shown: null, clearQueries: () => {} }),
  ToolsListPanel: () => null,
}));
vi.mock('./bottom_menu/ModelEffortChip', () => ({ ModelEffortChip: () => null }));
vi.mock('./bottom_menu/CostTracker', () => ({ CostTracker: () => null }));
vi.mock('./MessageQueue', () => ({ default: () => null }));
vi.mock('./MentionPopover', () => {
  const MentionPopoverMock = React.forwardRef(() => null);
  MentionPopoverMock.displayName = 'MentionPopoverMock';
  return { default: MentionPopoverMock };
});

import ChatInput from './ChatInput';
import { ConfigProvider } from './ConfigContext';
import { ChatState } from '../types/chatState';

/** The catalog the daemon serves: the chat's model has a 1.1M window. */
const CATALOG: Catalog = [
  {
    name: 'versa_azure',
    metadata: { known_models: [{ name: 'gpt-5.5', context_limit: 1_100_000 }] },
  },
];
/** The same catalog after the model's window changed on the daemon. */
const CATALOG_400K: Catalog = [
  {
    name: 'versa_azure',
    metadata: { known_models: [{ name: 'gpt-5.5', context_limit: 400_000 }] },
  },
];

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
  daemon.fetches.length = 0;
  selection.model = null;
  selection.provider = null;
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

function composer(chatState: ChatState, effectiveModel?: { provider: string; model: string }) {
  return (
    <ConfigProvider>
      <ChatInput
        effectiveModel={effectiveModel}
        sessionId="20261106_1"
        handleSubmit={vi.fn()}
        chatState={chatState}
        onStop={vi.fn()}
        initialValue=""
        setView={vi.fn()}
        totalTokens={29_900}
        accumulatedInputTokens={0}
        accumulatedOutputTokens={0}
        droppedFiles={[]}
        onFilesProcessed={vi.fn()}
        messagesLength={6}
        toolCount={0}
      />
    </ConfigProvider>
  );
}

const ring = () => screen.queryByTestId('context-window-indicator');
const fetchCount = () => daemon.fetches.length;
const held = (index: number): HeldFetch => daemon.fetches[index] as HeldFetch;

/**
 * A reload: the composer mounts on a chat that is still loading, before the
 * app's selection is known; then the selection and the chat's row land. Returns
 * the index of the fetch the composer's CURRENT lookup is waiting on, which is
 * the last one asked for.
 */
async function reload() {
  const view = render(composer(ChatState.LoadingConversation));
  // The ConfigProvider's own mount fetch, and the composer's first lookup.
  await waitFor(() => expect(fetchCount()).toBe(2));

  selection.model = 'gpt-5.5';
  selection.provider = 'versa_azure';
  view.rerender(composer(ChatState.Idle));
  // The selection moved, so the composer looks the window up again.
  await waitFor(() => expect(fetchCount()).toBe(3));
  return { view, own: 2 };
}

describe('the context ring after a reload (catalog fetches queued)', () => {
  it('appears as soon as the app’s catalog lands, while the composer’s own fetch is still queued', async () => {
    const { own } = await reload();
    expect(ring()).toBeNull();

    // Everything ahead of the composer's fetch in the line answers; its own does not.
    await act(async () => {
      for (let i = 0; i < own; i++) held(i).resolve(CATALOG);
    });

    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 1\.1M$/));
  });

  it('draws nothing while the chat is still loading, even with the catalog in hand (V2-R4-1)', async () => {
    const view = render(composer(ChatState.LoadingConversation));
    await waitFor(() => expect(fetchCount()).toBe(2));
    await act(async () => {
      held(0).resolve(CATALOG);
      held(1).resolve(CATALOG);
    });
    // The window is known for the model for NEW chats, but this chat's binding is not.
    expect(ring()).toBeNull();

    selection.model = 'gpt-5.5';
    selection.provider = 'versa_azure';
    view.rerender(composer(ChatState.Idle));
    // A warm catalog answers at once: no waiting on the lookup's own fetch.
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 1\.1M$/));
    expect(fetchCount()).toBe(3);
  });

  it('takes a fresher window from its own fetch when that one answers', async () => {
    const { own } = await reload();
    await act(async () => {
      for (let i = 0; i < own; i++) held(i).resolve(CATALOG);
    });
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 1\.1M$/));

    await act(async () => held(own).resolve(CATALOG_400K));
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 400k$/));
  });

  it('keeps the catalog’s window when its own fetch then fails', async () => {
    const { own } = await reload();
    await act(async () => {
      for (let i = 0; i < own; i++) held(i).resolve(CATALOG);
    });
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 1\.1M$/));

    // `getProviders` reports a failed fetch as an empty catalog. That is no news
    // about the window, and must not put the 128k default in its place.
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    await act(async () => held(own).reject(new Error('daemon unreachable')));
    logged.mockRestore();

    expect(ring()).toHaveAccessibleName(/of 1\.1M$/);
  });

  it('draws no window from a catalog that does not name the model, and falls back as before', async () => {
    const { own } = await reload();
    const other: Catalog = [
      { name: 'versa_azure', metadata: { known_models: [{ name: 'other', context_limit: 9 }] } },
    ];
    await act(async () => {
      for (let i = 0; i < own; i++) held(i).resolve(other);
    });
    expect(ring()).toBeNull();

    // Its own fetch does not name it either: the default window, as it always was.
    await act(async () => held(own).resolve(other));
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 128k$/));
  });

  it('never draws the window of a model the chat has left, when the catalog lands late', async () => {
    const { view, own } = await reload();
    // The person switches this chat to a model the catalog does not name yet,
    // while every fetch, the old lookup's included, is still out.
    view.rerender(composer(ChatState.Idle, { provider: 'versa_azure', model: 'gpt-6-sol' }));
    await waitFor(() => expect(fetchCount()).toBe(own + 2));

    // The app's catalog lands naming only the model the chat was on.
    await act(async () => {
      for (let i = 0; i <= own; i++) held(i).resolve(CATALOG);
    });
    expect(ring()).toBeNull();

    // The new model's own lookup answers, and only that is drawn.
    const withSol: Catalog = [
      {
        name: 'versa_azure',
        metadata: {
          known_models: [
            { name: 'gpt-5.5', context_limit: 1_100_000 },
            { name: 'gpt-6-sol', context_limit: 400_000 },
          ],
        },
      },
    ];
    await act(async () => held(own + 1).resolve(withSol));
    await waitFor(() => expect(ring()).toHaveAccessibleName(/of 400k$/));
  });
});
