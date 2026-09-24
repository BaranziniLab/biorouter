import { useRef } from 'react';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { DangerousConfirmDialog } from '../../ui/DangerousConfirmDialog';
import { toastSuccess } from '../../../toasts';
import { channelName, personLabel } from '../identity';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ConfirmIntent, CrewController, ErrorSource } from '../state/types';
import { confirmCopy, dialogErrorCopy } from './copy';
import { DialogErrorNote } from './fields';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView } from './workspace';

/**
 * Every confirmation of the copy deck's confirmation table that is a dialog (the two inline
 * two-step confirmations live beside the rows they act on).
 *
 * The friction follows the spec's privacy table: exposing a connection or a workspace is a typed
 * `DangerousConfirmDialog` whose phrase is the workspace's own name, so the person checks WHICH
 * workspace and not just the word "public"; removing a person types their username, checked
 * case-sensitively on top of the primitive's case-folded gate; the irreversible institution label
 * has a confirmation no key can answer. Destructive confirmations hold focus on Cancel.
 *
 * React authorizes nothing here: each confirm sends the request the broker or daemon decides, and a
 * refusal renders in the confirmation that sent it (`dialog:confirm`).
 */

const SOURCE: ErrorSource = 'dialog:confirm';

export interface CrewConfirmationProps {
  confirm: ConfirmIntent;
  /** Called when the confirmation is dismissed or its action succeeded. */
  onClose(): void;
}

/** The confirmation for one intent. Mount it while the intent is open. */
export function CrewConfirmation({ confirm, onClose }: CrewConfirmationProps) {
  switch (confirm.action) {
    case 'make-connection-public':
      return <MakeConnectionPublic connectionId={confirm.connectionId} onClose={onClose} />;
    case 'allow-workspace-public':
      return <AllowWorkspacePublic onClose={onClose} />;
    case 'make-workspace-private':
      return <MakeWorkspacePrivate onClose={onClose} />;
    case 'set-institution':
      return <SetInstitution institutionId={confirm.institutionId} onClose={onClose} />;
    case 'remove-person':
      return <RemovePerson principalId={confirm.principalId} onClose={onClose} />;
    case 'archive-channel':
      return <ArchiveChannel channelId={confirm.channelId} onClose={onClose} />;
    case 'remove-channel-member':
      return (
        <RemoveChannelMember
          channelId={confirm.channelId}
          principalId={confirm.principalId}
          onClose={onClose}
        />
      );
    case 'remove-connection':
      return <RemoveConnection connectionId={confirm.connectionId} onClose={onClose} />;
    case 'stop-task':
      return <StopTask runId={confirm.runId} onClose={onClose} />;
  }
}

/** Run a confirmation's action under `dialog:confirm`; true only when it finished. */
function runConfirmed(
  crew: CrewController,
  key: string,
  action: () => Promise<unknown>
): Promise<boolean> {
  return crew
    .act(SOURCE, key, async () => {
      await action();
      return true as const;
    })
    .then((done) => done === true);
}

export interface MakeConnectionPublicDialogProps {
  workspace: string;
  busy: boolean;
  onConfirm(): void;
  onCancel(): void;
}

/**
 * Private → Public for this person's connection. Shared by the intent (the privacy popover and
 * the Privacy tab) and by Connection settings, which confirms before it saves a changed mode.
 */
export function MakeConnectionPublicDialog({
  workspace,
  busy,
  onConfirm,
  onCancel,
}: MakeConnectionPublicDialogProps) {
  return (
    <DangerousConfirmDialog
      open
      title={confirmCopy.makeConnectionPublic.title(workspace)}
      description={confirmCopy.makeConnectionPublic.description}
      phrase={workspace}
      fieldLabel={confirmCopy.typeToConfirm(workspace)}
      confirmLabel={confirmCopy.makeConnectionPublic.confirm}
      cancelLabel={confirmCopy.cancel}
      busy={busy}
      onConfirm={onConfirm}
      onCancel={onCancel}
    >
      <DialogErrorNote source={SOURCE} />
    </DangerousConfirmDialog>
  );
}

