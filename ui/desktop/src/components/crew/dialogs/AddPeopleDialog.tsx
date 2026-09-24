import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { Note } from '../../ui/note';
import { toastSuccess } from '../../../toasts';
import { channelName, personLabel, teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { addPeopleCopy as copy } from './copy';
import { DialogErrorNote, Field, labelId } from './fields';
import {
  addPeopleCandidates,
  channelsSeenAfterTeamAdd,
  directAddChannels,
  directAddResultFrom,
  directAddSupported,
  usernameList,
  type ChannelChoice,
} from './people';
import { PersonPicker } from './PersonPicker';
import { useCloseWhenMissing } from './useCloseWhenMissing';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:add-people';
const INVITE_KEY = 'mutate:invitation.create';
const TEAM_ADD_KEY = 'mutate:team.add_member';
const CHANNEL_ADD_KEY = 'mutate:channel.add_member';

export interface AddPeopleDialogProps {
  target: 'team' | 'channel';
  targetId: string;
  onClose(): void;
}

/**
 * Add people to a team or a channel (ui-redesign-spec, "Dialog inventory"; L4).
 *
 * Two brokers, two honest outcomes:
 * - One that adds members directly (`direct_add_v1` in its hello) gets `team.add_member` or
 *   `channel.add_member`: the person is in when it answers, and the dialog says which channels
 *   they can now see. A team addition also offers the team's other channels the host may add
 *   people to, the ones they own checked; #general always comes with the team.
 * - An older one gets `invitation.create`, which the person must accept in Crew. Nothing here says
 *   they are in: the result, and every waiting invitation, reads "invited, not accepted yet".
 *
 * Either way the choice is sent with the person's `expected_username`, so a snapshot altered on the
 * way cannot redirect it to someone else, and the broker decides whether the viewer may add them.
 * An empty picker is never a dead end: it names who is still waiting and offers the next step —
 * inviting people to the workspace (the host), or adding them to the team first.
 */
export function AddPeopleDialog({ target, targetId, onClose }: AddPeopleDialogProps) {
  const { crew, snapshot, dir, workspace } = useDialogView();
  const formId = React.useId();
  const personId = `${formId}-person`;
  const [principalId, setPrincipalId] = React.useState<string | null>(null);
  const [checked, setChecked] = React.useState<Record<string, boolean>>({});
  const directAdd = directAddSupported(crew.capabilities);
  const channel =
    target === 'channel' ? (snapshot?.channels.find((item) => item.id === targetId) ?? null) : null;
  const team =
    target === 'team'
      ? (snapshot?.teams.find((item) => item.id === targetId) ?? null)
      : channel
        ? (snapshot?.teams.find((item) => item.id === channel.team_id) ?? null)
        : null;
  const { candidates, others, pending, pendingTeam } = addPeopleCandidates(
    snapshot,
    dir,
    target === 'team'
      ? { kind: 'team', teamId: targetId }
      : { kind: 'channel', channelId: targetId },
    { directAdd }
  );
  const choices = directAdd && target === 'team' ? directAddChannels(snapshot, targetId, dir) : [];
  const key = !directAdd ? INVITE_KEY : target === 'team' ? TEAM_ADD_KEY : CHANNEL_ADD_KEY;
  const sending = crew.isPending(key);
  useCloseWhenMissing(
    snapshot !== null && (target === 'team' ? team === null : channel === null),
    onClose
  );

  const isChecked = (choice: ChannelChoice) =>
    choice.always || (checked[choice.id] ?? choice.checked);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const person = candidates.find((item) => item.id === principalId);
    if (!person?.id) return;
    const label = personLabel(person, 'inline', dir);
    const channelIds = choices
      .filter((choice) => !choice.always && isChecked(choice))
      .map((choice) => choice.id);
    void crew
      .act(SOURCE, key, async () => {
        if (!directAdd) {
          await crew.mutate('invitation.create', {
            kind: target,
            target_id: targetId,
            principal_id: person.id,
            expected_username: person.username,
          });
          return copy.sent(label);
        }
        if (target === 'team') {
          const result = directAddResultFrom(
            await crew.mutate('team.add_member', {
              team_id: targetId,
              principal_id: person.id,
              expected_username: person.username,
              ...(channelIds.length > 0 ? { channel_ids: channelIds } : {}),
            })
          );
          return result.alreadyMember && result.addedChannels.length === 0
            ? copy.alreadyIn(label, teamName(team))
            : copy.added(label, channelsSeenAfterTeamAdd(snapshot, targetId, result.addedChannels));
        }
        const result = directAddResultFrom(
          await crew.mutate('channel.add_member', {
            channel_id: targetId,
            principal_id: person.id,
            expected_username: person.username,
          })
        );
        return result.alreadyMember
          ? copy.alreadyIn(label, channelName(channel))
          : copy.added(label, channelName(channel));
      })
      .then((said) => {
        if (said === undefined) return;
        toastSuccess({ msg: said });
        onClose();
      });
  };

  function emptyState(): { text: string; action?: React.ReactNode } {
    const inviteToWorkspace = dir.viewerIsHost ? (
      <Button
        type="button"
        variant="secondary"
        size="sm"
        onClick={() => crew.openDialog({ kind: 'invite-people' })}
      >
        {copy.inviteToWorkspace(workspace)}
      </Button>
    ) : undefined;
    const workspaceEmpty = dir.people.every((person) => person.isYou || person.isFormer);
    if (workspaceEmpty) return { text: copy.noOne(workspace), action: inviteToWorkspace };
    if (target === 'team') {
      if (!directAdd && pending.length > 0)
        return {
          text: copy.allInvited(workspace, teamName(team), usernameList(pending)),
          action: inviteToWorkspace,
        };
      return { text: copy.allInWorkspace(workspace), action: inviteToWorkspace };
    }
    if (others.length === 0) {
      const canAddToTeam = Boolean(team && (team.created_by === dir.me?.id || dir.viewerIsHost));
      return {
        text:
          !directAdd && pendingTeam.length > 0
            ? `${copy.noOneInTeam(teamName(team))} ${copy.waitingToAccept(usernameList(pendingTeam))}`
            : copy.noOneInTeam(teamName(team)),
        action:
          canAddToTeam && team ? (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() =>
                crew.openDialog({ kind: 'add-people', target: 'team', targetId: team.id })
              }
            >
              {copy.addToTeam(teamName(team))}
            </Button>
          ) : undefined,
      };
    }
    return {
      text:
        !directAdd && pending.length > 0
          ? `${copy.allInTeam(teamName(team))} ${copy.waiting(usernameList(pending))}`
          : copy.allInTeam(teamName(team)),
    };
  }

  const empty = candidates.length === 0 ? emptyState() : null;

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
        {empty === null ? (
          <>
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
            {!directAdd && pending.length > 0 ? (
              <p className="text-supporting text-text-muted">
                {copy.waiting(usernameList(pending))}
              </p>
            ) : null}
            {choices.length > 0 ? (
              <ChannelChoices
                choices={choices}
                isChecked={isChecked}
                disabled={sending}
                onChange={(id, next) => setChecked((current) => ({ ...current, [id]: next }))}
              />
            ) : null}
          </>
        ) : (
          <Note tone="neutral" role="status" action={empty.action}>
            {empty.text}
          </Note>
        )}
        <DialogErrorNote source={SOURCE} />
      </form>
    </ModalShell>
  );
}

