/**
 * @vitest-environment jsdom
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModelAndProviderProvider, useModelAndProvider } from './ModelAndProviderContext';
import { subscribeSessionBindingChanges } from '../utils/sessionBindingSync';
import type Model from './settings/models/modelInterface';

const mocks = vi.hoisted(() => ({
  llamacppStatus: vi.fn(),
  llamacppWarmup: vi.fn(),
  setConfigProvider: vi.fn(),
  updateAgentProvider: vi.fn(),
  read: vi.fn(),
  getProviders: vi.fn(),
  refreshConfig: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock('../api', () => ({
  llamacppStatus: mocks.llamacppStatus,
  llamacppWarmup: mocks.llamacppWarmup,
  setConfigProvider: mocks.setConfigProvider,
  updateAgentProvider: mocks.updateAgentProvider,
}));

vi.mock('../toasts', () => ({
  toastError: mocks.toastError,
  toastSuccess: mocks.toastSuccess,
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({
    read: mocks.read,
    getProviders: mocks.getProviders,
    refreshConfig: mocks.refreshConfig,
  }),
}));

/**
 * Issue #56 DR-16. The model picker is the USER's act, and the daemon cannot
 * tell it from a model curling the same route unless the request carries
 * `X-User-Action`. Without this bridge the harness would exercise the
 * fail-closed path (no header) and the assertions below would pin nothing about
 * the one property the picker depends on.
 */
const USER_ACTION_KEY = 'user-action-key-under-test';
const userActionHeader = { 'X-User-Action': USER_ACTION_KEY };
Object.defineProperty(window, 'electron', {
  writable: true,
  value: { getUserActionKey: async () => USER_ACTION_KEY },
});

const llamaModel: Model = {
  name: 'qwen3.6',
  provider: 'llamacpp',
  alias: 'Qwen3.6',
  subtext: 'Llama Server',
  context_limit: 131072,
};

const sidecar = {
  state: 'stopped',
  model: null,
  hf_spec: null,
  ollama_name: null,
  model_path: null,
  model_source: null,
  port: 11543,
  binary_path: '/opt/homebrew/bin/llama-server',
  build: 'test',
  detail: null,
  context_size: null,
  warmed: false,
};

const statusResponse = {
  data: {
    sidecar,
    catalog: [
      {
        name: 'qwen3.6',
        display_name: 'Qwen3.6 35B',
        ollama_name: 'qwen3.6:latest',
        hf_spec: 'unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q4_K_M',
        download_size: '23 GB',
        description: 'Large Qwen3.6 MoE model from Ollama library.',
        min_gpu_memory_gib: 48,
        recommended_gpu_memory_gib: 64,
        context_limit: 262144,
        downloaded: false,
        download_status: 'not_downloaded',
        download_source: 'none',
        fallback_downloaded: false,
        fallback_download_status: 'not_downloaded',
        model_path: null,
        suitable: false,
        suitability_status: 'above_recommendation',
        suitability_message:
          'This model recommends 64 GiB of GPU-addressable memory; this machine reports 8 GiB vram.',
        is_default: true,
      },
    ],
    system: {
      os: 'windows',
      total_memory_gib: 32,
      accelerator_memory_gib: 8,
      accelerator_memory_kind: 'vram',
      default_context_size: 32768,
      model_cache_dir: '/Users/test/.ollama/models',
      model_cache_layout: 'Ollama manifests/blobs; Hugging Face fallback cache under the same root',
    },
  },
};

function SwitchHarness() {
  const { changeModel } = useModelAndProvider();
  return (
    <button type="button" onClick={() => void changeModel(null, llamaModel)}>
      Switch to local
    </button>
  );
}

function StatusHarness() {
  const { modelConfigStatus, currentProvider } = useModelAndProvider();
  return <div data-testid="status">{`${modelConfigStatus}:${currentProvider ?? 'none'}`}</div>;
}

const publicModel: Model = {
  name: 'claude-opus-4',
  provider: 'anthropic',
  alias: 'Claude Opus',
  subtext: 'Anthropic',
  context_limit: 200000,
};

const privacyBarrier409 = {
  code: 'privacy_barrier',
  session_classification: 'private',
  provider_tier: 'public',
  available_private_providers: [],
};