function MakeConnectionPublic({
  connectionId,
  onClose,
}: {
  connectionId: string;
  onClose(): void;
}) {
  const { crew, workspace } = useDialogView(connectionId);
  const saved = crew.connections.find((item) => item.id === connectionId) ?? null;
  const key = 'connection.update';
  useCloseWhenMissing(saved === null, onClose);
  if (!saved) return null;
  return (
    <MakeConnectionPublicDialog
      workspace={workspace}
      busy={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() =>
        void runConfirmed(crew, key, async () => {
          // L18: the daemon replaces the whole record, so the whole record is sent.
          await crew.updateConnection(saved.id, { ...connectionUpdateBody(saved), mode: 'public' });
          if (saved.id === crew.connectionId) await crew.refresh();
        }).then((done) => done && onClose())
      }
    />
  );
}

function AllowWorkspacePublic({ onClose }: { onClose(): void }) {
  const { crew, workspace } = useDialogView();
  const key = 'mutate:policy.set';
  return (
    <DangerousConfirmDialog
      open
      title={confirmCopy.allowWorkspacePublic.title(workspace)}
      description={confirmCopy.allowWorkspacePublic.description}
      phrase={workspace}
      fieldLabel={confirmCopy.typeToConfirm(workspace)}
      confirmLabel={confirmCopy.allowWorkspacePublic.confirm}
      cancelLabel={confirmCopy.cancel}
      busy={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() =>
        void runConfirmed(crew, key, async () => {
          if (!crew.snapshot) throw new Error(dialogErrorCopy.privacyUnverified);
          await crew.mutate('policy.set', {
            mode: 'public',
            institution_id: crew.snapshot.workspace.institution_id ?? null,
          });
        }).then((done) => done && onClose())
      }
    >
      <DialogErrorNote source={SOURCE} />
    </DangerousConfirmDialog>
  );
}

function MakeWorkspacePrivate({ onClose }: { onClose(): void }) {
  const { crew, workspace } = useDialogView();
  const key = 'mutate:policy.set';
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.makeWorkspacePrivate.title(workspace)}
      message={confirmCopy.makeWorkspacePrivate.description}
      confirmLabel={confirmCopy.makeWorkspacePrivate.confirm}
      cancelLabel={confirmCopy.cancel}
      isSubmitting={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() =>
        void runConfirmed(crew, key, async () => {
          if (!crew.snapshot) throw new Error(dialogErrorCopy.privacyUnverified);
          await crew.mutate('policy.set', {
            mode: 'private',
            institution_id: crew.snapshot.workspace.institution_id ?? null,
          });
        }).then((done) => {
          if (!done) return;
          toastSuccess({ msg: confirmCopy.makeWorkspacePrivate.toast(workspace) });
          onClose();
        })
      }
    >
      <DialogErrorNote source={SOURCE} />
    </ConfirmationModal>
  );
}

function SetInstitution({ institutionId, onClose }: { institutionId: string; onClose(): void }) {
  const { crew, workspace } = useDialogView();
  const key = 'mutate:policy.set';
  return (
    <DangerousConfirmDialog
      open
      title={confirmCopy.setInstitution.title(workspace, institutionId)}
      description={confirmCopy.setInstitution.description(institutionId)}
      confirmLabel={confirmCopy.setInstitution.confirm(institutionId)}
      cancelLabel={confirmCopy.cancel}
      busy={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() =>
        void runConfirmed(crew, key, async () => {
          if (!crew.snapshot) throw new Error(dialogErrorCopy.privacyUnverified);
          await crew.mutate('policy.set', {
            mode: crew.snapshot.workspace.mode,
            institution_id: institutionId,
          });
        }).then((done) => done && onClose())
      }
    >
      <DialogErrorNote source={SOURCE} />
    </DangerousConfirmDialog>
  );
}

