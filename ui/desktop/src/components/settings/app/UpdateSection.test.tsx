import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, fireEvent, act, cleanup } from '@testing-library/react';
import UpdateSection from './UpdateSection';
import { aboutCopy } from './copy';
import type { UpdaterEventPayload } from '../../../utils/updaterState';

let emit: (p: UpdaterEventPayload) => void = () => {};
let installUpdate: ReturnType<typeof vi.fn>;
let checkForUpdates: ReturnType<typeof vi.fn>;

function emitAct(p: UpdaterEventPayload) {
  act(() => emit(p));
}

beforeEach(() => {
  installUpdate = vi.fn();
  checkForUpdates = vi.fn().mockResolvedValue({ updateInfo: {}, error: null });
  (window as unknown as { electron: unknown }).electron = {
    getVersion: () => '1.85.4',
    onUpdaterEvent: (cb: (p: UpdaterEventPayload) => void) => {
      emit = cb;
      return () => {
        emit = () => {};
      };
    },
    getUpdateState: vi.fn().mockResolvedValue(null),
    checkForUpdates,
    installUpdate,
  };
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Settings > App > About > Version: one-click flow', () => {
  it('shows the current version as the Version row’s value', () => {
    render(<UpdateSection />);
    expect(screen.getByText('1.85.4')).toBeTruthy();
    expect(screen.getByText(aboutCopy.version)).toBeTruthy();
  });

  it('drives electron-updater check, shows progress, then one-click install', async () => {
    render(<UpdateSection />);

    fireEvent.click(screen.getByRole('button', { name: /Check for Updates/i }));
    expect(checkForUpdates).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/Checking for updates/i)).toBeTruthy();

    emitAct({ event: 'update-available', data: { version: '1.86.0' } });
    emitAct({ event: 'download-progress', data: { percent: 60 } });
    expect(await screen.findByText(/Downloading 1\.86\.0/i)).toBeTruthy();
    expect(screen.getByText('60%')).toBeTruthy();

    emitAct({ event: 'update-downloaded', data: { version: '1.86.0' } });
    // "Restart to update" replaces the button; the version moves to the status line (§3.13).
    const installBtn = await screen.findByRole('button', { name: aboutCopy.restartToUpdate });
    expect(screen.getByText(aboutCopy.ready('1.86.0'))).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Check for Updates/i })).toBeNull();
    fireEvent.click(installBtn);
    expect(installUpdate).toHaveBeenCalledTimes(1);
  });

  it('reports up-to-date when no newer release', async () => {
    render(<UpdateSection />);
    fireEvent.click(screen.getByRole('button', { name: /Check for Updates/i }));
    emitAct({ event: 'update-not-available' });
    expect(await screen.findByText(/up to date/i)).toBeTruthy();
  });

  it('surfaces a check error', async () => {
    checkForUpdates.mockResolvedValue({ updateInfo: null, error: 'offline' });
    render(<UpdateSection />);
    fireEvent.click(screen.getByRole('button', { name: /Check for Updates/i }));
    expect(await screen.findByText(/Could not complete the update/i)).toBeTruthy();
    // The failure stays visible, with the one way forward beside it.
    expect(screen.getByRole('alert')).toHaveTextContent('offline');
    expect(screen.getByRole('button', { name: aboutCopy.downloadFromSite })).toBeTruthy();
  });

  it('keeps its explanation as help the button row hears, not a paragraph', () => {
    const { container } = render(<UpdateSection />);
    expect(container.querySelector('p')).toBeNull();
    expect(screen.getByRole('button', { name: 'About Version' })).toBeTruthy();
  });

  it('says why the button is busy when the startup check already has a download going', async () => {
    (
      window as unknown as { electron: { getUpdateState: ReturnType<typeof vi.fn> } }
    ).electron.getUpdateState = vi.fn().mockResolvedValue({
      status: 'downloading',
      updateAvailable: true,
      latestVersion: '1.86.0',
      percent: 40,
    });
    render(<UpdateSection />);
    // Nobody pressed "Check for updates" here, and the line is still there: a disabled button
    // with no reason beside it reads as broken.
    expect(await screen.findByText(/Downloading 1\.86\.0/i)).toBeTruthy();
    emitAct({ event: 'download-progress', data: { percent: 40 } });
    expect(screen.getByText('40%')).toBeTruthy();
    // Busy, not `disabled`: a focused button that turns `disabled` hands focus to <body> in
    // Chromium (V4-R3-4). It still reads as unavailable to a screen reader, and the press does
    // nothing.
    const check = screen.getByRole('button', { name: aboutCopy.checkForUpdates });
    expect(check).not.toBeDisabled();
    expect(check).toHaveAttribute('aria-disabled', 'true');
    expect(check).toHaveAttribute('aria-busy', 'true');
    expect(check).toHaveClass('br-action-busy');
    fireEvent.click(check);
    expect(checkForUpdates).not.toHaveBeenCalled();
  });

  it('keeps the button a live control while a check runs, and ignores a second press (V4-R3-4)', async () => {
    let finish: (v: unknown) => void = () => {};
    checkForUpdates.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    render(<UpdateSection />);
    const check = screen.getByRole('button', { name: aboutCopy.checkForUpdates });
    // Idle: an ordinary button, no busy state advertised.
    expect(check).not.toHaveAttribute('aria-busy');
    expect(check).not.toHaveAttribute('aria-disabled');
    expect(check).not.toHaveClass('br-action-busy');

    check.focus();
    fireEvent.click(check);
    expect(await screen.findByText(/Checking for updates/i)).toBeTruthy();
    expect(checkForUpdates).toHaveBeenCalledTimes(1);

    // Under way: the same node, still enabled and still focused.
    const busy = screen.getByRole('button', { name: aboutCopy.checkForUpdates });
    expect(busy).toBe(check);
    expect(busy).not.toBeDisabled();
    expect(busy).toHaveAttribute('aria-busy', 'true');
    expect(busy).toHaveAttribute('aria-disabled', 'true');
    expect(document.activeElement).toBe(busy);
    fireEvent.click(busy);
    expect(checkForUpdates).toHaveBeenCalledTimes(1);

    // The verdict lands: the button is an ordinary one again.
    emitAct({ event: 'update-not-available' });
    await act(async () => finish({ updateInfo: {}, error: null }));
    expect(await screen.findByText(/up to date/i)).toBeTruthy();
    expect(busy).not.toHaveAttribute('aria-busy');
    expect(busy).not.toHaveAttribute('aria-disabled');
    expect(busy).not.toHaveClass('br-action-busy');
    fireEvent.click(busy);
    expect(checkForUpdates).toHaveBeenCalledTimes(2);
  });

  it('keeps the same button node from the check through the download to Restart to update', async () => {
    render(<UpdateSection />);
    const check = screen.getByRole('button', { name: aboutCopy.checkForUpdates });
    check.focus();
    fireEvent.click(check);
    emitAct({ event: 'update-available', data: { version: '1.86.0' } });
    emitAct({ event: 'update-downloaded', data: { version: '1.86.0' } });
    // One element in one position: keyboard focus rides through the label change.
    const restart = await screen.findByRole('button', { name: aboutCopy.restartToUpdate });
    expect(restart).toBe(check);
    expect(document.activeElement).toBe(restart);
    expect(restart).not.toHaveAttribute('aria-busy');
  });

  it('keeps the verdicts, "up to date" and a failure, for a check made in this panel', () => {
    render(<UpdateSection />);
    emitAct({ event: 'update-not-available' });
    expect(screen.queryByText(/up to date/i)).toBeNull();
    emitAct({ event: 'error', data: 'offline' });
    expect(screen.queryByText(/Could not complete the update/i)).toBeNull();
  });

  it('recovers a ready-to-install update opened from a fresh Settings panel', async () => {
    (
      window as unknown as { electron: { getUpdateState: ReturnType<typeof vi.fn> } }
    ).electron.getUpdateState = vi
      .fn()
      .mockResolvedValue({ status: 'downloaded', latestVersion: '1.86.0', percent: 100 });
    render(<UpdateSection />);
    const installBtn = await screen.findByRole('button', { name: aboutCopy.restartToUpdate });
    expect(screen.getByText(aboutCopy.ready('1.86.0'))).toBeTruthy();
    fireEvent.click(installBtn);
    expect(installUpdate).toHaveBeenCalledTimes(1);
  });
});
