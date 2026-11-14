import { useEffect, useState } from 'react';
import { Download } from '../icons/app-icons';
import { UPDATES_ENABLED } from '../../updates';
import {
  hasKnownUpdate,
  initialUpdaterState,
  reduceUpdaterEvent,
  stateFromSnapshot,
  UPDATE_PHASE_ORDER,
  type UpdaterState,
} from '../../utils/updaterState';
import { requestUpdateModal } from '../../utils/updateUiEvents';
import { SidebarMenuItem } from '../ui/sidebar';
import { isBrowserSurface } from '../../utils/surface';
import { ServeSidebarUpdateRow } from '../serveUpdate/ServeUpdateEntryPoints';
import { sidebarCopy } from './copy';

/**
 * The lower-left "Update to …" row. On a `biorouter serve` page the daemon
 * updates itself (SD-19), so the row reads the browser's update store; in the
 * desktop app it reads the Electron main process's updater.
 */
export default function SidebarUpdateButton() {
  return isBrowserSurface() ? <ServeSidebarUpdateRow /> : <DesktopSidebarUpdateButton />;
}

function DesktopSidebarUpdateButton() {
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
          UPDATE_PHASE_ORDER.indexOf(recovered.phase) > UPDATE_PHASE_ORDER.indexOf(previous.phase)
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
