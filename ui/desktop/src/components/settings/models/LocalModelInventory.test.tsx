import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, render, screen, fireEvent } from '@testing-library/react';
import LocalModelInventory from './LocalModelInventory';
import {
  llamaServerStore,
  resetLlamaServerStoreForTests,
  LLAMA_SERVER_OPERATION_TIMEOUT_MS,
  LLAMA_SERVER_POLL_INTERVAL_MS,
} from './llamaServerStore';

const mockLlamacppStatus = vi.fn();
const mockLlamacppEnsure = vi.fn();
const mockLlamacppWarmup = vi.fn();

vi.mock('../../../api', () => ({
  llamacppStatus: (...args: unknown[]) => mockLlamacppStatus(...args),
  llamacppEnsure: (...args: unknown[]) => mockLlamacppEnsure(...args),
  llamacppWarmup: (...args: unknown[]) => mockLlamacppWarmup(...args),
  llamacppDelete: vi.fn(),
}));

const mockCheckOllamaStatus = vi.fn();
const mockPullOllamaModel = vi.fn();

vi.mock('../../../utils/ollamaDetection', () => ({
  checkOllamaStatus: (...args: unknown[]) => mockCheckOllamaStatus(...args),
  deleteOllamaModel: vi.fn(),
  pullOllamaModel: (...args: unknown[]) => mockPullOllamaModel(...args),
}));

const mockToastError = vi.fn();
const mockToastSuccess = vi.fn();
vi.mock('../../../toasts', () => ({
  toastService: {
    error: (...args: unknown[]) => mockToastError(...args),
    success: (...args: unknown[]) => mockToastSuccess(...args),
  },
}));

