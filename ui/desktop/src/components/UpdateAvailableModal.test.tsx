import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent, act, cleanup, waitFor } from '@testing-library/react';
import UpdateAvailableModal from './UpdateAvailableModal';
import type { UpdaterEventPayload, UpdaterState } from '../utils/updaterState';
import { OPEN_UPDATE_MODAL_EVENT } from '../utils/updateUiEvents';

// Capture the callback the component registers via onUpdaterEvent so the test
// can push real updater events at it, then assert on the rendered UI.
let emit: (p: UpdaterEventPayload) => void = () => {};
let installUpdate: ReturnType<typeof vi.fn>;
let openExternal: ReturnType<typeof vi.fn>;

function emitAct(p: UpdaterEventPayload) {
  act(() => emit(p));
}

function requestModal(state: UpdaterState) {
  act(() => {
    window.dispatchEvent(new CustomEvent(OPEN_UPDATE_MODAL_EVENT, { detail: state }));
  });
}

beforeEach(() => {
  localStorage.clear();
  installUpdate = vi.fn();
  openExternal = vi.fn().mockResolvedValue(undefined);
  const electron = {
    getVersion: () => '1.85.4',
    onUpdaterEvent: (cb: (p: UpdaterEventPayload) => void) => {
      emit = cb;
      return () => {
        emit = () => {};
      };
    },
    getUpdateState: vi.fn().mockResolvedValue(null),
    installUpdate,
    openExternal,
  };
  (window as unknown as { electron: unknown }).electron = electron;
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('UpdateAvailableModal — one-click flow', () => {
  it('renders nothing until an update event arrives', () => {
    const { container } = render(<UpdateAvailableModal />);
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText(/Restart & Update/i)).toBeNull();
  });

  it('shows a progress bar while downloading, then a one-click Restart & Update', async () => {
    render(<UpdateAvailableModal />);

    emitAct({ event: 'update-available', data: { version: '1.86.0' } });
    emitAct({ event: 'download-progress', data: { percent: 47 } });
    expect(screen.queryByText(/Downloading update/i)).toBeNull();
    requestModal({
      phase: 'available',
      latestVersion: '1.86.0',
      percent: 47,
      usingFallback: false,
    });

    // Downloading state: progress shown, no install button yet.
    expect(await screen.findByText(/Downloading update/i)).toBeTruthy();
    expect(screen.getByText('47%')).toBeTruthy();
    expect(screen.getByText('1.86.0')).toBeTruthy(); // new version row
    expect(screen.queryByRole('button', { name: /Restart & Update/i })).toBeNull();

    // Download completes → one-click install button appears.
    emitAct({ event: 'update-downloaded', data: { version: '1.86.0' } });
    const installBtn = await screen.findByRole('button', { name: /Restart & Update/i });
    expect(screen.getByText(/Update ready to install/i)).toBeTruthy();

    fireEvent.click(installBtn);
    expect(installUpdate).toHaveBeenCalledTimes(1);
  });

  it('recovers a download already finished before mount via getUpdateState', async () => {
    (
      window as unknown as { electron: { getUpdateState: ReturnType<typeof vi.fn> } }
    ).electron.getUpdateState = vi.fn().mockResolvedValue({
      updateAvailable: true,
      status: 'downloaded',
      latestVersion: '1.86.0',
      percent: 100,
    });

    render(<UpdateAvailableModal />);
    expect(screen.queryByRole('button', { name: /Restart & Update/i })).toBeNull();
    requestModal({
      phase: 'downloaded',
      latestVersion: '1.86.0',
      percent: 100,
      usingFallback: false,
    });
    const installBtn = await screen.findByRole('button', { name: /Restart & Update/i });
    fireEvent.click(installBtn);
    expect(installUpdate).toHaveBeenCalledTimes(1);
  });

  it('"Later" dismisses and records per-version dismissal', async () => {
    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-downloaded', data: { version: '1.86.0' } });
    requestModal({
      phase: 'downloaded',
      latestVersion: '1.86.0',
      percent: 100,
      usingFallback: false,
    });

    const laterBtn = await screen.findByRole('button', { name: /^Later$/i });
    fireEvent.click(laterBtn);
    expect(localStorage.getItem('biorouter:update-modal-dismissed-version')).toBe('1.86.0');
    expect(installUpdate).not.toHaveBeenCalled();
  });

  it('shows an error state without a fake install button', async () => {
    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-available', data: { version: '1.86.0' } });
    emitAct({ event: 'error', data: 'network unreachable' });
    requestModal({
      phase: 'error',
      latestVersion: '1.86.0',
      percent: 0,
      usingFallback: false,
      error: 'network unreachable',
    });

    expect(await screen.findByText(/Update download failed/i)).toBeTruthy();
    expect(screen.getByText(/network unreachable/i)).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Restart & Update/i })).toBeNull();
  });

  it('a background error after download does not hide the install button', async () => {
    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-downloaded', data: { version: '1.86.0' } });
    requestModal({
      phase: 'downloaded',
      latestVersion: '1.86.0',
      percent: 100,
      usingFallback: false,
    });
    await screen.findByRole('button', { name: /Restart & Update/i });

    emitAct({ event: 'error', data: 'late blip' });
    expect(screen.getByRole('button', { name: /Restart & Update/i })).toBeTruthy();
  });

  it('does not interrupt the user when an update is detected', async () => {
    localStorage.setItem('biorouter:update-modal-dismissed-version', '1.86.0');
    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-available', data: { version: '1.86.0' } });
    await waitFor(() => {
      expect(screen.queryByText(/Downloading update/i)).toBeNull();
    });

    requestModal({
      phase: 'available',
      latestVersion: '1.86.0',
      percent: 0,
      usingFallback: false,
    });
    expect(await screen.findByText(/Downloading update/i)).toBeTruthy();
  });

  it('reopens a dismissed update when the sidebar button requests it', async () => {
    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-downloaded', data: { version: '1.86.0' } });
    requestModal({
      phase: 'downloaded',
      latestVersion: '1.86.0',
      percent: 100,
      usingFallback: false,
    });

    fireEvent.click(await screen.findByRole('button', { name: /^Later$/i }));
    requestModal({
      phase: 'downloaded',
      latestVersion: '1.86.0',
      percent: 100,
      usingFallback: false,
    });

    expect(await screen.findByRole('button', { name: /Restart & Update/i })).toBeTruthy();
  });
});

