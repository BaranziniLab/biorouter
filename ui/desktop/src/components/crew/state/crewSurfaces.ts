import { useCallback, useEffect, useRef, useState } from 'react';
import { nextOpener, restoreFocusSoon } from './focusReturn';
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
 * a refresh (an error shown in it must outlive a manual refresh) and a channel switch, on the same
 * tab, so two channels' members can be compared side by side (QA Q2-33); the agent and chat-access
 * panes are about the channel they were opened on, so a switch closes them. Every pane closes on a
 * connection switch, on lost access, and when the verified view is cleared. A finished dialog
 * mutation closes its dialog; a started task leaves the pane to its layout.
 */
export function nextUiAfterReset(ui: CrewUi, reason: SurfaceResetReason): CrewUi {
  const keepDialog = (dialog: DialogIntent | null) =>
    dialog && !isSnapshotBoundDialog(dialog) ? dialog : null;
  switch (reason) {
    case 'refresh':
      return ui.dialog && isSnapshotBoundDialog(ui.dialog) ? { ...ui, dialog: null } : ui;
    case 'channel-changed': {
      const pane = ui.pane?.mode === 'details' ? ui.pane : null;
      const dialog = keepDialog(ui.dialog);
      return pane === ui.pane && dialog === ui.dialog ? ui : { dialog, pane };
    }
    case 'protected-cleared':
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

/**
 * Dialog and pane intents, the sign-in dialog, and the surface-reset listeners.
 *
 * Opening a dialog records its opener (`focusReturn.ts`) — the control that had focus, or the
 * trigger of the menu it sat in — and once the dialog is gone, however it closed (Cancel, Escape, a
 * finished mutation, a reset), focus goes back there. Crew dialogs have no Radix trigger, so
 * without this focus fell to `<body>` after every one of them (QA T-15). Sign in keeps its own
 * opener in `SignInDialog`, because it closes with an exit animation.
 */
export function useCrewSurfaces(): CrewSurfaces {
  const [ui, setUi] = useState<CrewUi>({ dialog: null, pane: null });
  const [signIn, setSignIn] = useState<SignInState>({ open: false, reason: null });
  const listeners = useRef(new Set<SurfaceResetListener>());
  const dialogOpener = useRef<HTMLElement | null>(null);
  const hadDialog = useRef(false);

  // A dialog that closed, by any route, hands focus back to its opener once it has unmounted. A
  // dialog replaced by another is not a close: the opener carries over (`nextOpener`).
  useEffect(() => {
    const open = ui.dialog !== null;
    if (hadDialog.current && !open) {
      restoreFocusSoon(dialogOpener.current);
      dialogOpener.current = null;
    }
    hadDialog.current = open;
  }, [ui.dialog]);

  const openSignIn = useCallback((reason: 'user' | 'auto' = 'user') => {
    setSignIn((current) => (current.open ? current : { open: true, reason }));
  }, []);
  const closeSignIn = useCallback(
    () =>
      setSignIn((current) =>
        current.open || current.reason ? { open: false, reason: null } : current
      ),
    []
  );
  const openDialog = useCallback(
    (intent: DialogIntent) => {
      if (intent.kind === 'sign-in') {
        openSignIn('user');
        return;
      }
      // Recorded now, from the event that opens it, while the opener still has focus.
      dialogOpener.current = nextOpener(dialogOpener.current);
      setUi((current) => ({ ...current, dialog: intent }));
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
