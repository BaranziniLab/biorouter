import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import LlamaServerInlineCard from './LlamaServerInlineCard';
import { llamaCopy } from './copy';
import { localModelsCopy } from '../settings/models/copy';
import { llamaMemoryWarning, windowsVramNote } from '../settings/models/localModelMemory';
import type { LlamaCppModel, LlamaCppSystemInfo } from '../../api';
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
        context_limit: 131072,
        active_params_b: 31,
        speed_hint: 'Fast, dense 31B; needs 32 GiB of GPU memory, 48 GiB recommended',
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
    await waitFor(() => expect(select).toHaveTextContent('Qwen3.8 27B · 19 GB · Needs download'));
    expect(select).not.toHaveTextContent('Gemma 4 31B');
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
        'Qwen3.8 27B · 19 GB · Needs download',
        'Gemma 4 31B · 18 GB · Needs download',
      ]);
      expect(options[0]).toHaveAttribute('aria-selected', 'true');
      expect(options[0].querySelector('[data-slot="menu-item-indicator"]')).not.toBeNull();
      expect(options[1]).toHaveAttribute('aria-selected', 'false');
      expect(options[1].querySelector('[data-slot="menu-item-indicator"]')).toBeNull();
    });

    it('picking a model changes the field and the facts below it', async () => {
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      await screen.findByRole('combobox', { name: 'Model' });
      chooseModel(/Gemma 4 31B/);

      expect(screen.getByTestId('llamacpp-model-select')).toHaveTextContent(
        'Gemma 4 31B · 18 GB · Needs download'
      );
      expect(screen.getByTestId('llamacpp-size-speed')).toHaveTextContent(/^Fast · 128k context$/);
    });

    it('cannot be changed while a start is in flight', async () => {
      mockLlamacppStatus.mockResolvedValue(
        statusResponse({ state: 'starting', model: 'qwen3.8-27b', detail: 'downloading 42%' })
      );
      llamaServerStore.beginOperation('start', 'qwen3.8-27b', 'downloading 42%');
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

  // #35 asked for the size and the speed. FR-R2-6: the size is said once in the option and once
  // in the button, so the Advanced summary keeps the speed and the context and not a third size.
  it('shows download size and expected speed for the selected model (#35), the size once each', async () => {
    render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
    const info = await screen.findByTestId('llamacpp-size-speed');
    expect(info).toHaveTextContent(/^Moderate · 256k context$/);
    expect(screen.getByTestId('llamacpp-model-select')).toHaveTextContent('19 GB');
    expect(screen.getByTestId('llamacpp-start')).toHaveTextContent(
      llamaCopy.downloadAndStart('19 GB')
    );
    expect(screen.getAllByText(/\b19 GB/)).toHaveLength(2);
  });

  // Vision FR-R1-3 / FR-S-2: the default first screen printed a box of seven lines (description,
  // size and speed, download state, fallback spec, Ollama tag, store path in mono, the fit
  // sentence) and a warning, and pushed the panel's only action below the fold.
  describe('says one line and keeps the rest behind Advanced', () => {
    it('shows the facts line and the action, with the ids and paths closed away', async () => {
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      await screen.findByTestId('llamacpp-size-speed');
      expect(screen.getByRole('button', { name: llamaCopy.advanced })).toHaveAttribute(
        'aria-expanded',
        'false'
      );
      for (const machine of ['qwen3.8:27b', 'ggml-org/Qwen3.8-27B-GGUF:Q4_K_M', '/tmp/models']) {
        expect(screen.queryByText(machine, { exact: false })).toBeNull();
      }
      expect(screen.queryByText('Default local model')).toBeNull();
      expect(screen.getByTestId('llamacpp-start')).toHaveTextContent(
        llamaCopy.downloadAndStart('19 GB')
      );
      // No status at rest: the row says the server is built in.
      expect(screen.queryByText(/no install needed/)).toBeNull();
    });

    it('opens Advanced on the description, the fit and the machine values in mono', async () => {
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      fireEvent.click(await screen.findByRole('button', { name: llamaCopy.advanced }));
      const details = await screen.findByTestId('llamacpp-details');
      expect(details).toHaveTextContent('Default local model');
      // The fit in plain words with its two numbers, never the daemon's sentence, which repeated
      // the row's label and printed the machine id `apple_unified` (FR-R2-6).
      expect(details).toHaveTextContent(
        'MemoryFits this Mac (64 GB unified memory, 48 GB recommended)'
      );
      expect(details).not.toHaveTextContent('Recommended for this machine.');
      expect(details).not.toHaveTextContent('apple_unified');
      const tag = screen.getByText('qwen3.8:27b');
      expect(tag).toHaveClass('font-mono');
      expect(screen.getByText('/tmp/models')).toHaveClass('font-mono');
      // Labels are the sans face; the old "Store:" label was mono with its value.
      expect(screen.getByText(localModelsCopy.detail.modelStore)).not.toHaveClass('font-mono');
    });

    // Vision FR-R3-6: opening the disclosure (then "Details") replaced the summary "Fast · 128k
    // context" with rows that never said the context window, and the fallback row's label read as
    // a note ("Fallback GGUF, may download").
    it('keeps the context window in Advanced and names the fallback row plainly', async () => {
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      fireEvent.click(await screen.findByRole('button', { name: llamaCopy.advanced }));
      const details = await screen.findByTestId('llamacpp-details');
      expect(details).toHaveTextContent(`${localModelsCopy.detail.context}256k`);

      const label = screen.getByText(localModelsCopy.detail.fallbackDownload);
      expect(label.tagName).toBe('DT');
      expect(label).toHaveTextContent(/^Fallback download$/);
      const value = label.nextElementSibling as HTMLElement;
      // The spec in mono, where the download stands as a sans word after it.
      expect(value).not.toHaveClass('font-mono');
      expect(screen.getByText('ggml-org/Qwen3.8-27B-GGUF:Q4_K_M')).toHaveClass('font-mono');
      expect(value).toHaveTextContent(
        `ggml-org/Qwen3.8-27B-GGUF:Q4_K_M · ${llamaCopy.fallbackIfNeeded}`
      );
      expect(details).not.toHaveTextContent(/GGUF,|may download/);
    });
  });

  // Vision FR-R1-4: a model above 16 GB opened with "This model is above the 16 GB laptop tier"
  // even on the 128 GB machine the app recommends it for.
  describe('warns only when this machine is short of the model', () => {
    const roomy = () =>
      mockLlamacppStatus.mockResolvedValue({
        data: {
          ...statusResponse().data,
          catalog: [catalogEntry()],
          system: { ...statusResponse().data.system, accelerator_memory_gib: 128 },
        },
      });

    it('says nothing on a machine that meets the recommendation', async () => {
      roomy();
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      const facts = await screen.findByTestId('llamacpp-size-speed');
      expect(facts).not.toHaveTextContent(localModelsCopy.largeForThisMac);
      expect(screen.queryByText(/laptop tier/)).toBeNull();
      expect(screen.queryByRole('status')).toBeNull();
    });

    it('says "Large for this Mac" in the facts line when the daemon says it is', async () => {
      mockLlamacppStatus.mockResolvedValue({
        data: {
          ...statusResponse().data,
          catalog: [
            catalogEntry({
              recommended_gpu_memory_gib: 96,
              suitability_status: 'above_recommendation',
            }),
          ],
        },
      });
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      expect(await screen.findByTestId('llamacpp-size-speed')).toHaveTextContent(
        localModelsCopy.largeForThisMac
      );
      // Advanced says it once, with the numbers, rather than a second shortfall sentence.
      fireEvent.click(screen.getByRole('button', { name: llamaCopy.advanced }));
      const details = await screen.findByTestId('llamacpp-details');
      expect(details).toHaveTextContent(
        'Large for this Mac (64 GB unified memory, 96 GB recommended)'
      );
      expect(details).not.toHaveTextContent(/This machine reports/);
    });

    it('says the memory was not read, in words, when the daemon could not read it', async () => {
      mockLlamacppStatus.mockResolvedValue({
        data: {
          ...statusResponse().data,
          catalog: [catalogEntry({ suitability_status: 'unknown_resources' })],
          system: {
            ...statusResponse().data.system,
            os: 'linux',
            accelerator_memory_gib: null,
            accelerator_memory_kind: 'unknown_vram',
          },
        },
      });
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      fireEvent.click(await screen.findByRole('button', { name: llamaCopy.advanced }));
      const details = await screen.findByTestId('llamacpp-details');
      expect(details).toHaveTextContent('VRAM not detected (48 GB recommended)');
      expect(details).not.toHaveTextContent('unknown_vram');
    });

    /**
     * Both catalog models need 32 GB (#393). A 16 GB machine still gets the default, with the
     * daemon's below-minimum verdict in words: the verdict in the summary and the Memory row, the
     * shared sentence before a start, and none of the old "Gemma 4 is the laptop default" advice,
     * which named a model that is gone. In GB, the download sizes' unit (FR-R4-5).
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
      const entry = below.data.catalog[0] as unknown as LlamaCppModel;
      const system = below.data.system as unknown as LlamaCppSystemInfo;
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);

      const facts = await screen.findByTestId('llamacpp-size-speed');
      expect(facts).toHaveTextContent(localModelsCopy.belowMinimum(32));
      fireEvent.click(screen.getByRole('button', { name: llamaCopy.advanced }));
      const details = await screen.findByTestId('llamacpp-details');
      expect(details).toHaveTextContent(
        `${localModelsCopy.detail.memory}${localModelsCopy.belowMinimum(32)} (16 GB unified memory, 48 GB recommended)`
      );
      // Not "not detected": the memory was read, and it is short.
      expect(details).not.toHaveTextContent(/not detected/);
      expect(document.body.textContent).not.toMatch(/laptop/i);

      fireEvent.click(screen.getByTestId('llamacpp-start'));
      expect(await screen.findByText('Load Qwen3.8 27B anyway?')).toBeInTheDocument();
      // The confirmation says the shared sentence, in GB.
      const sentence = llamaMemoryWarning(entry, system) as string;
      expect(sentence).toMatch(/^Qwen3\.8 27B needs at least 32 GB of GPU-addressable memory/);
      expect(screen.getByText(sentence)).toBeInTheDocument();
      expect(mockLlamacppEnsure).not.toHaveBeenCalled();
    });

    /**
     * An Intel Mac with 64 GB clears every memory figure; the old raw-number check started it
     * without a word while the daemon rated it CPU-only (#393). Its memory is system RAM.
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

      expect(await screen.findByTestId('llamacpp-size-speed')).toHaveTextContent(
        localModelsCopy.cpuOnly
      );
      fireEvent.click(screen.getByRole('button', { name: llamaCopy.advanced }));
      expect(await screen.findByTestId('llamacpp-details')).toHaveTextContent(
        `${localModelsCopy.cpuOnly} (64 GB system RAM, 48 GB recommended)`
      );
      fireEvent.click(screen.getByTestId('llamacpp-start'));

      expect(await screen.findByText('Load Qwen3.8 27B anyway?')).toBeInTheDocument();
      expect(mockLlamacppEnsure).not.toHaveBeenCalled();
      expect(
        screen.getByText(/without GPU acceleration on this machine \(64 GB system RAM\)/)
      ).toBeInTheDocument();
    });

    // What the Memory row cannot say: on Windows, system RAM does not count. Not on a CPU-only
    // machine, where system RAM is exactly what counts.
    it('says the Windows VRAM note in Advanced, but not on a CPU-only machine', async () => {
      const windows = statusResponse();
      windows.data.system = {
        ...windows.data.system,
        os: 'windows',
        accelerator_memory_gib: 64,
        accelerator_memory_kind: 'vram',
      };
      mockLlamacppStatus.mockResolvedValue(windows);
      const note = windowsVramNote(windows.data.system as unknown as LlamaCppSystemInfo);
      expect(note).not.toBeNull();
      const view = render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      fireEvent.click(await screen.findByRole('button', { name: llamaCopy.advanced }));
      expect(await screen.findByTestId('llamacpp-details')).toHaveTextContent(note as string);
      view.unmount();
      resetLlamaServerStoreForTests();

      const cpu = statusResponse();
      cpu.data.system = {
        ...cpu.data.system,
        os: 'windows',
        accelerator_memory_gib: 64,
        accelerator_memory_kind: 'cpu_only',
      };
      mockLlamacppStatus.mockResolvedValue(cpu);
      render(<LlamaServerInlineCard onSuccess={vi.fn()} />);
      fireEvent.click(await screen.findByRole('button', { name: llamaCopy.advanced }));
      expect(await screen.findByTestId('llamacpp-details')).not.toHaveTextContent(note as string);
    });
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
    chooseModel(/Gemma 4 31B/);
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
