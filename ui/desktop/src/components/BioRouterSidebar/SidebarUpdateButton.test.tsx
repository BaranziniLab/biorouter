import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SidebarUpdateButton from './SidebarUpdateButton';
import UpdateAvailableModal from '../UpdateAvailableModal';
import type { UpdaterEventPayload, UpdaterState } from '../../utils/updaterState';
import { OPEN_UPDATE_MODAL_EVENT } from '../../utils/updateUiEvents';

let listeners: Set<(payload: UpdaterEventPayload) => void>;
let getUpdateState: ReturnType<typeof vi.fn>;

function emit(payload: UpdaterEventPayload) {
  act(() => listeners.forEach((listener) => listener(payload)));
}

beforeEach(() => {
  listeners = new Set();
  getUpdateState = vi.fn().mockResolvedValue(null);
  (window as unknown as { electron: unknown }).electron = {
    onUpdaterEvent: (listener: (payload: UpdaterEventPayload) => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getUpdateState,
  };
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('SidebarUpdateButton', () => {
  it('stays hidden when Biorouter is current', () => {
    render(<SidebarUpdateButton />);
    expect(screen.queryByTestId('sidebar-update-button')).toBeNull();

    emit({ event: 'checking-for-update' });
    emit({ event: 'update-not-available', data: { version: '1.88.0' } });
    expect(screen.queryByTestId('sidebar-update-button')).toBeNull();
  });

  it('appears for a newer version and requests the existing update prompt on click', () => {
    let requestedState: UpdaterState | undefined;
    const handleRequest = (event: Event) => {
      requestedState = (event as CustomEvent<UpdaterState>).detail;
    };
    window.addEventListener(OPEN_UPDATE_MODAL_EVENT, handleRequest);

    render(<SidebarUpdateButton />);
    emit({ event: 'update-available', data: { version: '1.89.0' } });

    const button = screen.getByRole('button', { name: 'Update Biorouter to 1.89.0' });
    // A rail row in sentence case, not a caps "UPDATE" bar (spec 3.4, F-14).
    expect(button).toHaveTextContent('Update to 1.89.0');
    expect(button).toHaveAttribute('data-tone', 'update');
    fireEvent.click(button);

    expect(requestedState?.phase).toBe('available');
    expect(requestedState?.latestVersion).toBe('1.89.0');
    window.removeEventListener(OPEN_UPDATE_MODAL_EVENT, handleRequest);
  });

  it('opens the existing update prompt only after the button is clicked', async () => {
    render(
      <>
        <SidebarUpdateButton />
        <UpdateAvailableModal />
      </>
    );
    emit({ event: 'update-available', data: { version: '1.89.0' } });

    expect(screen.queryByText(/Downloading update/i)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Update Biorouter to 1.89.0' }));

    expect(await screen.findByText(/Downloading update/i)).toBeTruthy();
  });

  it('remains visible through a recheck or download error, then hides when up to date', () => {
    render(<SidebarUpdateButton />);
    emit({ event: 'update-available', data: { version: '1.89.0' } });
    expect(screen.getByTestId('sidebar-update-button')).toBeTruthy();

    emit({ event: 'checking-for-update' });
    expect(screen.getByTestId('sidebar-update-button')).toBeTruthy();

    emit({ event: 'error', data: 'temporary network error' });
    expect(screen.getByTestId('sidebar-update-button')).toBeTruthy();

    emit({ event: 'update-not-available' });
    expect(screen.queryByTestId('sidebar-update-button')).toBeNull();
  });

  it('recovers an update that was found before the sidebar mounted', async () => {
    getUpdateState.mockResolvedValue({
      updateAvailable: true,
      status: 'downloaded',
      latestVersion: '1.89.0',
      percent: 100,
    });

    render(<SidebarUpdateButton />);

    expect(await screen.findByRole('button', { name: 'Update Biorouter to 1.89.0' })).toBeTruthy();
  });
});
