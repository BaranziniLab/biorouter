import type { ReactNode } from 'react';
import { useCrew } from '../state/CrewControllerContext';
import type { DialogIntent, DialogKind } from '../state/types';
import { AddPeopleDialog } from './AddPeopleDialog';
import { CrewConfirmation } from './confirmations';
import { ConnectionSettingsDialog } from './ConnectionSettingsDialog';
import { CreateChannelDialog } from './CreateChannelDialog';
import { CreateTeamDialog } from './CreateTeamDialog';
import { EditProfileDialog } from './EditProfileDialog';
import { InvitePeopleDialog } from './InvitePeopleDialog';
import { KeysDialog } from './KeysDialog';
import { LetInDialog } from './LetInDialog';
import { RenameDialog } from './RenameDialog';
import { SharePathDialog } from './SharePathDialog';
import { TransferOwnershipDialog } from './TransferOwnershipDialog';
import { WorkspaceSettingsDialog } from './WorkspaceSettingsDialog';

/**
 * The dialog kinds this area renders. `join` and `host` belong to onboarding and `sign-in` to the
 * controller's own sign-in state, so a layout mounts those elsewhere.
 */
export const HOSTED_DIALOG_KINDS: readonly DialogKind[] = [
  'connection-settings',
  'workspace-settings',
  'invite-people',
  'let-in',
  'create-team',
  'create-channel',
  'add-people',
  'transfer-ownership',
  'rename',
  'edit-profile',
  'keys',
  'share-path',
  'confirm',
];

/**
 * One key per distinct intent, so opening a second intent of the same kind (Let @carol in over
 * Let @bob in) mounts a fresh dialog: every dialog's state is its own and starts clean (L14).
 */
export function dialogKey(intent: DialogIntent): string {
  return JSON.stringify(intent);
}

export interface CrewDialogsProps {
  /** The Workspace settings Agent access tab's content, from the access area. */
  agentAccess?: ReactNode;
}

/**
 * Renders the dialog the controller's `ui.dialog` intent names, if this area owns it. Areas open
 * each other's dialogs only through `openDialog(intent)`, never by importing them; the layout
 * mounts this once. Each dialog is mounted only while its intent is open, so none carries state
 * from one opening to the next, and closing one returns focus to whatever opened it.
 */
export function CrewDialogs({ agentAccess }: CrewDialogsProps) {
  const { ui, closeDialog } = useCrew();
  const intent = ui.dialog;
  if (!intent) return null;
  const key = dialogKey(intent);
  switch (intent.kind) {
    case 'connection-settings':
      return (
        <ConnectionSettingsDialog
          key={key}
          connectionId={intent.connectionId}
          onClose={closeDialog}
        />
      );
    case 'workspace-settings':
      return (
        <WorkspaceSettingsDialog
          key={key}
          tab={intent.tab}
          agentAccess={agentAccess}
          onClose={closeDialog}
        />
      );
    case 'invite-people':
      return <InvitePeopleDialog key={key} onClose={closeDialog} />;
    case 'let-in':
      return <LetInDialog key={key} username={intent.username} onClose={closeDialog} />;
    case 'create-team':
      return <CreateTeamDialog key={key} onClose={closeDialog} />;
    case 'create-channel':
      return <CreateChannelDialog key={key} teamId={intent.teamId} onClose={closeDialog} />;
    case 'add-people':
      return (
        <AddPeopleDialog
          key={key}
          target={intent.target}
          targetId={intent.targetId}
          onClose={closeDialog}
        />
      );
    case 'transfer-ownership':
      return (
        <TransferOwnershipDialog
          key={key}
          channelId={intent.channelId}
          successorId={intent.successorId}
          onClose={closeDialog}
        />
      );
    case 'rename':
      return (
        <RenameDialog
          key={key}
          target={intent.target}
          targetId={intent.targetId}
          onClose={closeDialog}
        />
      );
    case 'edit-profile':
      return <EditProfileDialog key={key} onClose={closeDialog} />;
    case 'keys':
      return <KeysDialog key={key} onClose={closeDialog} />;
    case 'share-path':
      return <SharePathDialog key={key} onClose={closeDialog} />;
    case 'confirm':
      return <CrewConfirmation key={key} confirm={intent.confirm} onClose={closeDialog} />;
    case 'join':
    case 'host':
    case 'sign-in':
      return null;
  }
}
