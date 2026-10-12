import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import OllamaInlineCard from './OllamaInlineCard';
import { ollamaCopy } from './copy';
import { localModelsCopy } from '../settings/models/copy';
import type { OllamaRecommendationsResponse } from '../../api';
import type * as Detection from '../../utils/ollamaDetection';

const mocks = vi.hoisted(() => ({
  getOllamaSetup: vi.fn(),
  checkOllamaStatus: vi.fn(),
  getOllamaModels: vi.fn(),
  pullOllamaModel: vi.fn(),
  pollForOllama: vi.fn(),
  upsert: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

// Never the real Ollama on 11434: what comes back would depend on the developer's machine. The
// probes are mocked; the pure helpers (`sameOllamaTag`, the default host) are the real ones, so
// the card is tested against the matching it ships with.
vi.mock('../../utils/ollamaDetection', async (importOriginal) => ({
  ...(await importOriginal<typeof Detection>()),
  getOllamaSetup: mocks.getOllamaSetup,
  checkOllamaStatus: mocks.checkOllamaStatus,
  getOllamaModels: mocks.getOllamaModels,
  pullOllamaModel: mocks.pullOllamaModel,
  pollForOllama: mocks.pollForOllama,
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ upsert: mocks.upsert }),
}));

vi.mock('../../toasts', () => ({
  toastService: {
    success: (...args: unknown[]) => mocks.toastSuccess(...args),
    error: (...args: unknown[]) => mocks.toastError(...args),
  },
}));

const recommendations = (
  overrides: Partial<OllamaRecommendationsResponse> = {}
): OllamaRecommendationsResponse => ({
  models: [
    { tag: 'qwen3.8:27b', display_name: 'Qwen3.8 27B', approx_download_gb: 18, is_default: true },
    { tag: 'gemma4:31b', display_name: 'Gemma 4 31B', approx_download_gb: 19, is_default: false },
  ],
  default_model: 'qwen3.8:27b',
  min_memory_gib: 32,
  memory_gib: 64,
  memory_kind: 'apple_unified',
  suitability: 'suitable',
  suitability_message: 'This machine reports 64 GiB of unified memory.',
  base_url: 'http://localhost:11434/',
  host_configured: false,
  tier: 'private',
  ...overrides,
});

/** What `getOllamaSetup` derives: reachability from the host, privacy from the tier. */
const setup = (overrides: Partial<OllamaRecommendationsResponse> = {}, host?: string) => {
  const recs = recommendations(overrides);
  return {
    recommendations: recs,
    host: host ?? 'http://127.0.0.1:11434',
    reachable: !host || host.startsWith('http://127.0.0.1'),
    private: recs.tier === 'private',
  };
};

const tags = (...names: string[]) =>
  names.map((name) => ({ name, model: name, size: 1, digest: 'd', modified_at: '' }));

/**
 * The Model picker is the app's one select (`ui/Select`), as on Llama Server's panel: open it from
 * the keyboard and read its options, as the other shared-select tests do.
 */
function openModelMenu() {
  fireEvent.keyDown(screen.getByRole('combobox', { name: 'Model' }), {
    key: 'ArrowDown',
    code: 'ArrowDown',
  });
}

function optionLabels() {
  openModelMenu();
  return screen.getAllByRole('option').map((option) => option.textContent);
}

function chooseModel(label: RegExp) {
  openModelMenu();
  fireEvent.click(screen.getByRole('option', { name: label }));
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getOllamaSetup.mockResolvedValue(setup());
  mocks.checkOllamaStatus.mockImplementation(async (host: string) => ({ isRunning: true, host }));
  mocks.getOllamaModels.mockResolvedValue([]);
  mocks.pollForOllama.mockReturnValue(() => {});
  mocks.upsert.mockResolvedValue(undefined);
});

