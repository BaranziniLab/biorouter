import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import OllamaInlineCard from './OllamaInlineCard';
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

// The probes are mocked; the pure helpers (`sameOllamaTag`, the default host)
// are the real ones, so the card is tested against the matching it ships with.
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

describe('OllamaInlineCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getOllamaSetup.mockResolvedValue(setup());
    mocks.checkOllamaStatus.mockImplementation(async (host: string) => ({ isRunning: true, host }));
    mocks.getOllamaModels.mockResolvedValue([]);
    mocks.pollForOllama.mockReturnValue(() => {});
    mocks.upsert.mockResolvedValue(undefined);
  });

  it('recommends Qwen3.8 27B, with Gemma 4 31B as the alternative, from the daemon', async () => {
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const select = await screen.findByTestId('ollama-model-select');
    expect(select).toHaveValue('qwen3.8:27b');
    const options = Array.from(select.querySelectorAll('option')).map((o) => o.textContent);
    expect(options).toEqual([
      'Qwen3.8 27B (recommended) · ~18 GB · not installed',
      'Gemma 4 31B · ~19 GB · not installed',
    ]);
    expect(screen.getByTestId('ollama-download')).toHaveTextContent(
      'Download Qwen3.8 27B (~18 GB)'
    );
    // The tag onboarding used to hard-code is gone from every surface.
    expect(document.body.textContent).not.toMatch(/gpt-oss/);
  });

  it('preselects a recommended model the user already has', async () => {
    mocks.getOllamaModels.mockResolvedValue(tags('gemma4:31b', 'llama3.1:8b'));
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    expect(await screen.findByTestId('ollama-model-select')).toHaveValue('gemma4:31b');
    expect(screen.getByTestId('ollama-connect')).toBeInTheDocument();
  });

  it('warns a machine below 32 GiB, and still offers the default', async () => {
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
    expect(warning).toHaveTextContent('at least 32 GiB');
    expect(warning).toHaveAttribute('role', 'note');
    expect(screen.getByTestId('ollama-model-select')).toHaveValue('qwen3.8:27b');
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
      await screen.findByText(/Ollama not detected at http:\/\/127\.0\.0\.1:11500/)
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
    expect(note).toHaveTextContent('http://gpu-box.lab.example:11434');
    expect(note).toHaveTextContent('ollama pull qwen3.8:27b');
    expect(mocks.checkOllamaStatus).not.toHaveBeenCalled();
    expect(mocks.getOllamaModels).not.toHaveBeenCalled();
    expect(screen.queryByText(/not detected/)).toBeNull();
    expect(screen.queryByText('Install Ollama')).toBeNull();
    // A download would go to THIS machine's Ollama, and the memory figure is
    // this machine's, not the server's.
    expect(screen.queryByTestId('ollama-download')).toBeNull();
    expect(screen.queryByTestId('ollama-suitability')).toBeNull();
    const options = Array.from(
      screen.getByTestId('ollama-model-select').querySelectorAll('option')
    ).map((o) => o.textContent);
    expect(options).toEqual(['Qwen3.8 27B (recommended) · ~18 GB', 'Gemma 4 31B · ~19 GB']);

    fireEvent.change(screen.getByTestId('ollama-model-select'), {
      target: { value: 'gemma4:31b' },
    });
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

  it('offers Connect once a pull is verified', async () => {
    mocks.pullOllamaModel.mockResolvedValue({ ok: true });
    render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const download = await screen.findByTestId('ollama-download');
    mocks.getOllamaModels.mockResolvedValue(tags('qwen3.8:27b'));
    fireEvent.click(download);

    expect(await screen.findByTestId('ollama-connect')).toBeInTheDocument();
    expect(mocks.toastSuccess).toHaveBeenCalledTimes(1);
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
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
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
   * The header said "Run open-source models on your own machine. Free,
   * private, offline." over another computer's server, which the provider's
   * tier makes Public: a private chat cannot use it. And the catalog mounts
   * the card bare, with no header, so the body has to say it too.
   */
  it('never calls a public Ollama server private, with or without its header', async () => {
    const remote = setup(
      { base_url: 'http://gpu-box.lab.example:11434/', host_configured: true, tier: 'public' },
      'http://gpu-box.lab.example:11434'
    );
    mocks.getOllamaSetup.mockResolvedValue(remote);
    const view = render(<OllamaInlineCard onSuccess={vi.fn()} />);

    const note = await screen.findByTestId('ollama-public-note');
    expect(note).toHaveTextContent('public, not private');
    expect(note).toHaveTextContent('http://gpu-box.lab.example:11434');
    expect(document.body.textContent).not.toMatch(/Free, private, offline/);
    expect(document.body.textContent).not.toMatch(/your own machine/);
    expect(screen.getByText('Public · Ollama server')).toBeInTheDocument();
    view.unmount();

    render(<OllamaInlineCard onSuccess={vi.fn()} chrome="bare" />);
    expect(await screen.findByTestId('ollama-public-note')).toBeInTheDocument();
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
    expect(screen.getByText('Ollama at http://[::1]:11434')).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/another computer/);
    expect(screen.queryByTestId('ollama-public-note')).toBeNull();
    // The memory figure is this machine's, and this is this machine's server.
    expect(screen.getByTestId('ollama-suitability')).toHaveTextContent('16 GiB');
    expect(screen.getByText(/Free, private, offline/)).toBeInTheDocument();
  });

  it('makes no privacy claim before the daemon has answered', async () => {
    mocks.getOllamaSetup.mockImplementation(() => new Promise(() => {}));
    render(<OllamaInlineCard onSuccess={vi.fn()} />);
    expect(await screen.findByText('Checking for Ollama…')).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/private/);
  });
});
