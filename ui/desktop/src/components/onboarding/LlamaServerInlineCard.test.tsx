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
        name: 'gemma4-12b',
        display_name: 'Gemma 4 12B',
        download_size: '7.6 GB',
        active_params_b: 12,
        speed_hint: 'Fast — dense 12B, quick to load',
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

/**
 * Open the Model menu from the keyboard, as the other shared-select tests do. Synchronous on
 * purpose: Testing Library's `findBy*` polls on real timers and would hang under the fake ones a
 * few of these cases use, so a caller waits for the card first and then calls this.
 */
function openModelMenu() {
  fireEvent.keyDown(screen.getByRole('combobox', { name: 'Model' }), {
    key: 'ArrowDown',
    code: 'ArrowDown',
  });
}

/** Click the option whose label matches in the open menu. */
function chooseModel(label: RegExp) {
  openModelMenu();
  fireEvent.click(screen.getByRole('option', { name: label }));
}

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
    await waitFor(() => expect(select).toHaveTextContent('Gemma 4 E4B · 9.6 GB · Needs download'));
    expect(select).not.toHaveTextContent('Gemma 4 12B');
  });

  // The Model picker is the app's one select (vision round 4, V4-R4-3): the OS chevron and the OS
  // popup menu were the only ones left, drawn by a native <select>.
  describe('the Model picker is the shared select, not a native one', () => {
    it('is a combobox named by its label, with no native <select> anywhere in the card', async () => {
      const { container } = render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      const box = await screen.findByRole('combobox', { name: 'Model' });
      expect(box.id).toBe('llamacpp-model-select');
      expect(container.querySelector('select')).toBeNull();
    });

    it('lists every catalog model with its size and download state, and marks the chosen one', async () => {
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      await screen.findByRole('combobox', { name: 'Model' });
      openModelMenu();

      const options = screen.getAllByRole('option');
      expect(options.map((option) => option.textContent)).toEqual([
        'Gemma 4 E4B · 9.6 GB · Needs download',
        'Gemma 4 12B · 7.6 GB · Needs download',
      ]);
      expect(options[0]).toHaveAttribute('aria-selected', 'true');
      expect(options[0].querySelector('[data-slot="menu-item-indicator"]')).not.toBeNull();
      expect(options[1]).toHaveAttribute('aria-selected', 'false');
      expect(options[1].querySelector('[data-slot="menu-item-indicator"]')).toBeNull();
    });

    it('picking a model changes the field and the facts below it', async () => {
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      await screen.findByRole('combobox', { name: 'Model' });
      chooseModel(/Gemma 4 12B/);

      expect(screen.getByTestId('llamacpp-model-select')).toHaveTextContent(
        'Gemma 4 12B · 7.6 GB · Needs download'
      );
      expect(screen.getByTestId('llamacpp-size-speed')).toHaveTextContent(
        '7.6 GB download · Fast — dense 12B, quick to load'
      );
    });

    it('cannot be changed while a start is in flight', async () => {
      mockLlamacppStatus.mockResolvedValue(
        statusResponse({ state: 'starting', model: 'gemma4', detail: 'downloading 42%' })
      );
      llamaServerStore.beginOperation('start', 'gemma4', 'downloading 42%');
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      await screen.findByTestId('llamacpp-progress');

      // react-select drops the combobox role from a disabled input, so read the field itself.
      const field = screen.getByTestId('llamacpp-model-select');
      expect(field.querySelector('input[disabled]')).not.toBeNull();
      fireEvent.keyDown(field.querySelector('input') as HTMLElement, {
        key: 'ArrowDown',
        code: 'ArrowDown',
      });
      expect(screen.queryByRole('option')).toBeNull();
    });
  });

  it('shows download size and expected speed for the selected model (#35)', async () => {
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    const info = await screen.findByTestId('llamacpp-size-speed');
    expect(info).toHaveTextContent('9.6 GB download · Fast — ~4B active parameters');
  });

  it('restores the live progress box when remounting during an in-flight download', async () => {
    // A download started earlier (possibly from another surface) is still
    // running in the shared store while this card was unmounted.
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'starting', model: 'gemma4', detail: 'downloading 42%' })
    );
    llamaServerStore.beginOperation('start', 'gemma4', 'downloading 42%');

    const first = render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    expect(await first.findByTestId('llamacpp-progress')).toHaveTextContent(
      'Preparing gemma4. Loading or downloading on first use…'
    );
    first.unmount();

    // The store keeps the operation; a fresh mount shows progress immediately.
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    expect(await screen.findByTestId('llamacpp-progress')).toHaveTextContent('downloading 42%');
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();
  });

  it('runs the start flow through the shared store and connects on success', async () => {
    mockUpsert.mockResolvedValue(undefined);
    mockLlamacppEnsure.mockResolvedValue(statusResponse({ state: 'starting', model: 'gemma4' }));
    mockLlamacppWarmup.mockResolvedValue({
      data: {
        output: 'OK',
        sidecar: { state: 'ready', warmed: true, build: 'test', model: 'gemma4', detail: null },
      },
    });
    const onSuccess = vi.fn();
    render(<LlamaServerInlineCard onSuccess={onSuccess} />);

    const start = await screen.findByTestId('llamacpp-start');
    start.click();

    await waitFor(() => expect(onSuccess).toHaveBeenCalled());
    expect(mockLlamacppEnsure).toHaveBeenCalledWith({
      body: { model: 'gemma4' },
      throwOnError: true,
    });
    expect(mockLlamacppWarmup).toHaveBeenCalledWith({
      body: { model: 'gemma4' },
      throwOnError: true,
    });
    expect(mockUpsert).toHaveBeenCalledWith('BIOROUTER_MODEL', 'gemma4', false);
    // The operation ended, so the shared poll interval is stopped.
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
  });

  it('never writes config or navigates from an unmounted card (finding 7)', async () => {
    mockUpsert.mockResolvedValue(undefined);
    mockLlamacppEnsure.mockResolvedValue(statusResponse({ state: 'starting', model: 'gemma4' }));
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
        sidecar: { state: 'ready', warmed: true, build: 'test', model: 'gemma4', detail: null },
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
    mockLlamacppEnsure.mockResolvedValue(statusResponse({ state: 'starting', model: 'gemma4' }));
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
      statusResponse({ state: 'error', model: 'gemma4', detail: 'model blew up' })
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
    const retryOp = llamaServerStore.beginOperation('start', 'gemma4');
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
    chooseModel(/Gemma 4 12B/);
    fireEvent.click(screen.getByTestId('llamacpp-start'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const retryId = llamaServerStore.getSnapshot().operation?.id;
    expect(llamaServerStore.getSnapshot().operation).toMatchObject({ model: 'gemma4-12b' });
    expect(mockLlamacppEnsure).toHaveBeenCalledTimes(2);

    // The STALE ensure finally settles "successfully". The stale flow must
    // abort: no warm-up of the OLD model, no disturbance of the retry.
    await act(async () => {
      ensureResolvers[0](statusResponse({ state: 'ready', model: 'gemma4' }));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockLlamacppWarmup).not.toHaveBeenCalled();
    expect(llamaServerStore.getSnapshot().operation?.id).toBe(retryId);
    expect(mockToastError).toHaveBeenCalledTimes(1);
  });

  it('unmounting between config writes stops the remaining writes (re-review 5)', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'ready', warmed: true, model: 'gemma4' })
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