const catalogEntry = (overrides: Record<string, unknown> = {}) => ({
  name: 'gemma4',
  display_name: 'Gemma 4 E4B',
  family: 'Gemma 4',
  ollama_name: 'gemma4:latest',
  official_url: 'https://ollama.com/library/gemma4',
  hf_spec: 'google/gemma-4-E4B-it-qat-q4_0-gguf:Q4_0',
  download_size: '9.6 GB',
  description: 'Laptop default',
  min_gpu_memory_gib: 16,
  recommended_gpu_memory_gib: 16,
  context_limit: 131072,
  active_params_b: 4,
  speed_hint: 'Fast — ~4B active parameters',
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

const statusResponse = (
  sidecar: Record<string, unknown> = {},
  entry: Record<string, unknown> = {}
) => ({
  data: {
    sidecar: {
      state: 'stopped',
      warmed: false,
      build: 'test',
      detail: null,
      model: null,
      ...sidecar,
    },
    catalog: [catalogEntry(entry)],
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

describe('LocalModelInventory', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetLlamaServerStoreForTests();
    mockLlamacppStatus.mockResolvedValue(statusResponse());
    mockCheckOllamaStatus.mockResolvedValue({ isRunning: false });
  });

  afterEach(() => {
    resetLlamaServerStoreForTests();
    vi.useRealTimers();
  });

  it('shows the expected-speed hint next to the download size in each row (#35)', async () => {
    render(<LocalModelInventory />);
    const row = await screen.findByText(
      /Gemma 4 · 9\.6 GB · Fast — ~4B active parameters · 131,072 context/
    );
    expect(row).toBeInTheDocument();
  });

  it('shows the expected-speed detail row in the model info dialog (#35)', async () => {
    render(<LocalModelInventory />);
    fireEvent.click(await screen.findByText('View info'));

    expect(await screen.findByText('Expected speed')).toBeInTheDocument();
    expect(screen.getAllByText(/Fast — ~4B active parameters/).length).toBeGreaterThanOrEqual(2);
  });

  /// A model id is a NAME, so it is set in the body font — the way the composer
  /// chip, every model picker, the onboarding cards and ApplicationsView set it.
  /// `Ollama model` and `Fallback GGUF` were `mono` while the list row BEHIND
  /// this dialog prints those same two fields in the body font, so one field
  /// changed face between a row and the dialog that row opens.
  ///
  /// The path rows are asserted in the same test on purpose: the fix is "model
  /// ids are names, paths are paths", not "this dialog has no monospace", and a
  /// test that only checked the first half would be satisfied by flattening
  /// every row to the body font.
  ///
  /// jsdom never runs Tailwind, so a computed-font assertion would pass
  /// whatever the class says. This asserts the CLASS on the value element.
  it('sets model ids in the body font and paths in monospace (#F-21)', async () => {
    render(<LocalModelInventory />);
    fireEvent.click(await screen.findByText('View info'));

    const valueFor = async (label: string) => {
      const node = await screen.findByText(label);
      const value = node.nextElementSibling;
      expect(value, `no value cell beside "${label}"`).not.toBeNull();
      return value as HTMLElement;
    };

    // Model ids: names, so body font.
    expect((await valueFor('Ollama model')).className).not.toMatch(/font-mono/);
    expect((await valueFor('Fallback GGUF')).className).not.toMatch(/font-mono/);

    // Filesystem paths: a job mono earns (D-31), and still mono.
    expect((await valueFor('Model store')).className).toMatch(/font-mono/);
  });

  it('renders the busy row for an operation started elsewhere (#34)', async () => {
    llamaServerStore.beginOperation('start', 'gemma4', 'downloading 42%', { poll: false });
    render(<LocalModelInventory />);

    expect(await screen.findByText('downloading 42%')).toBeInTheDocument();
  });

  it('a refresh that settles after unmount updates the store but no component state (finding 8)', async () => {
    let resolveStatus!: (value: unknown) => void;
    mockLlamacppStatus.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveStatus = resolve;
        })
    );
    const view = render(<LocalModelInventory />);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(1);

    // The panel unmounts while the initial refresh is still in flight.
    view.unmount();
    resolveStatus(statusResponse());
    await Promise.resolve();
    await Promise.resolve();

    // The shared store still received the status; the guarded setters made
    // no post-unmount component-state writes (nothing to throw/warn on).
    expect(llamaServerStore.getSnapshot().status).not.toBeNull();
  });

  it('a stale Ollama check cannot begin a fallback operation that supersedes a retry (re-review 1)', async () => {
    const checkResolvers: Array<(value: { isRunning: boolean }) => void> = [];
    mockCheckOllamaStatus.mockImplementation(
      () =>
        new Promise((resolve) => {
          checkResolvers.push(resolve);
        })
    );
    render(<LocalModelInventory />);
    const install = await screen.findByText('Install');

    vi.useFakeTimers();
    fireEvent.click(install);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();

    // The 60-minute deadline fires while the Ollama check is still hanging:
    // terminal timeout, toasted exactly once.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LLAMA_SERVER_OPERATION_TIMEOUT_MS);
    });
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
    expect(mockToastError).toHaveBeenCalledTimes(1);

    // The user retries; the retry's own Ollama check hangs too.
    fireEvent.click(screen.getByText('Install'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const retryId = llamaServerStore.getSnapshot().operation?.id;
    expect(retryId).toBeDefined();

    // The STALE flow's check finally settles. It must not begin the fallback
    // operation (which would supersede the retry and its timers) or reach
    // the ensure call.
    await act(async () => {
      checkResolvers[0]({ isRunning: false });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(llamaServerStore.getSnapshot().operation?.id).toBe(retryId);
    expect(mockLlamacppEnsure).not.toHaveBeenCalled();
    expect(mockToastError).toHaveBeenCalledTimes(1);
  });

  it('a pull that completes after the deadline cannot toast stale success (re-review 2)', async () => {
    mockCheckOllamaStatus.mockResolvedValue({ isRunning: true });
    let resolvePull!: (value: boolean) => void;
    mockPullOllamaModel.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePull = resolve;
        })
    );
    render(<LocalModelInventory />);
    const install = await screen.findByText('Install');

    vi.useFakeTimers();
    fireEvent.click(install);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockPullOllamaModel).toHaveBeenCalled();

    // The pull outlives the 60-minute deadline: terminal timeout.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LLAMA_SERVER_OPERATION_TIMEOUT_MS);
    });
    expect(mockToastError).toHaveBeenCalledTimes(1);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();

    // The pull finally completes; the stale flow must stay silent — no
    // success toast, no post-success refresh.
    const statusCalls = mockLlamacppStatus.mock.calls.length;
    await act(async () => {
      resolvePull(true);
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockToastSuccess).not.toHaveBeenCalled();
    expect(mockLlamacppStatus.mock.calls.length).toBe(statusCalls);
  });

  it('a warm-up that settles after a polled terminal error cannot toast stale success (re-review 3)', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { downloaded: true, download_status: 'downloaded' })
    );
    let resolveWarmup!: (value: unknown) => void;
    mockLlamacppWarmup.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveWarmup = resolve;
        })
    );
    render(<LocalModelInventory />);
    const warm = await screen.findByText('Warm up');

    vi.useFakeTimers();
    fireEvent.click(warm);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // A poll tick reports a terminal sidecar error while the warm-up HTTP
    // call is still in flight; toasted immediately, operation cleared.
    mockLlamacppStatus.mockResolvedValue(
      statusResponse(
        { state: 'error', model: 'gemma4', detail: 'model blew up' },
        { downloaded: true, download_status: 'downloaded' }
      )
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LLAMA_SERVER_POLL_INTERVAL_MS);
    });
    expect(mockToastError).toHaveBeenCalledTimes(1);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();

    // The warm-up call then "succeeds" — but the flow already failed
    // terminally, so it must not report success.
    await act(async () => {
      resolveWarmup({
        data: {
          output: 'OK',
          sidecar: { state: 'ready', warmed: true, build: 'test', model: 'gemma4', detail: null },
        },
      });
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockToastSuccess).not.toHaveBeenCalled();
    expect(mockToastError).toHaveBeenCalledTimes(1);
  });

  it('ends the warm-up operation before the post-success refresh, disarming the deadline (re-review 3)', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { downloaded: true, download_status: 'downloaded' })
    );
    mockLlamacppWarmup.mockResolvedValue({
      data: {
        output: 'OK',
        sidecar: { state: 'ready', warmed: true, build: 'test', model: 'gemma4', detail: null },
      },
    });
    render(<LocalModelInventory />);
    const warm = await screen.findByText('Warm up');

    vi.useFakeTimers();
    // The post-success refresh hangs indefinitely.
    mockLlamacppStatus.mockImplementation(() => new Promise(() => {}));
    fireEvent.click(warm);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Success was reported, and the operation (with its deadline + poll
    // loop) ended BEFORE the refresh started.
    expect(mockToastSuccess).toHaveBeenCalledTimes(1);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();

    // The full deadline elapses while the refresh is still pending: it was
    // disarmed with the operation, so no timeout error can fire.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(LLAMA_SERVER_OPERATION_TIMEOUT_MS);
    });
    expect(mockToastError).not.toHaveBeenCalled();
  });
});
