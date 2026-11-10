/**
 * The install panel: it offers a local decision model, installs it through the
 * daemon and polls while the install runs, joins one that is running, and says
 * the daemon's own sentence when Ollama is stopped, elsewhere, or the install
 * failed. Nothing is installed unless the person presses Install.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DecisionRequestError,
  type DecisionInstallJob,
  type DecisionInstallStatus,
} from '../../utils/sessionDecisions';
import {
  DECISION_INSTALL_COPY,
  DecisionModelInstall,
  OLLAMA_DOWNLOAD_URL,
  formatInstallBytes,
} from './DecisionModelInstall';

const mocks = vi.hoisted(() => ({ fetch: vi.fn(), start: vi.fn(), open: vi.fn() }));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  fetchDecisionInstall: mocks.fetch,
  startDecisionInstall: mocks.start,
}));
vi.mock('../../utils/openInBrowser', () => ({ openInBrowser: mocks.open }));

function status(overrides: Partial<DecisionInstallStatus> = {}): DecisionInstallStatus {
  return {
    offers: [
      {
        model: 'tev1:0.8b',
        displayName: 'tev1:0.8b',
        approxSize: 'about 1.5 GB',
        note: 'small and fast',
        isDefault: true,
        installed: false,
      },
      {
        model: 'clef-flash:latest',
        displayName: 'clef-flash',
        approxSize: 'about 12 GB',
        note: 'larger, also reads images',
        isDefault: false,
        installed: false,
      },
    ],
    ollama: {
      baseUrl: 'http://127.0.0.1:11434/',
      state: 'running',
      version: '0.40.15',
      message: null,
    },
    job: null,
    ...overrides,
  };
}

function job(state: DecisionInstallJob['state'], extra: Partial<DecisionInstallJob> = {}) {
  return {
    model: 'tev1:0.8b',
    state,
    completedBytes: 0,
    totalBytes: 0,
    status: null,
    error: null,
    ...extra,
  } satisfies DecisionInstallJob;
}

const read = (value: DecisionInstallStatus) => ({ data: value, missing: false });

beforeEach(() => {
  mocks.fetch.mockReset().mockResolvedValue(read(status()));
  mocks.start.mockReset();
  mocks.open.mockReset();
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('DecisionModelInstall', () => {
  it('opens with why no model is usable: a private chat, a Crew chat, or none at all', async () => {
    const { rerender } = render(<DecisionModelInstall reason="private" pollIntervalMs={5} />);
    expect(screen.getByTestId('decision-install-why')).toHaveTextContent(
      'This chat is private, so only a decision model on this computer may read it, and none is installed.'
    );
    expect(await screen.findByTestId('decision-install-start')).toHaveTextContent(
      'Install tev1:0.8b'
    );
    // What the model is sits in the InfoTip after Install, with the download's size.
    const about = screen.getByRole('button', { name: 'About tev1:0.8b' });
    expect(about).toHaveAccessibleDescription('About 1.5 GB, runs in Ollama on this computer.');
    expect(screen.queryByText(/is a small decision model/)).toBeNull();
    rerender(<DecisionModelInstall reason="crew" pollIntervalMs={5} />);
    expect(screen.getByTestId('decision-install-why')).toHaveTextContent(
      'This chat works under a Crew grant, so only a decision model on this computer may read it, and none is installed.'
    );
    rerender(<DecisionModelInstall reason="none" pollIntervalMs={5} />);
    expect(screen.getByTestId('decision-install-why')).toHaveTextContent(
      'No decision model is installed and no Baseten key is set.'
    );
  });

  it('puts nothing beside the Install button but the InfoTip glyph, and keeps its name', async () => {
    render(<DecisionModelInstall reason="private" pollIntervalMs={5} />);
    const install = await screen.findByRole('button', { name: 'Install tev1:0.8b' });
    const row = install.parentElement!;
    expect(Array.from(row.children).map((child) => child.getAttribute('data-slot'))).toEqual([
      'button',
      'info-tip-trigger',
      'info-tip-description',
    ]);
    // The hidden help carries the size, in the offer's own figure, said "About" once.
    expect(within(row).getByText('About 1.5 GB, runs in Ollama on this computer.')).toHaveClass(
      'sr-only'
    );
  });

  it('in a browser, names the computer Biorouter runs on, not this one', async () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    render(<DecisionModelInstall reason="private" pollIntervalMs={5} />);
    expect(screen.getByTestId('decision-install-why')).toHaveTextContent(
      'only a decision model on the computer Biorouter runs on may read it'
    );
    expect(
      await screen.findByRole('button', { name: 'About tev1:0.8b' })
    ).toHaveAccessibleDescription(/runs in Ollama on the computer Biorouter runs on\.$/);
  });

  it('installs only when pressed: POSTs the default model, polls the progress, and says when it is done', async () => {
    const onInstalled = vi.fn();
    render(<DecisionModelInstall reason="private" onInstalled={onInstalled} pollIntervalMs={5} />);
    const install = await screen.findByTestId('decision-install-start');
    expect(mocks.start).not.toHaveBeenCalled();

    mocks.start.mockResolvedValue(status({ job: job('downloading') }));
    mocks.fetch
      .mockResolvedValueOnce(
        read(
          status({
            job: job('downloading', { completedBytes: 412_000_000, totalBytes: 1_500_000_000 }),
          })
        )
      )
      .mockResolvedValueOnce(read(status({ job: job('verifying') })))
      .mockResolvedValue(
        read(
          status({
            job: job('installed'),
            offers: status().offers.map((offer, i) => ({ ...offer, installed: i === 0 })),
          })
        )
      );
    fireEvent.click(install);
    expect(mocks.start).toHaveBeenCalledWith('tev1:0.8b');

    expect(await screen.findByText('Downloading tev1:0.8b… 412 MB of 1.5 GB')).toBeInTheDocument();
    const bar = screen.getByRole('progressbar');
    expect(bar).toHaveAttribute('aria-valuenow', '412000000');
    expect(bar).toHaveAttribute('aria-valuemax', '1500000000');
    expect(
      await screen.findByText('Checking that Ollama answers decisions with it…')
    ).toBeInTheDocument();
    expect(await screen.findByTestId('decision-install-done')).toHaveTextContent(
      'tev1:0.8b is installed.'
    );
    await waitFor(() => expect(onInstalled).toHaveBeenCalledTimes(1));
    // It stops reading once the install is over.
    const reads = mocks.fetch.mock.calls.length;
    await act(() => new Promise((resolve) => setTimeout(resolve, 40)));
    expect(mocks.fetch.mock.calls.length).toBe(reads);
    expect(onInstalled).toHaveBeenCalledTimes(1);
  });

  it('joins an install that is running when a second start is refused as busy', async () => {
    const onInstalled = vi.fn();
    render(<DecisionModelInstall onInstalled={onInstalled} pollIntervalMs={5} />);
    const install = await screen.findByTestId('decision-install-start');
    const busy = 'tev1:0.8b is still downloading. Wait for it to finish, then try again.';
    mocks.start.mockRejectedValue(new DecisionRequestError(busy, 409, busy));
    mocks.fetch
      .mockResolvedValueOnce(read(status({ job: job('downloading', { completedBytes: 1 }) })))
      .mockResolvedValue(read(status({ job: job('installed') })));
    fireEvent.click(install);
    expect(await screen.findByTestId('decision-install-done')).toBeInTheDocument();
    expect(screen.queryByTestId('decision-install-error')).toBeNull();
    await waitFor(() => expect(onInstalled).toHaveBeenCalledTimes(1));
  });

  it('shows an install that is already running when it opens, and its size only once known', async () => {
    mocks.fetch
      .mockResolvedValueOnce(read(status({ job: job('downloading') })))
      .mockReturnValue(new Promise(() => {}));
    render(<DecisionModelInstall pollIntervalMs={5} />);
    expect(await screen.findByText('Downloading tev1:0.8b…')).toBeInTheDocument();
    expect(screen.getByRole('progressbar')).not.toHaveAttribute('aria-valuenow');
  });

  it('says a failed install in the daemon’s words, and Retry starts it again', async () => {
    const failed = 'Ollama could not download `tev1:0.8b`: no space left on device.';
    mocks.fetch.mockResolvedValue(read(status({ job: job('failed', { error: failed }) })));
    render(<DecisionModelInstall pollIntervalMs={5} />);
    expect(await screen.findByTestId('decision-install-error')).toHaveTextContent(failed);
    mocks.start.mockResolvedValue(status({ job: job('downloading') }));
    fireEvent.click(screen.getByTestId('decision-install-retry'));
    expect(mocks.start).toHaveBeenCalledWith('tev1:0.8b');
    expect(await screen.findByText('Downloading tev1:0.8b…')).toBeInTheDocument();
  });

  it('does not say an older failure once the model is installed, quiet or not', async () => {
    // The daemon keeps a failed job until the next install starts; the model was
    // installed since from a terminal (`ollama pull tev1:0.8b`).
    const failed = 'Ollama could not download `tev1:0.8b`: connection reset.';
    mocks.fetch.mockResolvedValue(
      read(
        status({
          job: job('failed', { error: failed }),
          offers: status().offers.map((offer, i) => ({ ...offer, installed: i === 0 })),
        })
      )
    );
    const quiet = render(<DecisionModelInstall reason="private" quiet pollIntervalMs={5} />);
    await waitFor(() => expect(mocks.fetch).toHaveBeenCalledTimes(1));
    await act(() => Promise.resolve());
    expect(quiet.container).toBeEmptyDOMElement();
    quiet.unmount();

    const onInstalled = vi.fn();
    render(<DecisionModelInstall reason="private" onInstalled={onInstalled} pollIntervalMs={5} />);
    expect(await screen.findByTestId('decision-install-done')).toHaveTextContent(
      'tev1:0.8b is installed.'
    );
    expect(screen.queryByTestId('decision-install-error')).toBeNull();
    expect(screen.queryByTestId('decision-install-retry')).toBeNull();
    await waitFor(() => expect(onInstalled).toHaveBeenCalledTimes(1));
  });

  it('still says a failure it watched, even when the model is listed as installed', async () => {
    const onInstalled = vi.fn();
    render(<DecisionModelInstall onInstalled={onInstalled} pollIntervalMs={5} />);
    const install = await screen.findByTestId('decision-install-start');
    // Pulled, then the check that Ollama answers decisions with it failed.
    const failed = 'Ollama answered `tev1:0.8b` without the probabilities a decision needs.';
    mocks.start.mockResolvedValue(status({ job: job('verifying') }));
    mocks.fetch.mockResolvedValue(
      read(
        status({
          job: job('failed', { error: failed }),
          offers: status().offers.map((offer, i) => ({ ...offer, installed: i === 0 })),
        })
      )
    );
    fireEvent.click(install);
    expect(await screen.findByTestId('decision-install-error')).toHaveTextContent(failed);
    expect(screen.getByTestId('decision-install-retry')).toBeInTheDocument();
    expect(onInstalled).not.toHaveBeenCalled();
  });

  it('says a refused start in the daemon’s words when the status does not explain it', async () => {
    render(<DecisionModelInstall pollIntervalMs={5} />);
    const install = await screen.findByTestId('decision-install-start');
    const refused = '`x` is not a decision model Biorouter installs.';
    mocks.start.mockRejectedValue(new DecisionRequestError(refused, 400, refused));
    fireEvent.click(install);
    expect(await screen.findByTestId('decision-install-error')).toHaveTextContent(refused);
    expect(screen.getByTestId('decision-install-retry')).toBeInTheDocument();
  });

  it('with Ollama stopped, says so and offers Get Ollama and Check again', async () => {
    const stopped =
      'Ollama is not running at http://127.0.0.1:11434/. Install it from https://ollama.com/download or start it, then try again.';
    mocks.fetch.mockResolvedValueOnce(
      read(
        status({ ollama: { baseUrl: null, state: 'not_running', version: null, message: stopped } })
      )
    );
    render(<DecisionModelInstall reason="private" pollIntervalMs={5} />);
    expect(await screen.findByText(stopped)).toBeInTheDocument();
    expect(screen.queryByTestId('decision-install-start')).toBeNull();
    fireEvent.click(screen.getByTestId('decision-install-get-ollama'));
    expect(mocks.open).toHaveBeenCalledWith(OLLAMA_DOWNLOAD_URL);
    fireEvent.click(screen.getByTestId('decision-install-check-again'));
    expect(await screen.findByTestId('decision-install-start')).toBeInTheDocument();
    expect(mocks.fetch).toHaveBeenCalledTimes(2);
  });

  it('with Ollama on another computer, says so and offers no button', async () => {
    const elsewhere =
      'Biorouter installs decision models only into an Ollama on this computer, and OLLAMA_HOST is http://10.255.255.1:11434/.';
    mocks.fetch.mockResolvedValue(
      read(
        status({
          ollama: { baseUrl: null, state: 'not_this_computer', version: null, message: elsewhere },
        })
      )
    );
    render(<DecisionModelInstall reason="private" pollIntervalMs={5} />);
    expect(await screen.findByTestId('decision-install-elsewhere')).toHaveTextContent(elsewhere);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('tells the caller to read its list again when the model is installed already', async () => {
    const onInstalled = vi.fn();
    mocks.fetch.mockResolvedValue(
      read(
        status({ offers: status().offers.map((offer, i) => ({ ...offer, installed: i === 0 })) })
      )
    );
    render(<DecisionModelInstall onInstalled={onInstalled} pollIntervalMs={5} />);
    expect(await screen.findByTestId('decision-install-done')).toHaveTextContent(
      'tev1:0.8b is installed.'
    );
    await waitFor(() => expect(onInstalled).toHaveBeenCalledTimes(1));
  });

  it('quiet, says nothing until there is something to do', async () => {
    mocks.fetch.mockResolvedValue(
      read(
        status({ offers: status().offers.map((offer, i) => ({ ...offer, installed: i === 0 })) })
      )
    );
    const onInstalled = vi.fn();
    const { container, rerender } = render(
      <DecisionModelInstall quiet onInstalled={onInstalled} pollIntervalMs={5} />
    );
    await waitFor(() => expect(mocks.fetch).toHaveBeenCalled());
    await act(() => Promise.resolve());
    expect(container).toBeEmptyDOMElement();
    expect(onInstalled).not.toHaveBeenCalled();

    mocks.fetch.mockResolvedValue(read(status()));
    rerender(<DecisionModelInstall key="again" quiet pollIntervalMs={5} />);
    expect(await screen.findByTestId('decision-install-start')).toBeInTheDocument();
  });

  it('sizes bytes in decimal units, as Ollama and the offer say them', () => {
    expect(formatInstallBytes(412_000_000)).toBe('412 MB');
    expect(formatInstallBytes(1_500_000_000)).toBe('1.5 GB');
    expect(formatInstallBytes(0)).toBe('0 MB');
    expect(DECISION_INSTALL_COPY.downloaded('tev1:0.8b', '412 MB', '1.5 GB')).toBe(
      'Downloading tev1:0.8b… 412 MB of 1.5 GB'
    );
  });
});