function RemovePerson({ principalId, onClose }: { principalId: string; onClose(): void }) {
  const { crew, dir, workspace } = useDialogView();
  const typed = useRef('');
  const person = dir.byId(principalId);
  const key = 'mutate:enrollment.revoke';
  // No name to type means nothing to confirm against: never show a removal without its phrase.
  useCloseWhenMissing(person === null, onClose);
  if (!person) return null;
  const username = person.username;
  return (
    <DangerousConfirmDialog
      open
      title={confirmCopy.removePerson.title(personLabel(person, 'authority', dir), workspace)}
      description={confirmCopy.removePerson.description}
      phrase={username}
      fieldLabel={confirmCopy.typeToConfirm(username)}
      confirmLabel={confirmCopy.removePerson.confirm(workspace)}
      cancelLabel={confirmCopy.cancel}
      busy={crew.isPending(key)}
      onPhraseChange={(value) => {
        typed.current = value;
      }}
      onCancel={onClose}
      onConfirm={() => {
        // A Unix username is case-sensitive: `Bob` and `bob` can be two accounts. The primitive's
        // gate is case-folded, so the exact spelling is checked here, before anything is sent.
        if (typed.current.trim() !== username) {
          crew.reportError(confirmCopy.removePerson.mismatch, SOURCE);
          return;
        }
        void runConfirmed(crew, key, () =>
          crew.mutate('enrollment.revoke', {
            principal_id: principalId,
            expected_username: username,
          })
        ).then((done) => done && onClose());
      }}
    >
      <DialogErrorNote source={SOURCE} />
    </DangerousConfirmDialog>
  );
}

function ArchiveChannel({ channelId, onClose }: { channelId: string; onClose(): void }) {
  const { crew, snapshot } = useDialogView();
  const channel = snapshot?.channels.find((item) => item.id === channelId) ?? null;
  const key = 'mutate:channel.archive';
  useCloseWhenMissing(channel === null, onClose);
  if (!channel) return null;
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.archiveChannel.title(channelName(channel))}
      message={confirmCopy.archiveChannel.description}
      confirmLabel={confirmCopy.archiveChannel.confirm}
      cancelLabel={confirmCopy.cancel}
      confirmVariant="destructive"
      isSubmitting={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() =>
        void runConfirmed(crew, key, () =>
          crew.mutate('channel.archive', { channel_id: channelId })
        ).then((done) => done && onClose())
      }
    >
      <DialogErrorNote source={SOURCE} />
    </ConfirmationModal>
  );
}

function RemoveChannelMember({
  channelId,
  principalId,
  onClose,
}: {
  channelId: string;
  principalId: string;
  onClose(): void;
}) {
  const { crew, snapshot, dir } = useDialogView();
  const channel = snapshot?.channels.find((item) => item.id === channelId) ?? null;
  const person = dir.byId(principalId);
  const key = 'mutate:membership.revoke';
  useCloseWhenMissing(channel === null || person === null, onClose);
  if (!channel || !person) return null;
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.removeChannelMember.title(
        personLabel(person, 'authority', dir),
        channelName(channel)
      )}
      message={confirmCopy.removeChannelMember.description}
      confirmLabel={confirmCopy.removeChannelMember.confirm}
      cancelLabel={confirmCopy.cancel}
      confirmVariant="destructive"
      isSubmitting={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() =>
        void runConfirmed(crew, key, () =>
          crew.mutate('membership.revoke', {
            channel_id: channelId,
            principal_id: principalId,
            expected_username: person.username,
          })
        ).then((done) => done && onClose())
      }
    >
      <DialogErrorNote source={SOURCE} />
    </ConfirmationModal>
  );
}

function RemoveConnection({ connectionId, onClose }: { connectionId: string; onClose(): void }) {
  const { crew, workspace } = useDialogView(connectionId);
  const key = 'connection.remove';
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.removeConnection.title(workspace)}
      message={confirmCopy.removeConnection.description}
      confirmLabel={confirmCopy.removeConnection.confirm}
      cancelLabel={confirmCopy.cancel}
      confirmVariant="destructive"
      isSubmitting={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() =>
        void runConfirmed(crew, key, () => crew.removeConnection(connectionId)).then(
          (done) => done && onClose()
        )
      }
    >
      <DialogErrorNote source={SOURCE} />
    </ConfirmationModal>
  );
}

function StopTask({ runId, onClose }: { runId: string; onClose(): void }) {
  const { crew } = useDialogView();
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.stopTask.title}
      message={confirmCopy.stopTask.description}
      confirmLabel={confirmCopy.stopTask.confirm}
      cancelLabel={confirmCopy.stopTask.cancel}
      confirmVariant="destructive"
      isSubmitting={crew.isPending('run.cancel')}
      onCancel={onClose}
      onConfirm={() => {
        // `cancelRun` records its own failure in the connection bar and never throws: the task row
        // it came from is what shows whether the stop landed.
        void crew.cancelRun(runId);
        onClose();
      }}
    />
  );
}
