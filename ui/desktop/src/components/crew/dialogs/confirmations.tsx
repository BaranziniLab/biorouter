import { useCallback, useRef } from 'react';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { DangerousConfirmDialog } from '../../ui/DangerousConfirmDialog';
import { toastSuccess } from '../../../toasts';
import type { Snapshot } from '../crewApi';
import { channelName, personLabel, teamName } from '../identity';
import { sshUsername } from '../onboarding/joinText';
import { isWorkspaceDeleted } from '../state/connectFailure';
import { connectionUpdateBody } from '../state/useCrewConnections';
import type { ConfirmIntent, CrewController, ErrorSource } from '../state/types';
import { confirmCopy, dialogErrorCopy } from './copy';
import { DialogErrorNote } from './fields';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView, type DialogView } from './workspace';
import './dialogs.css';

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
 * Every one is an `alertdialog` (QA T-72): it interrupts to ask about one consequential action,
 * which is what that role tells a screen reader, where a plain `dialog` reads as a form.
 *
 * React authorizes nothing here: each confirm sends the request the broker or daemon decides, and a
 * refusal renders in the confirmation that sent it (`dialog:confirm`).
 */

const SOURCE: ErrorSource = 'dialog:confirm';

/**
 * Makes the confirmation it is rendered in an `alertdialog`. The shared primitives
 * (`ConfirmationModal`, `DangerousConfirmDialog`) render Radix's `role="dialog"` and take no role
 * of their own, and they belong to every privacy surface in the app, so the role is set here, on
 * the one element Radix names the dialog by. React never rewrites an attribute whose prop did not
 * change, so it holds for the dialog's life.
 *
 * Its `crew-confirmation` class is also how Crew's stylesheet finds a confirmation: the primitives
 * take no class, and `CrewDialogs`' `crew-dialog` reaches only a `ModalShell`. `dialogs.css` gives
 * the typed-name field of a dialog that holds this mark the same focus edge every other Crew
 * dialog field has (QA Q2-25), with `:has()` rather than by writing a class onto a node React owns.
 */
function AlertDialogRole() {
  const mark = useCallback((node: HTMLSpanElement | null) => {
    node?.closest('[role="dialog"]')?.setAttribute('role', 'alertdialog');
  }, []);
  return <span ref={mark} className="crew-confirmation" hidden />;
}

/** The body every confirmation renders between its words and its buttons. */
function ConfirmBody() {
  return (
    <>
      <AlertDialogRole />
      <DialogErrorNote source={SOURCE} />
    </>
  );
}

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
    case 'delete-channel':
      return <DeleteChannel channelId={confirm.channelId} onClose={onClose} />;
    case 'delete-team':
      return <DeleteTeam teamId={confirm.teamId} onClose={onClose} />;
    case 'delete-workspace':
      return <DeleteWorkspace connectionId={confirm.connectionId} onClose={onClose} />;
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
  /** The name to type; the workspace's own name (`useDialogView().phrase`). */
  phrase: string;
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
  phrase,
  busy,
  onConfirm,
  onCancel,
}: MakeConnectionPublicDialogProps) {
  return (
    <DangerousConfirmDialog
      open
      title={confirmCopy.makeConnectionPublic.title(workspace)}
      description={confirmCopy.makeConnectionPublic.description}
      phrase={phrase}
      fieldLabel={confirmCopy.typeToConfirm(phrase)}
      confirmLabel={confirmCopy.makeConnectionPublic.confirm}
      cancelLabel={confirmCopy.cancel}
      busy={busy}
      onConfirm={onConfirm}
      onCancel={onCancel}
    >
      <ConfirmBody />
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
  const { crew, workspace, phrase } = useDialogView(connectionId);
  const saved = crew.connections.find((item) => item.id === connectionId) ?? null;
  const key = 'connection.update';
  useCloseWhenMissing(saved === null, onClose);
  if (!saved) return null;
  return (
    <MakeConnectionPublicDialog
      workspace={workspace}
      phrase={phrase}
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
  const { crew, workspace, phrase } = useDialogView();
  const key = 'mutate:policy.set';
  return (
    <DangerousConfirmDialog
      open
      title={confirmCopy.allowWorkspacePublic.title(workspace)}
      description={confirmCopy.allowWorkspacePublic.description}
      phrase={phrase}
      fieldLabel={confirmCopy.typeToConfirm(phrase)}
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
      <ConfirmBody />
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
      <ConfirmBody />
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
      <ConfirmBody />
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
      <ConfirmBody />
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
      <ConfirmBody />
    </ConfirmationModal>
  );
}

/**
 * Where to go when the channel on screen is about to be deleted, or its team: another open channel
 * of the same team (its #general first), else of any team, else none. Moving there first retires
 * the deleted channel's observation, so its end is not read as access the person lost.
 */
export function channelAfterDeleting(
  snapshot: Snapshot | null,
  deleted: { channelId?: string; teamId?: string }
): string | null {
  if (!snapshot) return null;
  const gone = (channel: Snapshot['channels'][number]) =>
    channel.archived ||
    channel.id === deleted.channelId ||
    (deleted.teamId !== undefined && channel.team_id === deleted.teamId);
  const open = snapshot.channels.filter((channel) => !gone(channel));
  const teamOf = deleted.channelId
    ? snapshot.channels.find((channel) => channel.id === deleted.channelId)?.team_id
    : undefined;
  const team = snapshot.teams.find((item) => item.id === teamOf);
  return (
    open.find((channel) => channel.id === team?.general_channel_id)?.id ??
    open.find((channel) => channel.team_id === teamOf)?.id ??
    open[0]?.id ??
    null
  );
}

/**
 * Delete a channel for everyone (OW-4). Like Archive it is one destructive click, as the CLI's
 * `channels delete --yes`: the dialog names what goes. `expected_name` is the stored name the
 * person saw, so a rename meanwhile is refused (`name_changed`) rather than deleting a channel
 * under another name. Deleting the channel on screen first moves to another one.
 */
function DeleteChannel({ channelId, onClose }: { channelId: string; onClose(): void }) {
  const { crew, snapshot } = useDialogView();
  const channel = snapshot?.channels.find((item) => item.id === channelId) ?? null;
  const key = 'mutate:channel.delete';
  useCloseWhenMissing(channel === null, onClose);
  if (!channel) return null;
  const name = channelName(channel);
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.deleteChannel.title(name)}
      message={confirmCopy.deleteChannel.description(name)}
      confirmLabel={confirmCopy.deleteChannel.confirm}
      cancelLabel={confirmCopy.cancel}
      confirmVariant="destructive"
      isSubmitting={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() => {
        const onScreen = crew.channelId === channelId;
        const next = onScreen ? channelAfterDeleting(snapshot, { channelId }) : null;
        void runConfirmed(crew, key, async () => {
          await crew.mutate(
            'channel.delete',
            { channel_id: channelId, expected_name: channel.name },
            { refresh: !onScreen }
          );
          if (onScreen && next) crew.selectChannel(next);
          else if (onScreen) await crew.refresh();
        }).then((done) => {
          if (!done) return;
          toastSuccess({ msg: confirmCopy.deleteChannel.toast(name) });
          onClose();
        });
      }}
    >
      <ConfirmBody />
    </ConfirmationModal>
  );
}