/**
 * The generated @hey-api client's own error semantics, reproduced (see
 * `api/client/client.gen.ts`): a non-2xx RETURNS `{error: <parsed body>}` and
 * only THROWS when the caller passed `throwOnError`.
 *
 * A mock that rejects unconditionally would pass against the shipped bug — the
 * refusal would arrive at the catch arm no matter what the call site asked for,
 * and the missing `throwOnError` is the bug. This mock is what makes the test
 * discriminate.
 */
const clientRejecting = (body: unknown) => async (options?: { throwOnError?: boolean }) => {
  if (options?.throwOnError) {
    throw body;
  }
  return { error: body };
};

/**
 * A harness that reports what `changeModel` returned, so the refusal path can be
 * asserted on the boolean the callers branch on rather than on a rendered toast
 * alone.
 */
function SessionSwitchHarness() {
  const { changeModel } = useModelAndProvider();
  const [result, setResult] = useState<string>('pending');
  return (
    <>
      <button
        type="button"
        onClick={() => void changeModel('sess-1', publicModel).then((ok) => setResult(String(ok)))}
      >
        Switch this chat
      </button>
      <div data-testid="change-result">{result}</div>
    </>
  );
}

function renderHarness() {
  return render(
    <ModelAndProviderProvider>
      <SwitchHarness />
    </ModelAndProviderProvider>
  );
}

/**
 * ⚠ **The return type was a lie, and the lie crashed the whole renderer.**
 * `getCurrentModelDisplayName` is declared `Promise<string>`, and
 * `getModelDisplayName` returns its argument unchanged for a model it does not
 * recognise — so an install with no `BIOROUTER_MODEL` got `null`.
 * `ModelsBottomBar` stores that and reads `.length` off it, and the app fell to
 * the error boundary with "Cannot read properties of null (reading 'length')".
 *
 * It was unreachable only because first-run onboarding could not be skipped.
 * The moment a user could enter the app unconfigured — which is the point of
 * "Explore Biorouter first →" — it became the first thing they saw.
 */
describe('getCurrentModelDisplayName with nothing configured', () => {
  function Probe() {
    const { getCurrentModelDisplayName } = useModelAndProvider();
    const [name, setName] = useState<string>('pending');
    return (
      <button onClick={() => void getCurrentModelDisplayName().then((n) => setName(String(n)))}>
        {name}
      </button>
    );
  }

  it('never resolves null, whatever the config holds', async () => {
    mocks.read.mockResolvedValue(null);
    render(
      <ModelAndProviderProvider>
        <Probe />
      </ModelAndProviderProvider>
    );
    fireEvent.click(screen.getByRole('button'));
    await waitFor(() => expect(screen.getByRole('button')).toHaveTextContent('Select Model'));
  });
});

