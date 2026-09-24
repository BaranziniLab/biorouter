import { useCallback, useRef, useState } from 'react';
import type {
  CrewUi,
  DialogIntent,
  PaneIntent,
  SignInState,
  SurfaceResetListener,
  SurfaceResetReason,
} from './types';

/**
 * Dialogs whose content is drawn from the verified snapshot. A refresh or a lost verification
 * closes them; Join, Host, Connection settings, Keys, Invite people and Let in stay open because
 * they are about the connection or hold a result the person still has to copy.
 */
export function isSnapshotBoundDialog(dialog: DialogIntent): boolean {
  switch (dialog.kind) {
    case 'workspace-settings':
    case 'create-team':
    case 'create-channel':
    case 'add-people':
    case 'transfer-ownership':
    case 'rename':
    case 'edit-profile':
    case 'share-path':
      return true;
    case 'confirm':
      return dialog.confirm.action !== 'remove-connection';
    default:
      return false;
  }
}

/**
 * How the new layout's surfaces react when the controller resets them. The details pane survives
 * a refresh (an error shown in it must outlive a manual refresh); it closes on a channel or
 * connection switch, on lost access, and when the verified view is cleared. A finished dialog
 * mutation closes its dialog; a started task leaves the pane to its layout.
 */
export function nextUiAfterReset(ui: CrewUi, reason: SurfaceResetReason): CrewUi {
  const keepDialog = (dialog: DialogIntent | null) =>
    dialog && !isSnapshotBoundDialog(dialog) ? dialog : null;
  switch (reason) {
    case 'refresh':
      return ui.dialog && isSnapshotBoundDialog(ui.dialog) ? { ...ui, dialog: null } : ui;
    case 'protected-cleared':
    case 'channel-changed':
    case 'channel-revoked':
    case 'connection-changed':
      return ui.pane || (ui.dialog && isSnapshotBoundDialog(ui.dialog))
        ? { dialog: keepDialog(ui.dialog), pane: null }
        : ui;
    case 'mutated':
      return ui.dialog ? { ...ui, dialog: null } : ui;
    case 'run-started':
      return ui;
  }
}

export interface CrewSurfaces {
  ui: CrewUi;
  openDialog(intent: DialogIntent): void;
  closeDialog(): void;
  openPane(intent: PaneIntent): void;
  closePane(): void;
  resetSurfaces(reason: SurfaceResetReason): void;
  subscribeSurfaceReset(listener: SurfaceResetListener): () => void;
  signIn: SignInState;
  openSignIn(reason?: 'user' | 'auto'): void;
  closeSignIn(): void;
}

/** Dialog and pane intents, the sign-in dialog, and the surface-reset listeners. */
export function useCrewSurfaces(): CrewSurfaces {
  const [ui, setUi] = useState<CrewUi>({ dialog: null, pane: null });
  const [signIn, setSignIn] = useState<SignInState>({ open: false, reason: null });
  const listeners = useRef(new Set<SurfaceResetListener>());

  const openSignIn = useCallback((reason: 'user' | 'auto' = 'user') => {
    setSignIn((current) => (current.open ? current : { open: true, reason }));
  }, []);
  const closeSignIn = useCallback(() => setSignIn({ open: false, reason: null }), []);
  const openDialog = useCallback(
    (intent: DialogIntent) => {
      if (intent.kind === 'sign-in') openSignIn('user');
      else setUi((current) => ({ ...current, dialog: intent }));
    },
    [openSignIn]
  );
  const closeDialog = useCallback(() => setUi((current) => ({ ...current, dialog: null })), []);
  const openPane = useCallback(
    (intent: PaneIntent) => setUi((current) => ({ ...current, pane: intent })),
    []
  );
  const closePane = useCallback(() => setUi((current) => ({ ...current, pane: null })), []);
  // Listeners run synchronously, inside the same update as the state change that caused the
  // reset, so a layout's own surfaces close in the same render.
  const resetSurfaces = useCallback((reason: SurfaceResetReason) => {
    setUi((current) => nextUiAfterReset(current, reason));
    for (const listener of [...listeners.current]) listener(reason);
  }, []);
  const subscribeSurfaceReset = useCallback((listener: SurfaceResetListener) => {
    listeners.current.add(listener);
    return () => {
      listeners.current.delete(listener);
    };
  }, []);

  return {
    ui,
    openDialog,
    closeDialog,
    openPane,
    closePane,
    resetSurfaces,
    subscribeSurfaceReset,
    signIn,
    openSignIn,
    closeSignIn,
  };
}