/** The `channels` count a `team.delete` answer carries, as a number or a list, else null. */
function deletedChannelCount(result: unknown): number | null {
  if (typeof result !== 'object' || result === null) return null;
  const channels = (result as { channels?: unknown }).channels;
  if (typeof channels === 'number' && Number.isSafeInteger(channels) && channels >= 0)
    return channels;
  return Array.isArray(channels) ? channels.length : null;
}

/** Delete a team and every channel in it for everyone (OW-4), as `teams delete --yes` does. */
function DeleteTeam({ teamId, onClose }: { teamId: string; onClose(): void }) {
  const { crew, snapshot } = useDialogView();
  const team = snapshot?.teams.find((item) => item.id === teamId) ?? null;
  const key = 'mutate:team.delete';
  useCloseWhenMissing(team === null, onClose);
  if (!team) return null;
  const name = teamName(team);
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.deleteTeam.title(name)}
      message={confirmCopy.deleteTeam.description(name)}
      confirmLabel={confirmCopy.deleteTeam.confirm}
      cancelLabel={confirmCopy.cancel}
      confirmVariant="destructive"
      isSubmitting={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() => {
        const onScreen = crew.teamId === teamId;
        const next = onScreen ? channelAfterDeleting(snapshot, { teamId }) : null;
        let channels: number | null = null;
        void runConfirmed(crew, key, async () => {
          const result = await crew.mutate(
            'team.delete',
            { team_id: teamId, expected_name: team.name },
            { refresh: !onScreen }
          );
          channels = deletedChannelCount(result);
          if (onScreen && next) crew.selectChannel(next);
          else if (onScreen) await crew.refresh();
        }).then((done) => {
          if (!done) return;
          toastSuccess({ msg: confirmCopy.deleteTeam.toast(name, channels) });
          onClose();
        });
      }}
    >
      <ConfirmBody />
    </ConfirmationModal>
  );
}

/**
 * The host deletes the workspace for everyone (OW-5): typed with the workspace's own name, as
 * every confirmation that cannot be undone and concerns the whole workspace is. The daemon sends
 * `workspace.delete`, then removes the connection from this computer, so the dialog is not bound
 * to the snapshot (`crewSurfaces`): the request ends the view while it is on its way.
 */