describe('ModelAndProviderProvider Llama Server warm-up', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockImplementation(async (key: string) => {
      if (key === 'BIOROUTER_MODEL') return 'gpt-4o';
      if (key === 'BIOROUTER_PROVIDER') return 'openai';
      return null;
    });
    mocks.getProviders.mockResolvedValue([]);
    mocks.llamacppStatus.mockResolvedValue(statusResponse);
    mocks.llamacppWarmup.mockResolvedValue({
      data: {
        output: 'pong',
        sidecar: {
          ...sidecar,
          state: 'ready',
          model: llamaModel.name,
          warmed: true,
          context_size: 32768,
        },
      },
    });
    mocks.setConfigProvider.mockResolvedValue(undefined);
    mocks.updateAgentProvider.mockResolvedValue(undefined);
    mocks.refreshConfig.mockResolvedValue(undefined);
  });

  it('keeps the previous model when the warm-up prompt is declined', async () => {
    renderHarness();

    fireEvent.click(screen.getByRole('button', { name: 'Switch to local' }));

    expect(await screen.findByText('Warm up local model')).toBeInTheDocument();
    expect(screen.getAllByText(/8 GiB VRAM/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('Needs download')).toBeInTheDocument();
    expect(screen.getByText('Llama Server fallback')).toBeInTheDocument();
    expect(screen.getByText('Fallback may download')).toBeInTheDocument();
    expect(screen.getByText('Ollama model')).toBeInTheDocument();
    expect(screen.getByText('qwen3.6:latest')).toBeInTheDocument();
    expect(screen.getByText('Model store')).toBeInTheDocument();
    expect(screen.getByText('/Users/test/.ollama/models')).toBeInTheDocument();
    expect(screen.getByText(/32,768 tokens/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Keep previous model' }));

    await waitFor(() => {
      expect(screen.queryByText('Warm up local model')).not.toBeInTheDocument();
    });
    expect(mocks.llamacppWarmup).not.toHaveBeenCalled();
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();
    expect(mocks.updateAgentProvider).not.toHaveBeenCalled();
  });

  it('switches to Llama Server only after a non-empty warm-up output', async () => {
    renderHarness();

    fireEvent.click(screen.getByRole('button', { name: 'Switch to local' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Warm up model' }));

    await waitFor(() => {
      expect(mocks.llamacppWarmup).toHaveBeenCalledWith({
        body: { model: llamaModel.name },
        throwOnError: true,
      });
      expect(mocks.setConfigProvider).toHaveBeenCalledWith({
        body: {
          provider: 'llamacpp',
          model: llamaModel.name,
        },
        // Issue #56 DR-16: `/config/set_provider` writes BIOROUTER_PROVIDER and
        // is guarded unconditionally, so the picker must prove it is the user.
        headers: userActionHeader,
        throwOnError: true,
      });
    });
  });

  it('does not switch models when warm-up returns an empty completion', async () => {
    mocks.llamacppWarmup.mockResolvedValueOnce({
      data: {
        output: '  ',
        sidecar: {
          ...sidecar,
          state: 'ready',
          model: llamaModel.name,
          warmed: false,
        },
      },
    });
    renderHarness();

    fireEvent.click(screen.getByRole('button', { name: 'Switch to local' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Warm up model' }));

    await waitFor(() => {
      expect(mocks.toastError).toHaveBeenCalledWith(
        expect.objectContaining({ title: 'Llama Server warm-up failed' })
      );
    });
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();
  });

  // #52 — the switch writes BIOROUTER_PROVIDER/BIOROUTER_MODEL through the API,
  // never through ConfigContext's `upsert`, so nothing invalidated that cache.
  it('refreshes the cached config after writing the new provider', async () => {
    renderHarness();

    fireEvent.click(screen.getByRole('button', { name: 'Switch to local' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Warm up model' }));

    await waitFor(() => expect(mocks.setConfigProvider).toHaveBeenCalled());
    await waitFor(() => expect(mocks.refreshConfig).toHaveBeenCalled());
  });

  it('still reports the switch as successful when the cache refresh fails', async () => {
    mocks.refreshConfig.mockRejectedValue(new Error('daemon unreachable'));
    renderHarness();

    fireEvent.click(screen.getByRole('button', { name: 'Switch to local' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Warm up model' }));

    await waitFor(() => expect(mocks.toastSuccess).toHaveBeenCalled());
    expect(mocks.toastError).not.toHaveBeenCalledWith(
      expect.objectContaining({ title: expect.stringContaining('llamacpp') })
    );
  });
});

// Issue #56 Gate A. `updateAgentProvider` was called WITHOUT `throwOnError`,
// so the generated client returned `{error}` instead of throwing: a 409 privacy
// refusal was discarded, `setConfigProvider` rewrote the global default to the
// refused provider (P4), and a green toast claimed the switch worked while the
// session was still bound to the private model.
describe('ModelAndProviderProvider privacy barrier', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockImplementation(async (key: string) => {
      if (key === 'BIOROUTER_MODEL') return 'gpt-5.5';
      if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
      return null;
    });
    mocks.getProviders.mockResolvedValue([]);
    mocks.setConfigProvider.mockResolvedValue(undefined);
    mocks.refreshConfig.mockResolvedValue(undefined);
    mocks.updateAgentProvider.mockImplementation(clientRejecting(privacyBarrier409));
  });

  it('asks the generated client to throw, or the refusal is discarded', async () => {
    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(mocks.updateAgentProvider).toHaveBeenCalled());
    expect(mocks.updateAgentProvider).toHaveBeenCalledWith(
      expect.objectContaining({ throwOnError: true })
    );
  });

  it('invalidates this chat callable-tool count after a successful provider change', async () => {
    mocks.updateAgentProvider.mockResolvedValueOnce({ data: '' });
    const changed = vi.fn();
    window.addEventListener('session-tools:changed', changed);
    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(screen.getByTestId('change-result')).toHaveTextContent('true'));
    expect(changed).toHaveBeenCalledWith(
      expect.objectContaining({ detail: { sessionId: 'sess-1' } })
    );
    window.removeEventListener('session-tools:changed', changed);
  });

  it('does not report success when the session bind is refused', async () => {
    const changed = vi.fn();
    window.addEventListener('session-tools:changed', changed);
    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(screen.getByTestId('change-result')).toHaveTextContent('false'));
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    // P4: the global default must not be rewritten by a refused per-session bind.
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();
    expect(changed).not.toHaveBeenCalled();
    expect(mocks.toastError).toHaveBeenCalledWith(
      expect.objectContaining({
        title: expect.stringContaining("Can't switch this chat"),
      })
    );
    window.removeEventListener('session-tools:changed', changed);
  });
});

/**
 * Round 3 / N1 — the composer now states the chat's own binding whenever it
 * differs from the app-wide selection, which is only correct while this
 * renderer's copy of the row is current. A per-chat switch is one of the two
 * things that makes it stale, and this is where it is closed.
 */
describe('ModelAndProviderProvider announces the binding it just wrote', () => {
  const seen: Array<Record<string, unknown>> = [];
  let unsubscribe: () => void = () => {};

  beforeEach(() => {
    vi.clearAllMocks();
    seen.length = 0;
    mocks.read.mockImplementation(async (key: string) => {
      if (key === 'BIOROUTER_MODEL') return 'gpt-5.5';
      if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
      return null;
    });
    mocks.getProviders.mockResolvedValue([]);
    mocks.setConfigProvider.mockResolvedValue(undefined);
    mocks.refreshConfig.mockResolvedValue(undefined);
    unsubscribe = subscribeSessionBindingChanges((change) =>
      seen.push(change as unknown as Record<string, unknown>)
    );
  });

  afterEach(() => unsubscribe());

  it('carries the session, the provider, the model and the window', async () => {
    mocks.updateAgentProvider.mockResolvedValue({ data: '' });
    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(screen.getByTestId('change-result')).toHaveTextContent('true'));
    expect(seen).toEqual([
      {
        sessionId: 'sess-1',
        provider: 'anthropic',
        model: 'claude-opus-4',
        contextLimit: 200000,
      },
    ]);
  });

  /**
   * ⚠ **Ordering, not just occurrence.** The announcement lands BEFORE
   * `setConfigProvider` moves the global default, so in the only render where
   * the row and the selection can disagree it is the ROW that holds the new
   * binding. Announcing afterwards would invert that window and flash the model
   * the user had just switched away from — the regression PR #192 narrowed its
   * rule to avoid.
   */
  it('before the global default moves, not after', async () => {
    const order: string[] = [];
    unsubscribe();
    unsubscribe = subscribeSessionBindingChanges(() => order.push('announce'));
    mocks.updateAgentProvider.mockImplementation(async () => {
      order.push('bind');
      return { data: '' };
    });
    mocks.setConfigProvider.mockImplementation(async () => {
      order.push('global');
    });

    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(screen.getByTestId('change-result')).toHaveTextContent('true'));
    expect(order).toEqual(['bind', 'announce', 'global']);
  });

  /** A refused bind wrote no row, so there is nothing to announce. */
  it('says nothing when the bind was refused', async () => {
    mocks.updateAgentProvider.mockImplementation(clientRejecting(privacyBarrier409));
    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(screen.getByTestId('change-result')).toHaveTextContent('false'));
    expect(seen).toEqual([]);
  });

  /**
   * The app-wide picker (Settings → Models, Settings → Providers) passes
   * `sessionId = null` and writes no row at all — which is exactly why an
   * existing chat goes on running its own model after such a switch, and why the
   * composer must say so. Nothing to announce here either.
   */
  it('says nothing for an app-wide switch, which writes no row', async () => {
    // Already loaded, so the switch takes no warm-up dialog and no download.
    mocks.llamacppStatus.mockResolvedValue({
      data: {
        ...statusResponse.data,
        sidecar: { ...sidecar, state: 'ready', model: 'qwen3.6', warmed: true },
      },
    });
    render(
      <ModelAndProviderProvider>
        <SwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch to local' }));

    await waitFor(() => expect(mocks.setConfigProvider).toHaveBeenCalled());
    expect(mocks.updateAgentProvider).not.toHaveBeenCalled();
    expect(seen).toEqual([]);
  });
});

// Issue #56 DR-16. A backend the app did not start was handed no user-action
// key, so it refuses the picker's own raise. That refusal arrives as a PLAIN
// STRING (not the typed Gate A body), so `privacyBarrierOf` returns null and the
// refusal used to fall through to the generic arm — reporting a policy decision
// as "anthropic/claude-opus-4 failed" with the model-facing prose as the
// message, which is the exact failure the Gate A comment three lines above it
// exists to prevent.
describe('ModelAndProviderProvider user-proof refusal', () => {
  // The real 409 body, verbatim from `PrivacyRefusal::TierRaiseNeedsUser`
  // (crates/biorouter/src/privacy/refusal.rs). Typed out rather than assembled
  // from the marker constant, because a fixture built from the thing under test
  // would pass however the two drift; the Rust side has its own test that the
  // marker survives a reword.
  const tierRaiseNeedsUser409 =
    "Switching this chat to a private model is the user's decision, not yours. The request to " +
    "switch it to 'llamacpp' did not come from the model picker, so the chat is unchanged and " +
    'still on its current model. Do not retry — the same call will be refused again. If this ' +
    'task genuinely needs a private model, stop and ask the user to switch this chat to a ' +
    'private model first — in the desktop app under Settings > Models, or with the model chip ' +
    'in the composer.';

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockImplementation(async (key: string) => {
      if (key === 'BIOROUTER_MODEL') return 'gpt-5.5';
      if (key === 'BIOROUTER_PROVIDER') return 'versa_azure';
      return null;
    });
    mocks.getProviders.mockResolvedValue([]);
    mocks.setConfigProvider.mockResolvedValue(undefined);
    mocks.refreshConfig.mockResolvedValue(undefined);
    mocks.updateAgentProvider.mockImplementation(clientRejecting(tierRaiseNeedsUser409));
  });

  it('explains the backend, instead of reporting the refusal as a provider failure', async () => {
    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(screen.getByTestId('change-result')).toHaveTextContent('false'));
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    // The global default must not be rewritten by a refused per-session bind.
    expect(mocks.setConfigProvider).not.toHaveBeenCalled();
    expect(mocks.toastError).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "Can't switch this chat to a private model",
        msg: expect.stringContaining('started outside the Biorouter app'),
      })
    );
    // Not the generic arm: the model-facing prose is not a user-facing message,
    // and the title must not read as a broken provider.
    expect(mocks.toastError).not.toHaveBeenCalledWith(
      expect.objectContaining({ title: expect.stringContaining('failed') })
    );
  });

  it('still reports an ordinary failure of the same route as a failure', async () => {
    // The discriminator is the marker, not "the body happens to be a string":
    // a 500 from `/agent/update_provider` also carries plain text, and telling
    // the user their backend has no user-action key would be a confident lie.
    mocks.updateAgentProvider.mockImplementation(
      clientRejecting('Failed to create anthropic provider: no API key configured')
    );
    render(
      <ModelAndProviderProvider>
        <SessionSwitchHarness />
      </ModelAndProviderProvider>
    );

    fireEvent.click(screen.getByRole('button', { name: 'Switch this chat' }));

    await waitFor(() => expect(screen.getByTestId('change-result')).toHaveTextContent('false'));
    expect(mocks.toastError).toHaveBeenCalledWith(
      expect.objectContaining({ title: expect.stringContaining('failed') })
    );
    expect(mocks.toastError).not.toHaveBeenCalledWith(
      expect.objectContaining({ title: "Can't switch this chat to a private model" })
    );
  });
});