describe('UpdateAvailableModal — assisted (non-mac) download', () => {
  /**
   * The Windows bug in one test.
   *
   * On Windows there is no electron-updater manifest, so the GitHub fallback
   * takes over — and that fallback deliberately does NOT download, waiting for
   * the user via the `download-update` IPC. Nothing in the renderer ever called
   * that IPC, so the modal announced "downloading in the background" and parked
   * a progress bar at 0% forever, with a permanently disabled "Preparing…"
   * button underneath.
   */
  it('offers a working Download button instead of a progress bar that never moves', async () => {
    const downloadUpdate = vi.fn().mockResolvedValue({ success: true, error: null });
    (window as unknown as { electron: Record<string, unknown> }).electron.downloadUpdate =
      downloadUpdate;

    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-available', data: { version: '1.86.0' }, usingFallback: true });
    requestModal({
      phase: 'available',
      latestVersion: '1.86.0',
      percent: 0,
      usingFallback: true,
    });

    // It must NOT claim a download is already running.
    expect(await screen.findByText(/Update available/i)).toBeTruthy();
    expect(screen.queryByText(/downloading in the background/i)).toBeNull();
    expect(screen.queryByText(/Preparing/i)).toBeNull();

    // It must offer an enabled control that actually starts the download.
    const button = await screen.findByRole('button', { name: /Download update/i });
    expect((button as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(button);
    await waitFor(() => expect(downloadUpdate).toHaveBeenCalledTimes(1));
  });

  /** On macOS electron-updater really is downloading, so the old copy stands. */
  it('keeps the background-download copy when electron-updater is doing the work', async () => {
    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-available', data: { version: '1.86.0' }, usingFallback: false });
    requestModal({ phase: 'available', latestVersion: '1.86.0', percent: 0, usingFallback: false });

    expect(await screen.findByText(/downloading in the background/i)).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Download update/i })).toBeNull();
  });

  /** Once bytes are moving, progress is real again even in assisted mode. */
  it('switches to the progress bar once the assisted download starts', async () => {
    render(<UpdateAvailableModal />);
    emitAct({ event: 'update-available', data: { version: '1.86.0' }, usingFallback: true });
    emitAct({ event: 'download-progress', data: { percent: 12 }, usingFallback: true });
    requestModal({
      phase: 'available',
      latestVersion: '1.86.0',
      percent: 12,
      usingFallback: true,
    });

    expect(await screen.findByText('12%')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Download update/i })).toBeNull();
  });
});
