import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import LlamaServerInlineCard from './LlamaServerInlineCard';
import {
  llamaServerStore,
  resetLlamaServerStoreForTests,
  LLAMA_SERVER_POLL_INTERVAL_MS,
  LLAMA_SERVER_OPERATION_TIMEOUT_MS,
} from '../settings/models/llamaServerStore';

const mockLlamacppStatus = vi.fn();
const mockLlamacppEnsure = vi.fn();
const mockLlamacppWarmup = vi.fn();

vi.mock('../../api', () => ({
  llamacppStatus: (...args: unknown[]) => mockLlamacppStatus(...args),
  llamacppEnsure: (...args: unknown[]) => mockLlamacppEnsure(...args),
  llamacppWarmup: (...args: unknown[]) => mockLlamacppWarmup(...args),
}));

vi.mock('react-router-dom', () => ({
  useNavigate: () => vi.fn(),
}));

const mockUpsert = vi.fn();
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ upsert: mockUpsert }),
}));

const mockToastError = vi.fn();
const mockToastSuccess = vi.fn();
vi.mock('../../toasts', () => ({
  toastService: {
    error: (...args: unknown[]) => mockToastError(...args),
    success: (...args: unknown[]) => mockToastSuccess(...args),
  },
}));

const catalogEntry = (overrides: Record<string, unknown> = {}) => ({
  name: 'qwen3.8-27b',
  display_name: 'Qwen3.8 27B',
  family: 'Qwen3.8',
  ollama_name: 'qwen3.8:27b',
  official_url: 'https://ollama.com/library/qwen3.8:27b',
  hf_spec: 'ggml-org/Qwen3.8-27B-GGUF:Q4_K_M',
  download_size: '19 GB',
  description: 'Default local model',
  min_gpu_memory_gib: 32,
  recommended_gpu_memory_gib: 48,
  context_limit: 262144,
  active_params_b: 27,
  speed_hint: 'Moderate, dense 27B; needs 32 GiB of GPU memory, 48 GiB recommended',
  is_default: true,
  downloaded: false,
  download_status: 'not_downloaded',
  download_source: 'none',
  fallback_downloaded: false,
  fallback_download_status: 'not_downloaded',
  model_path: null,
  suitable: true,
  suitability_status: 'suitable',
  suitability_message: 'Recommended for this machine.',
  ...overrides,
});

const statusResponse = (sidecar: Record<string, unknown> = {}) => ({
  data: {
    sidecar: {
      state: 'stopped',
      warmed: false,
      build: 'test',
      detail: null,
      model: null,
      ...sidecar,
    },
    catalog: [
      catalogEntry(),
      catalogEntry({
        name: 'gemma4-31b',
        display_name: 'Gemma 4 31B',
        family: 'Gemma 4',
        ollama_name: 'gemma4:31b',
        official_url: 'https://ollama.com/library/gemma4:31b',
        hf_spec: 'google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0',
        download_size: '18 GB',
        active_params_b: 31,
        speed_hint: 'Moderate, dense 31B; needs 32 GiB of GPU memory, 48 GiB recommended',
        is_default: false,
      }),
    ],
    system: {
      os: 'macos',
      total_memory_gib: 64,
      accelerator_memory_gib: 64,
      accelerator_memory_kind: 'apple_unified',
      default_context_size: 131072,
      model_cache_dir: '/tmp/models',
      model_cache_layout: 'test',
    },
  },
});