// A null provider/model means two different things — "not read yet" and
// "nothing is configured" — and consumers were sending users to Settings on
// the first. The status says which.
describe('ModelAndProviderProvider config readiness', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getProviders.mockResolvedValue([]);
  });

  it('reports loading until the configured provider has been read', async () => {
    let releaseRead: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      releaseRead = resolve;
    });
    mocks.read.mockImplementation(async (key: string) => {
      await gate;
      if (key === 'BIOROUTER_MODEL') return 'gpt-4o';
      if (key === 'BIOROUTER_PROVIDER') return 'openai';
      return null;
    });

    render(
      <ModelAndProviderProvider>
        <StatusHarness />
      </ModelAndProviderProvider>
    );

    expect(screen.getByTestId('status')).toHaveTextContent('loading:none');

    await act(async () => {
      releaseRead();
    });

    await waitFor(() => expect(screen.getByTestId('status')).toHaveTextContent('ready:openai'));
  });

  it('reports ready — not a permanent loading state — when the config read fails', async () => {
    mocks.read.mockRejectedValue(new Error('config unreadable'));

    render(
      <ModelAndProviderProvider>
        <StatusHarness />
      </ModelAndProviderProvider>
    );

    await waitFor(() => expect(screen.getByTestId('status')).toHaveTextContent('ready:none'));
  });
});