describe('OllamaInlineCard', () => {
  it('recommends Qwen3.8 27B, with Gemma 4 31B as the alternative, from the daemon', async () => {
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const field = await screen.findByTestId('ollama-model-select');
    expect(field).toHaveTextContent('Qwen3.8 27B (recommended) · ~18 GB · Needs download');
    expect(optionLabels()).toEqual([
      'Qwen3.8 27B (recommended) · ~18 GB · Needs download',
      'Gemma 4 31B · ~19 GB · Needs download',
    ]);
    expect(screen.getByTestId('ollama-download')).toHaveTextContent(ollamaCopy.downloadModel(18));
    // The tag onboarding used to hard-code is gone from every surface.
    expect(document.body.textContent).not.toMatch(/gpt-oss/);
  });

  it('is a combobox named by its label, with no native <select> in the card', async () => {
    const { container } = render(<OllamaInlineCard onSuccess={vi.fn()} />);
    const box = await screen.findByRole('combobox', { name: 'Model' });
    expect(box.id).toBe('ollama-model-select');
    expect(container.querySelector('select')).toBeNull();
  });

  it('preselects a recommended model the user already has', async () => {
    mocks.getOllamaModels.mockResolvedValue(tags('gemma4:31b', 'llama3.1:8b'));
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    expect(await screen.findByTestId('ollama-model-select')).toHaveTextContent(
      'Gemma 4 31B · ~19 GB · Installed'
    );
    expect(screen.getByTestId('ollama-connect')).toBeInTheDocument();
  });

  /**
   * The verdict is the daemon's; the words are the Llama Server rows' (a warning word, its reason
   * in an InfoTip, in GB). The daemon's GiB paragraph used to be printed as a warning box.
   */
  it('warns a machine below 32 GB, and still offers the default', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup({
        memory_gib: 16,
        suitability: 'below_minimum',
        suitability_message:
          'This machine reports 16 GiB of unified memory; the recommended local models need at least 32 GiB.',
      })
    );
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const warning = await screen.findByTestId('ollama-suitability');
    expect(warning).toHaveAttribute('role', 'note');
    expect(within(warning).getByText(localModelsCopy.belowMinimum(32))).toHaveClass(
      'text-text-warning'
    );
    const tip = within(warning).getByRole('button', {
      name: `About ${localModelsCopy.memoryLabel}`,
    });
    expect(tip).toHaveAccessibleDescription(
      localModelsCopy.ollamaFitHelp.belowMinimum('16 GB unified memory')
    );
    expect(document.body.textContent).not.toMatch(/GiB/);
    expect(screen.getByTestId('ollama-model-select')).toHaveTextContent('Qwen3.8 27B');
  });

  it('says unmeasured memory quietly, not as a warning', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup({ memory_gib: null, memory_kind: 'unknown_vram', suitability: 'unknown_resources' })
    );
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const line = await screen.findByTestId('ollama-suitability');
    expect(within(line).getByText('VRAM not detected')).not.toHaveClass('text-text-warning');
  });

  it('shows no warning on a suitable machine', async () => {
    render(<OllamaInlineCard onSuccess={vi.fn()} />);
    await screen.findByTestId('ollama-model-select');
    expect(screen.queryByTestId('ollama-suitability')).toBeNull();
  });

  it('probes the configured Ollama server, and names it', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup(
        { base_url: 'http://127.0.0.1:11500/', host_configured: true },
        'http://127.0.0.1:11500'
      )
    );
    mocks.checkOllamaStatus.mockImplementation(async (host: string) => ({
      isRunning: false,
      host,
    }));
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    expect(
      await screen.findByText(ollamaCopy.notDetected('http://127.0.0.1:11500'))
    ).toBeInTheDocument();
    expect(mocks.checkOllamaStatus).toHaveBeenCalledWith('http://127.0.0.1:11500');
  });

  /**
   * The renderer's CSP admits only loopback, so a probe of another machine's
   * Ollama is refused and used to read as "Ollama not detected" beside an
   * Install Ollama link, for a user whose server was working.
   */
  it('does not probe an Ollama on another computer, and lets the user connect to it', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup(
        {
          base_url: 'http://gpu-box.lab.example:11434/',
          host_configured: true,
          tier: 'public',
          memory_gib: 16,
          suitability: 'below_minimum',
          suitability_message: 'This machine reports 16 GiB of unified memory.',
        },
        'http://gpu-box.lab.example:11434'
      )
    );
    const onSuccess = vi.fn();
    render(<OllamaInlineCard onSuccess={onSuccess} />);

    const note = await screen.findByTestId('ollama-remote-note');
    // One instruction, its reason one hover or Tab away; the host is the status line's.
    expect(note).toHaveTextContent(ollamaCopy.remoteInstruction);
    expect(
      within(note).getByRole('button', { name: `About ${ollamaCopy.remoteTipLabel}` })
    ).toHaveAccessibleDescription(ollamaCopy.remoteTip);
    expect(
      screen.getByText(ollamaCopy.onAnotherComputer('http://gpu-box.lab.example:11434'))
    ).toBeInTheDocument();
    expect(note).toHaveTextContent('ollama pull qwen3.8:27b');
    expect(mocks.checkOllamaStatus).not.toHaveBeenCalled();
    expect(mocks.getOllamaModels).not.toHaveBeenCalled();
    expect(screen.queryByText(/not detected/)).toBeNull();
    expect(screen.queryByText(ollamaCopy.install)).toBeNull();
    // A download would go to THIS machine's Ollama, and the memory figure is
    // this machine's, not the server's.
    expect(screen.queryByTestId('ollama-download')).toBeNull();
    expect(screen.queryByTestId('ollama-suitability')).toBeNull();
    expect(optionLabels()).toEqual(['Qwen3.8 27B (recommended) · ~18 GB', 'Gemma 4 31B · ~19 GB']);

    fireEvent.click(screen.getByRole('option', { name: /Gemma 4 31B/ }));
    expect(note).toHaveTextContent('ollama pull gemma4:31b');
    fireEvent.click(screen.getByTestId('ollama-connect'));

    await waitFor(() => expect(onSuccess).toHaveBeenCalled());
    expect(mocks.upsert.mock.calls).toEqual([
      ['BIOROUTER_PROVIDER', 'ollama', false],
      ['BIOROUTER_MODEL', 'gemma4:31b', false],
    ]);
  });

  /** The reported bug: a failed pull toasted "Model Downloaded!". */
  it('reports a failed pull as a failure, in Ollama’s words', async () => {
    mocks.pullOllamaModel.mockResolvedValue({ ok: false, error: 'unexpected EOF' });
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    fireEvent.click(await screen.findByTestId('ollama-download'));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    expect(mocks.toastError.mock.calls[0][0].msg).toMatch(/qwen3\.8:27b: unexpected EOF/);
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    expect(mocks.pullOllamaModel).toHaveBeenCalledWith(
      'qwen3.8:27b',
      expect.any(Function),
      'http://127.0.0.1:11434'
    );
    // Still offered, not reported ready.
    expect(await screen.findByTestId('ollama-download')).toBeInTheDocument();
    expect(screen.queryByTestId('ollama-connect')).toBeNull();
  });

  it('offers Use Ollama once a pull is verified', async () => {
    mocks.pullOllamaModel.mockResolvedValue({ ok: true });
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const download = await screen.findByTestId('ollama-download');
    mocks.getOllamaModels.mockResolvedValue(tags('qwen3.8:27b'));
    fireEvent.click(download);

    expect(await screen.findByTestId('ollama-connect')).toBeInTheDocument();
    expect(mocks.toastSuccess).toHaveBeenCalledTimes(1);
    expect(mocks.toastSuccess.mock.calls[0][0]).toMatchObject({
      title: ollamaCopy.downloadedToast,
      msg: ollamaCopy.downloadedToastDetail('Qwen3.8 27B', 'qwen3.8:27b'),
    });
    expect(mocks.toastError).not.toHaveBeenCalled();
  });

  it('writes OLLAMA_HOST on connect only when it is unset', async () => {
    mocks.getOllamaModels.mockResolvedValue(tags('qwen3.8:27b'));
    const onSuccess = vi.fn();
    render(<OllamaInlineCard onSuccess={onSuccess} />);

    fireEvent.click(await screen.findByTestId('ollama-connect'));

    await waitFor(() => expect(onSuccess).toHaveBeenCalled());
    expect(mocks.upsert.mock.calls).toEqual([
      ['BIOROUTER_PROVIDER', 'ollama', false],
      ['BIOROUTER_MODEL', 'qwen3.8:27b', false],
      ['OLLAMA_HOST', 'localhost', false],
    ]);
  });

  it.each(['BIOROUTER_PROVIDER', 'BIOROUTER_MODEL', 'OLLAMA_HOST'])(
    'stops connecting when the card unmounts during the %s write',
    async (pendingKey) => {
      mocks.getOllamaModels.mockResolvedValue(tags('qwen3.8:27b'));
      let finishWrite!: () => void;
      mocks.upsert.mockImplementation((key: string) =>
        key === pendingKey
          ? new Promise<void>((resolve) => {
              finishWrite = resolve;
            })
          : Promise.resolve()
      );
      const onSuccess = vi.fn();
      const view = render(<OllamaInlineCard onSuccess={onSuccess} />);
      fireEvent.click(await screen.findByTestId('ollama-connect'));
      await waitFor(() =>
        expect(mocks.upsert).toHaveBeenCalledWith(pendingKey, expect.anything(), false)
      );
      const writesBeforeUnmount = mocks.upsert.mock.calls.length;

      view.unmount();
      await act(async () => {
        finishWrite();
      });

      expect(mocks.upsert).toHaveBeenCalledTimes(writesBeforeUnmount);
      expect(mocks.toastSuccess).not.toHaveBeenCalled();
      expect(onSuccess).not.toHaveBeenCalled();
    }
  );

  it('does not report a connection failure after the card unmounts', async () => {
    mocks.getOllamaModels.mockResolvedValue(tags('qwen3.8:27b'));
    let refuseWrite!: (error: Error) => void;
    mocks.upsert.mockImplementation(
      () =>
        new Promise<void>((_resolve, reject) => {
          refuseWrite = reject;
        })
    );
    const onSuccess = vi.fn();
    const view = render(<OllamaInlineCard onSuccess={onSuccess} />);
    fireEvent.click(await screen.findByTestId('ollama-connect'));
    await waitFor(() => expect(mocks.upsert).toHaveBeenCalledTimes(1));

    view.unmount();
    await act(async () => {
      refuseWrite(new Error('The daemon is offline'));
    });

    expect(mocks.upsert).toHaveBeenCalledTimes(1);
    expect(mocks.toastError).not.toHaveBeenCalled();
    expect(onSuccess).not.toHaveBeenCalled();
  });

  it('never replaces an OLLAMA_HOST the user configured', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup(
        { base_url: 'http://127.0.0.1:11500/', host_configured: true },
        'http://127.0.0.1:11500'
      )
    );
    mocks.getOllamaModels.mockResolvedValue(tags('qwen3.8:27b'));
    const onSuccess = vi.fn();
    render(<OllamaInlineCard onSuccess={onSuccess} />);

    fireEvent.click(await screen.findByTestId('ollama-connect'));

    await waitFor(() => expect(onSuccess).toHaveBeenCalled());
    expect(mocks.upsert.mock.calls.map(([key]) => key)).toEqual([
      'BIOROUTER_PROVIDER',
      'BIOROUTER_MODEL',
    ]);
  });

  it('says what went wrong when the daemon cannot answer, and retries', async () => {
    mocks.getOllamaSetup.mockRejectedValueOnce(
      new Error("Invalid Ollama host 'http://': no host name")
    );
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/Invalid Ollama host/);
    fireEvent.click(screen.getByRole('button', { name: ollamaCopy.tryAgain }));
    expect(await screen.findByTestId('ollama-model-select')).toBeInTheDocument();
  });

  /**
   * The known vitest flake: the mount effect's probes settled after the test
   * file's jsdom was torn down and set state, which threw `window is not
   * defined`. A probe that settles after unmount must end the chain there.
   */
  it('stops at the first probe that settles after unmount', async () => {
    let answer!: (value: unknown) => void;
    mocks.checkOllamaStatus.mockImplementation(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        })
    );
    const view = render(<OllamaInlineCard onSuccess={vi.fn()} />);
    await waitFor(() => expect(mocks.checkOllamaStatus).toHaveBeenCalled());

    view.unmount();
    await act(async () => {
      answer({ isRunning: true, host: 'http://127.0.0.1:11434' });
    });

    expect(mocks.getOllamaModels).not.toHaveBeenCalled();
  });

  /**
   * The card's header said "Run open-source models on your own machine. Free,
   * private, offline." over another computer's server, which the provider's
   * tier makes Public: a private chat cannot use it. The catalog's panel has
   * no header, so the body says it: the tier label, then the note.
   */
  it('never calls a public Ollama server private', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup(
        { base_url: 'http://gpu-box.lab.example:11434/', host_configured: true, tier: 'public' },
        'http://gpu-box.lab.example:11434'
      )
    );
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const note = await screen.findByTestId('ollama-public-note');
    expect(note).toHaveTextContent('public, not private');
    expect(note).toHaveTextContent('http://gpu-box.lab.example:11434');
    expect(note).toHaveAttribute('role', 'note');
    expect(document.body.textContent).not.toMatch(/Free, private, offline/);
    expect(document.body.textContent).not.toMatch(/your own machine/);
    // The status line says Public first: the row sits under Local, whose tier line says nothing
    // leaves this machine.
    const tier = screen.getByTestId('ollama-tier-label');
    expect(tier).toHaveTextContent(ollamaCopy.publicTier);
    expect(tier.closest('.br-status-pill')).toHaveTextContent(
      `${ollamaCopy.publicTier} · ${ollamaCopy.onAnotherComputer('http://gpu-box.lab.example:11434')}`
    );
  });

  /**
   * `OLLAMA_HOST=0.0.0.0` reaches this machine, so the card probes it through
   * 127.0.0.1 — but the tier's loopback rule calls it Public, and the card
   * used to say private over it because its own host list said "this computer".
   */
  it('probes a listen-everywhere host and still does not call it private', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup(
        { base_url: 'http://0.0.0.0:11434/', host_configured: true, tier: 'public' },
        'http://127.0.0.1:11434'
      )
    );
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    expect(await screen.findByTestId('ollama-model-select')).toBeInTheDocument();
    expect(mocks.checkOllamaStatus).toHaveBeenCalledWith('http://127.0.0.1:11434');
    expect(screen.getByTestId('ollama-public-note')).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/Free, private, offline/);
  });

  /**
   * The words name the server the user configured, never the address the renderer probes it at.
   * The note was handed the probe address, so `OLLAMA_HOST=0.0.0.0` read "treats the Ollama server
   * at http://127.0.0.1:11434 as public, not private: it counts only localhost, 127.0.0.1 and ::1
   * as this computer", which contradicts itself, over a status line that named no host at all.
   */
  it.each([
    ['running', true, ollamaCopy.running],
    ['not running', false, ollamaCopy.notDetected],
  ])(
    'names a listen-everywhere host as configured when Ollama is %s',
    async (_state, isRunning, words) => {
      mocks.getOllamaSetup.mockResolvedValue(
        setup(
          { base_url: 'http://0.0.0.0:11434/', host_configured: true, tier: 'public' },
          'http://127.0.0.1:11434'
        )
      );
      mocks.checkOllamaStatus.mockImplementation(async (host: string) => ({ isRunning, host }));
      render(<OllamaInlineCard onSuccess={vi.fn()} />);

      const note = await screen.findByTestId('ollama-public-note');
      expect(note).toHaveTextContent(ollamaCopy.publicNote('http://0.0.0.0:11434'));
      expect(note).not.toHaveTextContent('http://127.0.0.1:11434');
      expect(screen.getByTestId('ollama-tier-label').closest('.br-status-pill')).toHaveTextContent(
        `${ollamaCopy.publicTier} · ${words('http://0.0.0.0:11434')}`
      );
      expect(document.body.textContent).not.toMatch(/127\.0\.0\.1:11434/);
    }
  );

  it('does not name the default server on this computer', async () => {
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    expect(await screen.findByText(ollamaCopy.running())).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/localhost|127\.0\.0\.1/);
  });

  /**
   * `::1` is this computer by the tier (Private), but the CSP refuses it. It
   * used to read "Ollama on another computer", and lost its memory warning.
   */
  it('does not call an Ollama at ::1 another computer', async () => {
    mocks.getOllamaSetup.mockResolvedValue(
      setup(
        {
          base_url: 'http://[::1]:11434/',
          host_configured: true,
          tier: 'private',
          memory_gib: 16,
          suitability: 'below_minimum',
          suitability_message: 'This machine reports 16 GiB of unified memory.',
        },
        'http://[::1]:11434'
      )
    );
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    expect(await screen.findByTestId('ollama-remote-note')).toBeInTheDocument();
    expect(mocks.checkOllamaStatus).not.toHaveBeenCalled();
    expect(screen.getByText(ollamaCopy.atHost('http://[::1]:11434'))).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/another computer/);
    expect(screen.queryByTestId('ollama-public-note')).toBeNull();
    expect(screen.queryByTestId('ollama-tier-label')).toBeNull();
    // The memory figure is this machine's, and this is this machine's server.
    expect(screen.getByTestId('ollama-suitability')).toHaveTextContent(
      localModelsCopy.belowMinimum(32)
    );
  });

  it('makes no privacy claim before the daemon has answered', async () => {
    mocks.getOllamaSetup.mockImplementation(() => new Promise(() => {}));
    render(<OllamaInlineCard onSuccess={vi.fn()} />);
    expect(await screen.findByText(ollamaCopy.checking)).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/private|public/i);
  });

  it('keeps the user’s pick when a refresh of the pulled models lands', async () => {
    mocks.pullOllamaModel.mockResolvedValue({ ok: false, error: 'unexpected EOF' });
    render(<OllamaInlineCard onSuccess={vi.fn()} />);
    await screen.findByTestId('ollama-model-select');

    chooseModel(/Gemma 4 31B/);
    fireEvent.click(screen.getByTestId('ollama-download'));
    await waitFor(() => expect(mocks.getOllamaModels).toHaveBeenCalledTimes(2));

    expect(mocks.pullOllamaModel).toHaveBeenCalledWith(
      'gemma4:31b',
      expect.any(Function),
      'http://127.0.0.1:11434'
    );
    expect(screen.getByTestId('ollama-model-select')).toHaveTextContent('Gemma 4 31B');
  });
});