function DeleteWorkspace({ connectionId, onClose }: { connectionId: string; onClose(): void }) {
  const { crew, workspace, phrase, snapshot, server } = useDialogView(connectionId);
  const saved = crew.connections.find((item) => item.id === connectionId) ?? null;
  const key = 'connection.delete-workspace';
  const deleteWorkspace = crew.deleteWorkspace;
  // Nothing to name, or no way to send it: never show a deletion that could not say which.
  const missing = saved === null || !deleteWorkspace;
  const pending = useRef(false);
  useCloseWhenMissing(missing && !pending.current, onClose);
  if (!saved || !deleteWorkspace) return null;
  return (
    <DangerousConfirmDialog
      open
      title={confirmCopy.deleteWorkspace.title(workspace)}
      description={confirmCopy.deleteWorkspace.description(workspace, server)}
      phrase={phrase}
      fieldLabel={confirmCopy.typeToConfirm(phrase)}
      confirmLabel={confirmCopy.deleteWorkspace.confirm}
      cancelLabel={confirmCopy.cancel}
      busy={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={() => {
        if (!snapshot) {
          crew.reportError(dialogErrorCopy.privacyUnverified, SOURCE);
          return;
        }
        pending.current = true;
        void runConfirmed(crew, key, () =>
          deleteWorkspace(connectionId, {
            workspace_id: saved.workspace_id,
            // The broker compares the name it stores; a workspace from before names has none.
            expected_name: snapshot.workspace.name ?? null,
          })
        ).then((done) => {
          pending.current = false;
          if (!done) return;
          toastSuccess({ msg: confirmCopy.deleteWorkspace.toast(workspace) });
          onClose();
        });
      }}
    >
      <ConfirmBody />
    </DangerousConfirmDialog>
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
      <ConfirmBody />
    </ConfirmationModal>
  );
}

/**
 * Whether removing a connection ends its workspace's host controls, as far as the verified view
 * says (CLI-1, the CLI's `host_standing`): `only-host` when the person hosts it and the workspace
 * lists no other computer of theirs, `member` when they do not host it, `unknown` otherwise (no
 * view of this connection, or the host with another listed computer, which is not proof that
 * computer can still act as host).
 */
export function removalStanding(
  snapshot: DialogView['snapshot'],
  viewerIsHost: boolean,
  ownDeviceId: string | null | undefined
): 'only-host' | 'member' | 'unknown' {
  if (!snapshot) return 'unknown';
  if (!viewerIsHost) return 'member';
  const digits = (text: string) =>
    Array.from(text)
      .filter((char) => /[0-9a-f]/i.test(char))
      .slice(0, 16)
      .join('')
      .toUpperCase();
  const own = ownDeviceId ? digits(ownDeviceId) : '';
  const devices = snapshot.actor.devices ?? [];
  const others =
    devices.length > 1
      ? devices.filter((device) => !own || digits(device.fingerprint) !== own)
      : [];
  return others.length === 0 ? 'only-host' : 'unknown';
}

function RemoveConnection({ connectionId, onClose }: { connectionId: string; onClose(): void }) {
  const { crew, workspace, phrase, snapshot, dir } = useDialogView(connectionId);
  const key = 'connection.remove';
  const saved = crew.connections.find((item) => item.id === connectionId) ?? null;
  // A workspace its host deleted (OW-5): nothing is left on the server to host, rejoin or lose
  // (L6, L7), so the removal says only what it does here.
  const deleted = isWorkspaceDeleted(saved);
  const standing = deleted
    ? 'deleted'
    : removalStanding(snapshot, dir.viewerIsHost, saved?.device_id);
  const username = dir.me?.username ?? sshUsername(saved?.ssh_target) ?? null;
  const remove = () =>
    void runConfirmed(crew, key, () => crew.removeConnection(connectionId)).then(
      (done) => done && onClose()
    );
  // The host's last computer: the workspace's name is typed first (CLI-1).
  if (standing === 'only-host')
    return (
      <DangerousConfirmDialog
        open
        title={confirmCopy.removeConnection.title(workspace)}
        description={confirmCopy.removeConnection.onlyHostDescription(workspace, username)}
        phrase={phrase}
        fieldLabel={confirmCopy.typeToConfirm(phrase)}
        confirmLabel={confirmCopy.removeConnection.onlyHostConfirm}
        cancelLabel={confirmCopy.cancel}
        busy={crew.isPending(key)}
        onConfirm={remove}
        onCancel={onClose}
      >
        <ConfirmBody />
      </DangerousConfirmDialog>
    );
  return (
    <ConfirmationModal
      isOpen
      title={confirmCopy.removeConnection.title(workspace)}
      message={
        standing === 'deleted'
          ? confirmCopy.removeConnection.deletedDescription(workspace)
          : standing === 'member'
            ? confirmCopy.removeConnection.memberDescription(workspace, username)
            : confirmCopy.removeConnection.description
      }
      confirmLabel={confirmCopy.removeConnection.confirm}
      cancelLabel={confirmCopy.cancel}
      confirmVariant="destructive"
      isSubmitting={crew.isPending(key)}
      onCancel={onClose}
      onConfirm={remove}
    >
      <ConfirmBody />
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
    >
      <AlertDialogRole />
    </ConfirmationModal>
  );
}