/**
 * The channels a direct team addition also adds the person to: #general checked and fixed (it comes
 * with the team), the others as the caller chooses. A labelled group of real checkboxes, each
 * wrapped in its label so the whole row toggles it.
 */
export function ChannelChoices({
  choices,
  isChecked,
  onChange,
  disabled,
  label = copy.channels,
}: {
  choices: readonly ChannelChoice[];
  isChecked(choice: ChannelChoice): boolean;
  onChange(channelId: string, checked: boolean): void;
  disabled?: boolean;
  label?: string;
}) {
  const legendId = React.useId();
  return (
    <div role="group" aria-labelledby={legendId} className="flex min-w-0 flex-col gap-1">
      <span id={legendId} className="text-label text-text-default">
        {label}
      </span>
      {choices.map((choice) => (
        <label
          key={choice.id}
          className="flex min-w-0 items-center gap-2 text-body text-text-default"
        >
          <Checkbox
            checked={isChecked(choice)}
            disabled={disabled || choice.always}
            onChange={(event) => onChange(choice.id, event.target.checked)}
          />
          <span className="min-w-0 truncate" translate="no">
            {choice.label}
          </span>
          {choice.always ? (
            <span className="text-supporting text-text-muted">{copy.generalIncluded}</span>
          ) : null}
        </label>
      ))}
    </div>
  );
}
