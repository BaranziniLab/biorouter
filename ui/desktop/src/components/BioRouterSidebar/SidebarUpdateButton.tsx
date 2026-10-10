import { useEffect, useState } from 'react';
import { Download } from '../icons/app-icons';
import { UPDATES_ENABLED } from '../../updates';
import {
  hasKnownUpdate,
  initialUpdaterState,
  reduceUpdaterEvent,
  stateFromSnapshot,
  type UpdaterState,
} from '../../utils/updaterState';
import { requestUpdateModal } from '../../utils/updateUiEvents';
import { SidebarMenuItem } from '../ui/sidebar';
import { sidebarCopy } from './copy';

const PHASE_ORDER: UpdaterState['phase'][] = [
  'idle',
  'up-to-date',
  'checking',
  'error',
  'available',
  'downloaded',
];

export default function SidebarUpdateButton() {
  const [state, setState] = useState<UpdaterState>(initialUpdaterState);

  useEffect(() => {
    if (!UPDATES_ENABLED || !window.electron?.onUpdaterEvent) return;

    let cancelled = false;
    const dispose = window.electron.onUpdaterEvent((payload) => {
      if (!cancelled) setState((previous) => reduceUpdaterEvent(previous, payload));
    });

    window.electron
      .getUpdateState?.()
      ?.then((snapshot) => {
        if (cancelled || !snapshot) return;
        const recovered = stateFromSnapshot(snapshot);
        setState((previous) =>
          PHASE_ORDER.indexOf(recovered.phase) > PHASE_ORDER.indexOf(previous.phase)
            ? recovered
            : previous
        );
      })
      .catch(() => {
        // Live updater events remain the source of truth when no snapshot is available.
      });

    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  if (!UPDATES_ENABLED || !hasKnownUpdate(state)) return null;

  const versionLabel = state.latestVersion ? ` to ${state.latestVersion}` : '';

  // A normal rail row, not a filled caps bar (F-14): the download glyph in the
  // accent is the one spot of colour it needs. The accessible name keeps
  // "Update Biorouter to …", which tests and assistive technology read.
  return (
    <SidebarMenuItem>
      <button
        type="button"
        data-testid="sidebar-update-button"
        data-tone="update"
        aria-label={`Update Biorouter${versionLabel}`}
        onClick={() => requestUpdateModal(state)}
        className="br-nav-row no-drag"
      >
        <Download className="br-nav-row-icon" aria-hidden />
        <span className="br-nav-row-label">{sidebarCopy.update.row(state.latestVersion)}</span>
      </button>
    </SidebarMenuItem>
  );
}