/**
 * Ollama's panel on the first-run standard (R-34; vision FR-S-8, FR-S-11): sentence-case toasts
 * with no exclamation mark, buttons that say what they do on the 32px rung and are never stretched,
 * and no machine id in a button's label.
 */
describe('OllamaInlineCard on the first-run standard', () => {
  it('offers to install Ollama when it is not running, as an ordinary button', async () => {
    mocks.checkOllamaStatus.mockImplementation(async (host: string) => ({
      isRunning: false,
      host,
    }));
    render(<OllamaInlineCard onSuccess={vi.fn()} />);
    const install = await screen.findByRole('link', { name: ollamaCopy.install });
    expect(install.className).not.toContain('w-full');
    expect(install.className).not.toContain('h-control-lg');
    expect(screen.getByText(ollamaCopy.notDetected())).toBeInTheDocument();
  });

  it('says what a download is without naming the model id on the button', async () => {
    render(<OllamaInlineCard onSuccess={vi.fn()} />);
    const download = await screen.findByRole('button', { name: ollamaCopy.downloadModel(18) });
    expect(download).not.toHaveTextContent('qwen3.8');
    expect(download.className).not.toContain('w-full');
    // The model is named in the picker, by name; whether it is pulled is its option's word there.
    // The status says only that the server answers, with no tag (principles 4 and 10).
    expect(screen.getByText(ollamaCopy.running())).toBeInTheDocument();
    expect(screen.queryByText(/qwen3\.8:27b/)).toBeNull();
  });

  it('connects with a plain, sentence-case toast', async () => {
    mocks.getOllamaModels.mockResolvedValue(tags('qwen3.8:27b'));
    const onSuccess = vi.fn();
    render(<OllamaInlineCard onSuccess={onSuccess} />);
    expect(await screen.findByText(ollamaCopy.running())).toBeInTheDocument();
    expect(await screen.findByTestId('ollama-model-select')).toHaveTextContent(
      'Qwen3.8 27B (recommended) · ~18 GB · Installed'
    );
    fireEvent.click(screen.getByRole('button', { name: ollamaCopy.use }));
    await waitFor(() => expect(onSuccess).toHaveBeenCalled());
    const [toast] = mocks.toastSuccess.mock.calls[0] as [{ title: string; msg: string }];
    expect(toast.title).toBe(ollamaCopy.connectedToast);
    // The model by its name, never its tag.
    expect(toast.msg).toBe(ollamaCopy.connectedToastDetail('Qwen3.8 27B'));
    for (const title of [
      ollamaCopy.connectedToast,
      ollamaCopy.detectedToast,
      ollamaCopy.downloadedToast,
      ollamaCopy.downloadFailed,
      ollamaCopy.connectFailed,
    ]) {
      expect(title).not.toMatch(/!/);
    }
  });
});
