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
