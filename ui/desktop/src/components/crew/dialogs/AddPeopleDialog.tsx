import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { toastSuccess } from '../../../toasts';
import { channelName, personLabel, teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { addPeopleCopy as copy } from './copy';
import { DialogErrorNote, Field, labelId } from './fields';
import { addPeopleCandidates } from './people';
import { PersonPicker } from './PersonPicker';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:add-people';
const KEY = 'mutate:invitation.create';

export interface AddPeopleDialogProps {
  target: 'team' | 'channel';
  targetId: string;
  onClose(): void;
}

/**
 * Add people to a team or a channel (ui-redesign-spec, "Dialog inventory"; L4).
 *
 * The picker offers only people the invitation can apply to: not members, not people already
 * invited, and for a channel only members of its team (the broker requires that too). The choice is
 * sent as `invitation.create` with the person's `expected_username`, so a snapshot altered on the
 * way cannot redirect the invitation to someone else.
 */
export function AddPeopleDialog({ target, targetId, onClose }: AddPeopleDialogProps) {
  const { crew, snapshot, dir, workspace } = useDialogView();
  const formId = React.useId();
  const personId = `${formId}-person`;
  const [principalId, setPrincipalId] = React.useState<string | null>(null);
  const channel =
    target === 'channel' ? (snapshot?.channels.find((item) => item.id === targetId) ?? null) : null;
  const team =
    target === 'team'
      ? (snapshot?.teams.find((item) => item.id === targetId) ?? null)
      : channel
        ? (snapshot?.teams.find((item) => item.id === channel.team_id) ?? null)
        : null;
  const { candidates, others } = addPeopleCandidates(
    snapshot,
    dir,
    target === 'team'
      ? { kind: 'team', teamId: targetId }
      : { kind: 'channel', channelId: targetId }
  );
  const sending = crew.isPending(KEY);
  useCloseWhenMissing(
    snapshot !== null && (target === 'team' ? team === null : channel === null),
    onClose
  );

  const empty =
    others.length === 0
      ? copy.noOne(workspace)
      : target === 'channel'
        ? copy.allInTeam(teamName(team))
        : copy.allInWorkspace(workspace);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const person = candidates.find((item) => item.id === principalId);
    if (!person?.id) return;
    const label = personLabel(person, 'inline', dir);
    void crew
      .act(SOURCE, KEY, async () => {
        await crew.mutate('invitation.create', {
          kind: target,
          target_id: targetId,
          principal_id: person.id,
          expected_username: person.username,
        });
        return true as const;
      })
      .then((done) => {
        if (done !== true) return;
        toastSuccess({ msg: copy.sent(label) });
        onClose();
      });
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="md"
      purpose={sending ? 'required' : 'form'}
      title={
        target === 'channel'
          ? copy.titleChannel(channelName(channel))
          : copy.titleTeam(teamName(team))
      }
      footer={
        <>
          <Button type="button" variant="outline" onClick={onClose} disabled={sending}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={sending || !principalId}>
            {copy.submit}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        {candidates.length > 0 ? (
          <Field id={personId} label={copy.person}>
            <PersonPicker
              id={personId}
              labelledBy={labelId(personId)}
              label={copy.person}
              candidates={candidates}
              value={principalId}
              onChange={setPrincipalId}
              dir={dir}
            />
          </Field>
        ) : (
          <Note tone="neutral" role="status">
            {empty}
          </Note>
        )}
        <DialogErrorNote source={SOURCE} />
      </form>
    </ModalShell>
  );
}
