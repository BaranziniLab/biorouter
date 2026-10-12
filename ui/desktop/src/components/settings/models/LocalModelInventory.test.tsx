import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import LocalModelInventory from './LocalModelInventory';
import { localModelsCopy } from './copy';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
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

/** An Apple Silicon Mac, for the words that name the Mac. */
const MAC = { os: 'macos', accelerator_memory_kind: 'apple_unified' };

/** The row's one facts line for the fixture: speed word, download size, context window. */
const FACTS = 'Moderate · 19 GB · 256k context';

/** Open a row's ⋯ menu (the name carries the model, so each row's is distinct). */
const openRowMenu = async (displayName = 'Qwen3.8 27B') => {
  const user = userEvent.setup();
  await user.click(await screen.findByRole('button', { name: localModelsCopy.more(displayName) }));
  return user;
};

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

  it('shows one facts line per row: speed word, download size and context window (#35)', async () => {
    render(<LocalModelInventory />);
    // 262144 tokens is 256k; the rest of the speed hint, the family and the model id are in the
    // info dialog, so a row is a name and one line.
    expect(await screen.findByText(FACTS)).toBeInTheDocument();
    expect(screen.queryByText(/Qwen3\.8 ·/)).not.toBeInTheDocument();
    expect(screen.queryByText(/qwen3\.8:27b/)).not.toBeInTheDocument();
  });

  it('is a Local models section with the count and a refresh glyph in its header', async () => {
    const { container } = render(<LocalModelInventory />);
    expect(await screen.findByText('0 of 1 installed')).toBeInTheDocument();
    const section = container.querySelector(`#${SETTINGS_SECTION_IDS.localModels}`) as HTMLElement;
    expect(section).not.toBeNull();
    expect(
      within(section).getByRole('heading', { name: localModelsCopy.section })
    ).toBeInTheDocument();
    expect(
      within(section).getByRole('button', { name: localModelsCopy.refresh })
    ).toBeInTheDocument();
    // The explanation is the section's InfoTip, never a paragraph.
    expect(
      within(section).getByRole('button', { name: `About ${localModelsCopy.section}` })
    ).toBeInTheDocument();
  });

  it('installs from the row; an installed row has the ⋯ menu and no Install button', async () => {
    render(<LocalModelInventory />);
    expect(await screen.findByRole('button', { name: 'Install Qwen3.8 27B' })).toBeEnabled();
    expect(screen.queryByText(localModelsCopy.warmUp)).not.toBeInTheDocument();

    expect(screen.queryByText(localModelsCopy.installed)).not.toBeInTheDocument();

    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { downloaded: true, download_status: 'downloaded' })
    );
    fireEvent.click(screen.getByRole('button', { name: localModelsCopy.refresh }));
    await screen.findByText('1 of 1 installed');
    expect(screen.queryByRole('button', { name: 'Install Qwen3.8 27B' })).not.toBeInTheDocument();
    // An installed row says so, quietly, in the cell its ⋯ takes over on hover.
    const installed = screen.getByText(localModelsCopy.installed);
    expect(installed.closest('.br-row-actions-meta')).not.toBeNull();
  });

  it('reads the message of a status failure that arrives as a parsed body, never "[object Object]"', async () => {
    // The generated client throws the response body for a non-2xx status: an object, not an Error.
    mockLlamacppStatus.mockRejectedValue({ message: 'The daemon refused the request.' });
    render(<LocalModelInventory />);
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('The daemon refused the request.');
    expect(alert).not.toHaveTextContent('[object Object]');
    // The retry is in the note, and the refresh glyph stays in the header.
    expect(within(alert).getByRole('button', { name: localModelsCopy.retry })).toBeInTheDocument();
  });

  it('says a model is large for this Mac only when it is above the recommendation', async () => {
    mockLlamacppStatus.mockResolvedValue(statusResponse({}, {}, MAC));
    render(<LocalModelInventory />);
    await screen.findByText(FACTS);
    expect(screen.queryByText(localModelsCopy.largeForThisMac)).not.toBeInTheDocument();

    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { suitability_status: 'above_recommendation' }, MAC)
    );
    fireEvent.click(screen.getByRole('button', { name: localModelsCopy.refresh }));
    expect(await screen.findByText(localModelsCopy.largeForThisMac)).toHaveClass(
      'text-text-warning'
    );
  });

  /**
   * The two verdicts #393 added. They read "Unknown" in the old inventory chip, on exactly the
   * machines that most need telling: a 16 GB laptop still offered the default, and an Intel Mac
   * whose RAM clears every figure while the model runs on its processor. They are row warnings
   * now, in the facts line's warning ink, beside the Install they qualify.
   */
  it('warns a row below the minimum, and a CPU-only one, in the facts line', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { suitable: false, suitability_status: 'below_minimum' })
    );
    const view = render(<LocalModelInventory />);
    expect(await screen.findByText(localModelsCopy.belowMinimum(32))).toHaveClass(
      'text-text-warning'
    );
    expect(screen.getByText('Below the 32 GB minimum')).toBeInTheDocument();
    view.unmount();

    resetLlamaServerStoreForTests();
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { suitable: false, suitability_status: 'cpu_only' })
    );
    render(<LocalModelInventory />);
    expect(await screen.findByText(localModelsCopy.cpuOnly)).toHaveClass('text-text-warning');
  });

  it('keeps View info, Warm up and Delete in the row menu, Delete in danger ink', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({}, { downloaded: true, download_status: 'downloaded' })
    );
    render(<LocalModelInventory />);
    await openRowMenu();

    expect(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo })).toBeVisible();
    expect(screen.getByRole('menuitem', { name: localModelsCopy.warmUp })).toBeVisible();
    expect(screen.getByRole('menuitem', { name: localModelsCopy.delete })).toHaveClass(
      'data-[variant=destructive]:text-text-danger'
    );
  });

  it('shows the expected-speed detail row in the model info dialog (#35)', async () => {
    render(<LocalModelInventory />);
    const user = await openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo }));

    expect(await screen.findByText(localModelsCopy.detail.expectedSpeed)).toBeInTheDocument();
    // The row keeps only the speed word; the whole hint is the dialog's.
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Moderate, dense 27B/)).toBeInTheDocument();
  });

  // FR-R2-6: the Recommendation row printed the daemon's sentence, "Recommended for this machine:
  // 64 GiB apple_unified meets the 16 GiB recommendation.", which repeated the label and printed a
  // machine id. The memory numbers are the three rows above it, so the row is the verdict.
  it('says the fit as a verdict in words in the info dialog, never the machine id', async () => {
    mockLlamacppStatus.mockResolvedValue(statusResponse({}, {}, MAC));
    render(<LocalModelInventory />);
    const user = await openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo }));
    const dialog = await screen.findByRole('dialog');
    const fit = within(dialog).getByText(localModelsCopy.detail.recommendation).nextElementSibling;
    expect(fit).toHaveTextContent(/^Fits this Mac$/);
    expect(dialog).not.toHaveTextContent('apple_unified');
    expect(dialog).not.toHaveTextContent('Recommended for this machine.');
    // FR-R4-5: the memory rows say GB, the unit of the Download row beside them.
    const valueOf = (label: string) => within(dialog).getByText(label).nextElementSibling;
    expect(valueOf(localModelsCopy.detail.recommendedMemory)).toHaveTextContent(
      /^\d+ GB GPU-addressable memory$/
    );
    expect(valueOf(localModelsCopy.detail.detectedMemory)).toHaveTextContent(
      /^\d+ GB unified memory$/
    );
    // Every row the renderer words, and the Expected speed row: the catalog's hint without its
    // memory clause ("needs 32 GiB of GPU memory"), which the memory rows say in GB.
    for (const label of [
      localModelsCopy.detail.expectedSpeed,
      localModelsCopy.detail.minimumMemory,
      localModelsCopy.detail.recommendedMemory,
      localModelsCopy.detail.detectedMemory,
      localModelsCopy.detail.recommendation,
    ]) {
      expect(valueOf(label), label).not.toHaveTextContent('GiB');
    }
  });

  /// The three machine values of the info dialog (the model id, the Hugging Face spec and the
  /// path) are set in ONE face (vision round 2, V4-R2-8). They were mixed: the paths mono at the 13px
  /// code role, the two ids sans (F-21 argued an id is a name because the list row behind the dialog
  /// printed it in the body font; the row no longer prints it). Mono at the supporting size is the
  /// scale's mono size for IDs (spec 5.3). The prose rows stay in the body face, so this is not
  /// "the dialog is all monospace".
  ///
  /// jsdom never runs Tailwind, so a computed-font assertion would pass whatever the class says.
  /// This asserts the CLASS on the value element.
  it('sets the model id, the GGUF spec and the model store in one mono face (V4-R2-8)', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse(
        {},
        { download_status: 'downloaded', model_path: '/tmp/models/qwen3.8-27b.gguf' }
      )
    );
    render(<LocalModelInventory />);
    const user = await openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo }));

    const valueFor = async (label: string) => {
      const node = await screen.findByText(label);
      const value = node.nextElementSibling;
      expect(value, `no value cell beside "${label}"`).not.toBeNull();
      return value as HTMLElement;
    };

    const machine = [
      localModelsCopy.detail.ollamaModel,
      localModelsCopy.detail.fallbackDownload,
      localModelsCopy.detail.modelStore,
      localModelsCopy.detail.modelBlob,
    ];
    const classes: string[] = [];
    for (const label of machine) classes.push((await valueFor(label)).className);
    for (const [index, className] of classes.entries()) {
      expect(className, machine[index]).toMatch(/\bfont-mono\b/);
      expect(className, machine[index]).toMatch(/\btext-supporting\b/);
      // The 13px code role is the terminal's and a code block's, not a fact list's.
      expect(className, machine[index]).not.toMatch(/\btext-code\b/);
    }
    // One face, one size: the four class strings are the same.
    expect(new Set(classes).size).toBe(1);

    // Prose values stay in the body face.
    for (const label of [localModelsCopy.detail.family, localModelsCopy.detail.download]) {
      expect((await valueFor(label)).className, label).not.toMatch(/font-mono/);
    }
  });

  /// One model, one state (V4-R2-5). The model below is the one the app measured: Ollama's copy is
  /// partial and the fallback GGUF is whole. The row said "Installed" at rest, its facts line said
  /// "Partial download" and the dialog said "Fallback ready". It is "Installed" in all three now.
  it('says Installed in the row, the facts line and the dialog for a partial primary with a whole fallback (V4-R2-5)', async () => {
    mockLlamacppStatus.mockResolvedValue(
      statusResponse(
        {},
        {
          download_status: 'partial',
          download_source: 'ollama',
          fallback_download_status: 'downloaded',
          fallback_downloaded: true,
        }
      )
    );
    render(<LocalModelInventory />);

    // The row: the quiet status at rest, and no second word under the name.
    const installed = await screen.findByText(localModelsCopy.installed);
    expect(installed.closest('.br-row-actions-meta')).not.toBeNull();
    expect(screen.getByText(FACTS)).toBeInTheDocument();
    expect(screen.queryByText(localModelsCopy.status.partial)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Install Qwen3.8 27B' })).not.toBeInTheDocument();

    // The dialog: the same word, in the Status row, and no jargon.
    const user = await openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo }));
    const dialog = await screen.findByRole('dialog');
    const status = within(dialog).getByText(localModelsCopy.detail.status).nextElementSibling;
    expect(status).toHaveTextContent(new RegExp(`^${localModelsCopy.installed}$`));
    expect(within(dialog).queryByText(/Fallback ready/)).not.toBeInTheDocument();
    expect(within(dialog).queryByText(localModelsCopy.status.partial)).not.toBeInTheDocument();
  });

  it('says Partial download once under a model that is not installed, in the facts line and the dialog (V4-R2-5)', async () => {
    mockLlamacppStatus.mockResolvedValue(statusResponse({}, { download_status: 'partial' }));
    render(<LocalModelInventory />);

    expect(
      await screen.findByText(`${FACTS} · ${localModelsCopy.status.partial}`)
    ).toBeInTheDocument();
    // Nothing is installed, so the row has no Installed status and keeps its button.
    expect(screen.queryByText(localModelsCopy.installed)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Install Qwen3.8 27B' })).toBeEnabled();

    const user = await openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo }));
    const dialog = await screen.findByRole('dialog');
    const status = within(dialog).getByText(localModelsCopy.detail.status).nextElementSibling;
    expect(status).toHaveTextContent(new RegExp(`^${localModelsCopy.status.partial}$`));
  });

  it('says Needs download in the dialog only for a model nothing has started on (V4-R2-5)', async () => {
    render(<LocalModelInventory />);
    // The row says nothing about it: the Install button is the cue.
    expect(await screen.findByText(FACTS)).toBeInTheDocument();
    expect(screen.queryByText(localModelsCopy.status.needsDownload)).not.toBeInTheDocument();

    const user = await openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo }));
    const dialog = await screen.findByRole('dialog');
    const status = within(dialog).getByText(localModelsCopy.detail.status).nextElementSibling;
    expect(status).toHaveTextContent(new RegExp(`^${localModelsCopy.status.needsDownload}$`));
  });

  it('renders the busy row for an operation started elsewhere (#34)', async () => {
    llamaServerStore.beginOperation('start', 'qwen3.8-27b', 'downloading 42%', { poll: false });
    render(<LocalModelInventory />);

    expect(await screen.findByText('downloading 42%')).toBeInTheDocument();
    // The button's name follows the word it shows, and it waits while the work runs.
    const busyButton = screen.getByRole('button', { name: 'Installing Qwen3.8 27B' });
    expect(busyButton).toHaveTextContent(localModelsCopy.installing);
    expect(busyButton).toBeDisabled();
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
    await openRowMenu();
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
    await openRowMenu();
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

  /**
   * A 64 GB Intel Mac clears every memory figure. The confirmation follows the
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

    expect(await screen.findByText(localModelsCopy.confirmResources.title)).toBeInTheDocument();
    expect(
      screen.getByText(
        /Qwen3\.8 27B would run without GPU acceleration on this machine \(64 GB system RAM\)/
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

    /** Delete is in the row's ⋯ menu; the confirmation's button carries the same word. */
    const openDelete = async () => {
      const user = await openRowMenu();
      await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.delete }));
      expect(await screen.findByText(localModelsCopy.confirmDelete.title)).toBeInTheDocument();
    };

    const confirmDelete = async () => {
      await openDelete();
      const buttons = screen.getAllByRole('button', {
        name: localModelsCopy.confirmDelete.confirm,
      });
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
      await openDelete();
      expect(screen.queryByText(/qwen3\.8:27b/)).toBeNull();
      expect(
        screen.getByText(localModelsCopy.confirmDelete.ollamaMessage('Qwen3.8 27B'))
      ).toBeInTheDocument();
    });

    it('names the model by its display name when Ollama does not hold it', async () => {
      mockLlamacppStatus.mockResolvedValue(
        statusResponse(
          {},
          { fallback_downloaded: true, fallback_download_status: 'downloaded' },
          MAC
        )
      );
      render(<LocalModelInventory />);
      await openDelete();
      expect(
        screen.getByText(localModelsCopy.confirmDelete.message('Qwen3.8 27B'))
      ).toBeInTheDocument();
      expect(screen.queryByText(/deleted from Ollama/)).toBeNull();
      expect(screen.queryByText(/qwen3\.8:27b/)).toBeNull();
    });

    /**
     * Where the file lives is something a person acts on now: Delete takes it out of Ollama. So a
     * model Ollama holds says so, as its one state word, in the row and in the dialog alike.
     */
    it('says Downloaded in Ollama as the one state word, in the row and the dialog', async () => {
      render(<LocalModelInventory />);
      const word = await screen.findByText(localModelsCopy.status.inOllama);
      expect(word.closest('.br-row-actions-meta')).not.toBeNull();
      expect(screen.queryByText(localModelsCopy.installed)).not.toBeInTheDocument();
      expect(screen.getByText('1 of 1 installed')).toBeInTheDocument();

      const user = await openRowMenu();
      await user.click(await screen.findByRole('menuitem', { name: localModelsCopy.viewInfo }));
      const dialog = await screen.findByRole('dialog');
      const status = within(dialog).getByText(localModelsCopy.detail.status).nextElementSibling;
      expect(status).toHaveTextContent(new RegExp(`^${localModelsCopy.status.inOllama}$`));
    });
  });
});