describe('LlamaServerInlineCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetLlamaServerStoreForTests();
    mockLlamacppStatus.mockResolvedValue(statusResponse());
  });

  afterEach(() => {
    resetLlamaServerStoreForTests();
    vi.useRealTimers();
  });

  it('preselects the default catalog model', async () => {
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    const select = await screen.findByTestId('llamacpp-model-select');
    await waitFor(() => expect(select).toHaveValue('qwen3.8-27b'));
  });

  it('shows download size and expected speed for the selected model (#35)', async () => {
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    const info = await screen.findByTestId('llamacpp-size-speed');
    expect(info).toHaveTextContent(
      '19 GB download · Moderate, dense 27B; needs 32 GiB of GPU memory, 48 GiB recommended'
    );
  });

  it('names the catalog models in its description, read off the catalog', async () => {
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    expect(
      await screen.findByText(/^Run Qwen3\.8 27B or Gemma 4 31B on this computer\./)
    ).toBeInTheDocument();
  });

  /**
   * Both catalog models need 32 GiB. A 16 GiB machine still gets the default,
   * with the daemon's below-minimum verdict in words — and none of the old
   * "Gemma 4 is the laptop default" advice, which named a model that is gone.
   */
  it('warns a machine below the minimum without pointing it at a laptop model', async () => {
    const below = statusResponse();
    below.data.catalog = below.data.catalog.map((entry) => ({
      ...entry,
      suitable: false,
      suitability_status: 'below_minimum',
    }));
    below.data.system = { ...below.data.system, accelerator_memory_gib: 16 };
    mockLlamacppStatus.mockResolvedValue(below);
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);

    expect(
      await screen.findByText(/Qwen3\.8 27B needs at least 32 GiB of GPU-addressable memory/)
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/laptop/i);
  });

  /**
   * An Intel Mac with 64 GiB clears every memory figure; the old raw-number
   * check started it without a word while the daemon rated it CPU-only.
   */
  it('asks before loading on a CPU-only machine, whatever its memory', async () => {
    const cpu = statusResponse();
    cpu.data.catalog = cpu.data.catalog.map((entry) => ({
      ...entry,
      suitable: false,
      suitability_status: 'cpu_only',
    }));
    cpu.data.system = {
      ...cpu.data.system,
      accelerator_memory_gib: 64,
      accelerator_memory_kind: 'cpu_only',
    };
    mockLlamacppStatus.mockResolvedValue(cpu);
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);

    fireEvent.click(await screen.findByTestId('llamacpp-start'));

    expect(await screen.findByText('Load Qwen3.8 27B anyway?')).toBeInTheDocument();
    expect(mockLlamacppEnsure).not.toHaveBeenCalled();
    expect(screen.getAllByText(/64 GiB system RAM/).length).toBeGreaterThanOrEqual(1);
  });

  it('restores the live progress box when remounting during an in-flight download', async () => {
    // A download started earlier (possibly from another surface) is still
    // running in the shared store while this card was unmounted.
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'starting', model: 'qwen3.8-27b', detail: 'downloading 42%' })
    );
    llamaServerStore.beginOperation('start', 'qwen3.8-27b', 'downloading 42%');

    const first = render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    expect(await first.findByTestId('llamacpp-progress')).toHaveTextContent(
      'Preparing qwen3.8-27b. Loading or downloading on first use…'
    );
    first.unmount();

    // The store keeps the operation; a fresh mount shows progress immediately.
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    expect(await screen.findByTestId('llamacpp-progress')).toHaveTextContent('downloading 42%');
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();
  });

  it('runs the start flow through the shared store and connects on success', async () => {
    mockUpsert.mockResolvedValue(undefined);
    mockLlamacppEnsure.mockResolvedValue(
      statusResponse({ state: 'starting', model: 'qwen3.8-27b' })
    );
    mockLlamacppWarmup.mockResolvedValue({
      data: {
        output: 'OK',
        sidecar: {
          state: 'ready',
          warmed: true,
          build: 'test',
          model: 'qwen3.8-27b',
          detail: null,
        },
      },
    });
    const onSuccess = vi.fn();
    render(<LlamaServerInlineCard onSuccess={onSuccess} />);

    const start = await screen.findByTestId('llamacpp-start');
    start.click();

    await waitFor(() => expect(onSuccess).toHaveBeenCalled());
    expect(mockLlamacppEnsure).toHaveBeenCalledWith({
      body: { model: 'qwen3.8-27b' },
      throwOnError: true,
    });
    expect(mockLlamacppWarmup).toHaveBeenCalledWith({
      body: { model: 'qwen3.8-27b' },
      throwOnError: true,
    });
    expect(mockUpsert).toHaveBeenCalledWith('BIOROUTER_MODEL', 'qwen3.8-27b', false);
    // The operation ended, so the shared poll interval is stopped.
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
  });

  it('never writes config or navigates from an unmounted card (finding 7)', async () => {
    mockUpsert.mockResolvedValue(undefined);
    mockLlamacppEnsure.mockResolvedValue(
      statusResponse({ state: 'starting', model: 'qwen3.8-27b' })
    );
    let resolveWarmup!: (value: unknown) => void;
    mockLlamacppWarmup.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveWarmup = resolve;
        })
    );
    const onSuccess = vi.fn();
    const view = render(<LlamaServerInlineCard onSuccess={onSuccess} />);

    fireEvent.click(await view.findByTestId('llamacpp-start'));
    await waitFor(() => expect(mockLlamacppWarmup).toHaveBeenCalled());

    // The user navigates away mid warm-up; the flow keeps running.
    view.unmount();
    resolveWarmup({
      data: {
        output: 'OK',
        sidecar: {
          state: 'ready',
          warmed: true,
          build: 'test',
          model: 'qwen3.8-27b',
          detail: null,
        },
      },
    });

    // The flow still ends the shared operation cleanly...
    await waitFor(() => expect(llamaServerStore.getSnapshot().operation).toBeNull());
    // ...but the dead card must not configure the provider or navigate.
    expect(mockUpsert).not.toHaveBeenCalled();
    expect(onSuccess).not.toHaveBeenCalled();
  });

  it('surfaces a polled sidecar error immediately; the stale flow cannot double-toast or kill a retry (finding 9)', async () => {
    mockUpsert.mockResolvedValue(undefined);
    mockLlamacppEnsure.mockResolvedValue(
      statusResponse({ state: 'starting', model: 'qwen3.8-27b' })
    );
    let rejectWarmup!: (err: Error) => void;
    mockLlamacppWarmup.mockImplementation(
      () =>
        new Promise((_resolve, reject) => {
          rejectWarmup = reject;
        })
    );
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    const start = await screen.findByTestId('llamacpp-start');

    vi.useFakeTimers();
    fireEvent.click(start);
    // Let the ensure call settle and the shared poll loop start.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // The next poll tick reports a terminal sidecar error while the warm-up
    // HTTP call is still hanging.
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'error', model: 'qwen3.8-27b', detail: 'model blew up' })
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LLAMA_SERVER_POLL_INTERVAL_MS);
    });

    // Toasted immediately from the retained store error — NOT deferred until
    // the warm-up request eventually rejects.
    expect(mockToastError).toHaveBeenCalledTimes(1);
    expect(mockToastError.mock.calls[0][0]).toMatchObject({ msg: 'model blew up' });
    expect(llamaServerStore.getSnapshot().operation).toBeNull();

    // The user retries; when the stale flow's warm-up finally rejects it
    // must neither toast a second time nor end the retry's operation.
    const retryOp = llamaServerStore.beginOperation('start', 'qwen3.8-27b');
    await act(async () => {
      rejectWarmup(new Error('502 Bad Gateway'));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockToastError).toHaveBeenCalledTimes(1);
    expect(llamaServerStore.getSnapshot().operation).toMatchObject({ id: retryOp });
    llamaServerStore.endOperation(retryOp);
  });

  it('a superseded ensure never triggers warm-up for the old model (re-review 4)', async () => {
    const ensureResolvers: Array<(value: unknown) => void> = [];
    mockLlamacppEnsure.mockImplementation(
      () =>
        new Promise((resolve) => {
          ensureResolvers.push(resolve);
        })
    );
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    const start = await screen.findByTestId('llamacpp-start');

    vi.useFakeTimers();
    // Freeze status polling so the deadline advance below is deterministic.
    mockLlamacppStatus.mockImplementation(() => new Promise(() => {}));
    fireEvent.click(start);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockLlamacppEnsure).toHaveBeenCalledTimes(1);

    // The deadline times the flow out while ensure is still in flight.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LLAMA_SERVER_OPERATION_TIMEOUT_MS);
    });
    expect(mockToastError).toHaveBeenCalledTimes(1);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();

    // The user retries with a DIFFERENT model; a new operation now owns the
    // singleton sidecar.
    const select = screen.getByTestId('llamacpp-model-select');
    fireEvent.change(select, { target: { value: 'gemma4-31b' } });
    fireEvent.click(screen.getByTestId('llamacpp-start'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const retryId = llamaServerStore.getSnapshot().operation?.id;
    expect(llamaServerStore.getSnapshot().operation).toMatchObject({ model: 'gemma4-31b' });
    expect(mockLlamacppEnsure).toHaveBeenCalledTimes(2);

    // The STALE ensure finally settles "successfully". The stale flow must
    // abort: no warm-up of the OLD model, no disturbance of the retry.
    await act(async () => {
      ensureResolvers[0](statusResponse({ state: 'ready', model: 'qwen3.8-27b' }));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockLlamacppWarmup).not.toHaveBeenCalled();
    expect(llamaServerStore.getSnapshot().operation?.id).toBe(retryId);
    expect(mockToastError).toHaveBeenCalledTimes(1);
  });

  it('unmounting between config writes stops the remaining writes (re-review 5)', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'ready', warmed: true, model: 'qwen3.8-27b' })
    );
    let resolveUpsert!: () => void;
    mockUpsert.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveUpsert = resolve;
        })
    );
    const onSuccess = vi.fn();
    const view = render(<LlamaServerInlineCard onSuccess={onSuccess} />);

    fireEvent.click(await view.findByTestId('llamacpp-connect'));
    await waitFor(() => expect(mockUpsert).toHaveBeenCalledTimes(1));
    // The port goes over the wire as a NUMBER. `/config/upsert` writes what it is
    // given verbatim and the backend reads the key with a typed
    // `Config::get_param::<usize>()`; serde_yaml will not coerce `'11543'` into a
    // `usize`, so a quoted write deserialises as `Err` and is swallowed by the
    // `.ok()`/`.unwrap_or(DEFAULT)` at every call site — the setting saves and does
    // nothing. Asserted on the actual argument with `toBe` plus an explicit
    // `typeof`, because a loose matcher (`expect.anything()`, a truthy check, or a
    // `/11543/` pattern) is satisfied by the string that IS the bug.
    const [portKey, portValue, portSecret] = mockUpsert.mock.calls[0] as [string, unknown, boolean];
    expect(portKey).toBe('LLAMACPP_PORT');
    expect(portValue).toBe(11543);
    expect(typeof portValue).toBe('number');
    expect(portSecret).toBe(false);

    // The card unmounts while the FIRST write is still in flight; the two
    // remaining provider/model writes must never happen.
    view.unmount();
    resolveUpsert();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(mockUpsert).toHaveBeenCalledTimes(1);
    expect(mockToastSuccess).not.toHaveBeenCalled();
    expect(onSuccess).not.toHaveBeenCalled();
  });
});
