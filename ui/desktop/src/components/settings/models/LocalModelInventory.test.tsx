import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, render, screen, fireEvent, waitFor } from '@testing-library/react';
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
const mockDeleteOllamaModel = vi.fn();
const mockOllamaTagsForModelFile = vi.fn();

vi.mock('../../../utils/ollamaDetection', () => ({
  checkOllamaStatus: (...args: unknown[]) => mockCheckOllamaStatus(...args),
  deleteOllamaModel: (...args: unknown[]) => mockDeleteOllamaModel(...args),
  ollamaTagsForModelFile: (...args: unknown[]) => mockOllamaTagsForModelFile(...args),
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

const statusResponse = (
  sidecar: Record<string, unknown> = {},
  entry: Record<string, unknown> = {},
  system: Record<string, unknown> = {}
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
    // A discrete-GPU machine by default: there, Install pulls through Ollama
    // first (the bare tag is the GGUF build). Apple Silicon skips that pull —
    // see the MLX test below.
    system: {
      os: 'linux',
      total_memory_gib: 64,
      accelerator_memory_gib: 64,
      accelerator_memory_kind: 'vram',
      default_context_size: 131072,
      model_cache_dir: '/tmp/models',
      model_cache_layout: 'test',
      ...system,
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
      /Qwen3\.8 · 19 GB · Moderate, dense 27B; needs 32 GiB of GPU memory, 48 GiB recommended · 262,144 context/
    );
    expect(row).toBeInTheDocument();
  });

  it('shows the expected-speed detail row in the model info dialog (#35)', async () => {
    render(<LocalModelInventory />);
    fireEvent.click(await screen.findByText('View info'));

    expect(await screen.findByText('Expected speed')).toBeInTheDocument();
    expect(screen.getAllByText(/Moderate, dense 27B/).length).toBeGreaterThanOrEqual(2);
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
    llamaServerStore.beginOperation('start', 'qwen3.8-27b', 'downloading 42%', { poll: false });
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
    let resolvePull!: (value: { ok: boolean }) => void;
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
      resolvePull({ ok: true });
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
        { state: 'error', model: 'qwen3.8-27b', detail: 'model blew up' },
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
          sidecar: {
            state: 'ready',
            warmed: true,
            build: 'test',
            model: 'qwen3.8-27b',
            detail: null,
          },
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
        sidecar: {
          state: 'ready',
          warmed: true,
          build: 'test',
          model: 'qwen3.8-27b',
          detail: null,
        },
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
  /**
   * On Apple Silicon the bare Ollama tag is a manifest list that resolves to
   * the MLX build, which llama-server cannot load: the pull "succeeded" and the
   * row went on reading "Needs download" over ~19 GB the user could not use.
   */
  it('never pulls through Ollama on Apple Silicon, where the pull is MLX', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, {}, { os: 'macos', accelerator_memory_kind: 'apple_unified' })
    );
    mockCheckOllamaStatus.mockResolvedValue({ isRunning: true });
    mockLlamacppEnsure.mockResolvedValue(statusResponse({ state: 'starting' }));
    render(<LocalModelInventory />);

    fireEvent.click(await screen.findByText('Install'));

    await waitFor(() =>
      expect(mockLlamacppEnsure).toHaveBeenCalledWith(
        expect.objectContaining({ body: { model: 'qwen3.8-27b' } })
      )
    );
    expect(mockPullOllamaModel).not.toHaveBeenCalled();
    expect(mockCheckOllamaStatus).not.toHaveBeenCalled();
  });

  it('says what Ollama said when its pull fails', async () => {
    mockCheckOllamaStatus.mockResolvedValue({ isRunning: true });
    mockPullOllamaModel.mockResolvedValue({ ok: false, error: 'unexpected EOF' });
    render(<LocalModelInventory />);

    fireEvent.click(await screen.findByText('Install'));

    await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
    expect(mockToastError.mock.calls[0][0].msg).toBe(
      'Ollama could not pull qwen3.8:27b: unexpected EOF'
    );
    expect(mockToastSuccess).not.toHaveBeenCalled();
  });

  it('labels a machine below the minimum, and a CPU-only one', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { suitable: false, suitability_status: 'below_minimum' })
    );
    const view = render(<LocalModelInventory />);
    expect(await screen.findByText('Below 32 GiB minimum')).toBeInTheDocument();
    view.unmount();

    resetLlamaServerStoreForTests();
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { suitable: false, suitability_status: 'cpu_only' })
    );
    render(<LocalModelInventory />);
    expect(await screen.findByText('CPU only (slow)')).toBeInTheDocument();
  });

  /**
   * A 64 GiB Intel Mac clears every memory figure. The confirmation follows the
   * daemon's `cpu_only` verdict, and calls the memory system RAM, not VRAM.
   */
  it('asks before installing on a CPU-only machine, whatever its memory', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse(
        {},
        { suitable: false, suitability_status: 'cpu_only' },
        { os: 'macos', accelerator_memory_gib: 64, accelerator_memory_kind: 'cpu_only' }
      )
    );
    render(<LocalModelInventory />);

    fireEvent.click(await screen.findByText('Install'));

    expect(await screen.findByText('Continue anyway?')).toBeInTheDocument();
    expect(
      screen.getByText(
        /Qwen3\.8 27B would run without GPU acceleration on this machine \(64 GiB system RAM\)/
      )
    ).toBeInTheDocument();
    expect(mockCheckOllamaStatus).not.toHaveBeenCalled();
    expect(mockLlamacppEnsure).not.toHaveBeenCalled();
  });

  describe('deleting a model Llama Server found in Ollama', () => {
    /** Apple Silicon: the bare tag is MLX, the file came from the GGUF tag. */
    const GGUF_BLOB = '/Users/me/.ollama/models/blobs/sha256-f5f1dd89';
    const fromOllama = () =>
      statusResponse(
        {},
        {
          downloaded: true,
          download_status: 'downloaded',
          download_source: 'ollama',
          model_path: GGUF_BLOB,
        },
        { os: 'macos', accelerator_memory_kind: 'apple_unified' }
      );

    const confirmDelete = async () => {
      fireEvent.click(await screen.findByText('Delete'));
      const dialog = await screen.findByText('Delete this local model?');
      expect(dialog).toBeInTheDocument();
      const buttons = screen.getAllByRole('button', { name: 'Delete' });
      fireEvent.click(buttons[buttons.length - 1]);
    };

    beforeEach(() => {
      mockLlamacppStatus.mockResolvedValue(fromOllama());
      mockCheckOllamaStatus.mockResolvedValue({ isRunning: true });
      mockDeleteOllamaModel.mockResolvedValue(true);
    });

    /**
     * It deleted `ollama_name`: the MLX `qwen3.8:27b` the Ollama provider chats
     * with went, the GGUF Llama Server uses stayed, and the row still read
     * installed.
     */
    it('deletes the tag that holds the listed file, not the bare tag', async () => {
      mockOllamaTagsForModelFile.mockResolvedValue(['qwen3.8:27b-q4_K_M']);
      render(<LocalModelInventory />);

      await confirmDelete();

      await waitFor(() => expect(mockToastSuccess).toHaveBeenCalledTimes(1));
      expect(mockOllamaTagsForModelFile).toHaveBeenCalledWith(GGUF_BLOB, 'qwen3.8:27b');
      expect(mockDeleteOllamaModel.mock.calls).toEqual([['qwen3.8:27b-q4_K_M']]);
      expect(mockToastSuccess.mock.calls[0][0].msg).toMatch(/qwen3\.8:27b-q4_K_M/);
    });

    it('deletes nothing, and says how, when no tag can be matched to the file', async () => {
      mockOllamaTagsForModelFile.mockResolvedValue([]);
      render(<LocalModelInventory />);

      await confirmDelete();

      await waitFor(() => expect(mockToastError).toHaveBeenCalledTimes(1));
      expect(mockDeleteOllamaModel).not.toHaveBeenCalled();
      expect(mockToastError.mock.calls[0][0].msg).toMatch(/deleted nothing/);
      expect(mockToastError.mock.calls[0][0].msg).toMatch(/ollama rm/);
      expect(mockToastSuccess).not.toHaveBeenCalled();
    });

    it('does not name the bare tag in the confirmation', async () => {
      render(<LocalModelInventory />);
      fireEvent.click(await screen.findByText('Delete'));
      await screen.findByText('Delete this local model?');
      expect(screen.queryByText(/qwen3\.8:27b will be removed/)).toBeNull();
      expect(
        screen.getByText(/Qwen3\.8 27B's model file will be deleted from Ollama/)
      ).toBeInTheDocument();
    });
  });
});
