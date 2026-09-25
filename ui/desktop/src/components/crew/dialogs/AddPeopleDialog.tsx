import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Avatar } from '../../ui/avatar';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { Note } from '../../ui/note';
import { AlertTriangle, Check } from '../../icons/app-icons';
import { channelName, PersonName, teamName, type CrewPerson } from '../identity';
import { focusIsLost } from '../state/focusReturn';
import { failureMessage } from '../state/observationFailure';
import type { ErrorSource } from '../state/types';
import { addPeopleCopy as copy, dialogErrorCopy } from './copy';
import { DialogErrorNote } from './fields';
import {
  addPeopleCandidates,
  channelsSeenAfterTeamAdd,
  directAddChannels,
  directAddResultFrom,
  directAddSupported,
  listOf,
  targetMembers,
  usernameList,
  workspaceInvitees,
  type ChannelChoice,
  type DirectAddResult,
} from './people';
import { PersonChecklist } from './PersonPicker';
import { directAddRefusalText, refusalText } from './refusals';
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

/** What one person's addition came to, in a run of several. */
type Outcome =
  | { person: CrewPerson; ok: true; result: DirectAddResult | null }
  | { person: CrewPerson; ok: false; reason: string };

/** The summary a run of additions leaves in the dialog. */
interface Summary {
  text: string;
  failed: boolean;
}

/**
 * Add people to a team or a channel (ui-redesign-spec, "Dialog inventory"; L4).
 *
 * Several at once (QA Q2-05): a checklist of the people who may be added, with a search and
 * "Select all", and one "Add {n} people" that sends one request per person — the broker's shape —
 * and says in one line, in the dialog, who was added and who couldn't be and why. One refusal never
 * stops the others. The dialog stays open until Done, so the next few can follow.
 *
 * Two brokers, two honest outcomes:
 * - One that adds members directly (`direct_add_v1` in its hello) gets `team.add_member` or
 *   `channel.add_member`: the person is in when it answers, and the dialog says which channels
 *   they can now see. A team addition also offers the team's other channels the host may add
 *   people to, the ones they own checked; #general always comes with the team.
 * - An older one gets `invitation.create`, which the person must accept in Crew. Nothing here says
 *   they are in: the result, and every waiting invitation, reads "invited, not accepted yet".
 *
 * Either way each person is sent with their `expected_username`, so a snapshot altered on the way
 * cannot redirect it to someone else, and the broker decides whether the viewer may add them.
 *
 * Never a dead end (QA Q2-22): someone who may not add people here is told who may; with no one left
 * to add, the dialog lists who is already in, names the workspace's invitees who have not joined,
 * offers the next step, and shows one Done — no disabled Add.
 *
 * For a team, someone who may not add people there gets the team's member list instead (QA Q3-44):
 * "Members of {team}", the members first — host, you, then by name — then, muted, who may add
 * people, and one Done. It is what the team menu's "Members of {team}…" opens, for everyone; the
 * owner and the host see the Add people dialog there.
 */
export function AddPeopleDialog({ target, targetId, onClose }: AddPeopleDialogProps) {
  const { crew, snapshot, dir, workspace } = useDialogView();
  const formId = React.useId();
  const labelId = `${formId}-label`;
  const noteId = `${formId}-note`;
  const [selected, setSelected] = React.useState<string[]>([]);
  const [checked, setChecked] = React.useState<Record<string, boolean>>({});
  // The people this dialog added, kept out of the list until the next state frame shows them in.
  const [added, setAdded] = React.useState<ReadonlySet<string>>(() => new Set());
  const [summary, setSummary] = React.useState<Summary | null>(null);
  const searchRef = React.useRef<HTMLInputElement>(null);
  const directAdd = directAddSupported(crew.capabilities);
  const channel =
    target === 'channel' ? (snapshot?.channels.find((item) => item.id === targetId) ?? null) : null;
  const team =
    target === 'team'
      ? (snapshot?.teams.find((item) => item.id === targetId) ?? null)
      : channel
        ? (snapshot?.teams.find((item) => item.id === channel.team_id) ?? null)
        : null;
  const pickerTarget =
    target === 'team'
      ? ({ kind: 'team', teamId: targetId } as const)
      : ({ kind: 'channel', channelId: targetId } as const);
  const { candidates, others, pending, pendingTeam } = addPeopleCandidates(
    snapshot,
    dir,
    pickerTarget,
    { directAdd }
  );
  const offered = candidates.filter((person) => !added.has(person.id as string));
  const chosen = offered.filter((person) => selected.includes(person.id as string));
  const choices = directAdd && target === 'team' ? directAddChannels(snapshot, targetId, dir) : [];
  const key = !directAdd ? INVITE_KEY : target === 'team' ? TEAM_ADD_KEY : CHANNEL_ADD_KEY;
  const sending = crew.isPending(key);
  const place = target === 'channel' ? channelName(channel) : teamName(team);
  const invitees = workspaceInvitees(snapshot);
  useCloseWhenMissing(
    snapshot !== null && (target === 'team' ? team === null : channel === null),
    onClose
  );

  // The broker's rule, said before anyone is picked: adding directly, the owner (a team's creator)
  // or the host; inviting, the owner only. Unknown ownership is left to the broker.
  const ownerId = target === 'team' ? team?.created_by : channel?.owner_id;
  const owner = ownerId ? dir.byId(ownerId) : null;
  const mayAdd = !ownerId || ownerId === dir.me?.id || (directAdd && dir.viewerIsHost);
  // A team's member list, for someone who may not add people to it (QA Q3-44).
  const membersView = target === 'team' && !mayAdd;
  const onlyWho = directAdd
    ? copy.onlyOwnerOrHost(owner ? `@${owner.username}` : null, place)
    : copy.onlyOwner(owner ? `@${owner.username}` : null, place);

  const isChecked = (choice: ChannelChoice) =>
    choice.always || (checked[choice.id] ?? choice.checked);

  const summarize = (outcomes: readonly Outcome[]): Summary => {
    const done = outcomes.filter(
      (outcome): outcome is Extract<Outcome, { ok: true }> => outcome.ok
    );
    const already = done.filter(
      (outcome) =>
        outcome.result?.alreadyMember === true &&
        (target === 'channel' || outcome.result.addedChannels.length === 0)
    );
    const landed = done.filter((outcome) => !already.includes(outcome));
    const names = (list: readonly { person: CrewPerson }[]) =>
      usernameList(list.map((outcome) => outcome.person));
    const parts: string[] = [];
    if (landed.length > 0) {
      if (!directAdd) parts.push(copy.invitedMany(names(landed)));
      else if (target === 'channel') parts.push(copy.addedToChannel(names(landed), place));
      else {
        const channels = [
          ...new Set(landed.flatMap((outcome) => outcome.result?.addedChannels ?? [])),
        ];
        parts.push(
          copy.addedToTeam(
            names(landed),
            place,
            channelsSeenAfterTeamAdd(snapshot, targetId, channels)
          )
        );
      }
    }
    if (already.length === 1) parts.push(copy.alreadyIn(names(already), place));
    else if (already.length > 1) parts.push(copy.alreadyInMany(names(already), place));
    // One sentence per reason, so ten people refused for the same reason read as one line.
    const reasons = new Map<string, CrewPerson[]>();
    for (const outcome of outcomes) {
      if (outcome.ok) continue;
      reasons.set(outcome.reason, [...(reasons.get(outcome.reason) ?? []), outcome.person]);
    }
    for (const [reason, people] of reasons)
      parts.push(copy.couldNotAdd(usernameList(people), reason));
    return { text: parts.join(' '), failed: reasons.size > 0 };
  };

  // After a run, the Add that had focus is disabled (no one left ticked) or gone: carry on from the
  // search, or from Done where no one is left (its `autoFocus`), never from nowhere.
  React.useEffect(() => {
    if (!summary) return;
    const active = document.activeElement;
    if (focusIsLost() || (active instanceof HTMLButtonElement && active.disabled))
      searchRef.current?.focus();
  }, [summary]);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const people = chosen;
    if (people.length === 0 || sending) return;
    const channelIds = choices
      .filter((choice) => !choice.always && isChecked(choice))
      .map((choice) => choice.id);
    const words = directAdd ? directAddRefusalText : refusalText;
    void crew
      .act(SOURCE, key, async () => {
        const outcomes: Outcome[] = [];
        // One at a time, in the order shown: each is its own request, and each its own answer.
        for (const person of people) {
          const params = !directAdd
            ? {
                kind: target,
                target_id: targetId,
                principal_id: person.id,
                expected_username: person.username,
              }
            : target === 'team'
              ? {
                  team_id: targetId,
                  principal_id: person.id,
                  expected_username: person.username,
                  ...(channelIds.length > 0 ? { channel_ids: channelIds } : {}),
                }
              : {
                  channel_id: targetId,
                  principal_id: person.id,
                  expected_username: person.username,
                };
          const method = !directAdd
            ? 'invitation.create'
            : target === 'team'
              ? 'team.add_member'
              : 'channel.add_member';
          try {
            const answer = await crew.request(method, params, { mutation: true });
            outcomes.push({
              person,
              ok: true,
              result: directAdd ? directAddResultFrom(answer) : null,
            });
          } catch (failure) {
            outcomes.push({
              person,
              ok: false,
              reason: words(failureMessage(failure, dialogErrorCopy.fallback)),
            });
          }
        }
        return outcomes;
      })
      .then((outcomes) => {
        if (!outcomes) return;
        const landed = outcomes.filter((outcome) => outcome.ok).map((outcome) => outcome.person.id);
        setAdded((current) => new Set([...current, ...(landed as string[])]));
        setSelected((current) => current.filter((id) => !landed.includes(id)));
        setSummary(summarize(outcomes));
      });
  };

  /** Why there is no one to pick, and the way on from here. */
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

  // A body that is only a message is the dialog's description (QA Q2-28).
  const message: { text: string; action?: React.ReactNode } | null = !mayAdd
    ? { text: onlyWho }
    : offered.length === 0
      ? emptyState()
      : null;
  const members = message ? targetMembers(snapshot, dir, pickerTarget, added) : [];
  const inviteesLine =
    invitees.length > 0
      ? copy.invitedNotJoined(workspace, listOf(invitees.map((join) => `@${join.username}`)))
      : null;

  const footer = message ? (
    // Takes the focus when it replaces the form the last run emptied; on open, the dialog's own
    // first control does.
    <Button key="done" type="button" autoFocus={summary !== null} onClick={onClose}>
      {copy.done}
    </Button>
  ) : (
    <>
      <Button type="button" variant="secondary" onClick={onClose} disabled={sending}>
        {summary ? copy.done : copy.cancel}
      </Button>
      <Button type="submit" form={formId} disabled={sending || chosen.length === 0}>
        {chosen.length > 1 ? copy.addMany(chosen.length) : copy.submit}
      </Button>
    </>
  );

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="md"
      purpose={sending ? 'required' : 'form'}
      title={
        target === 'channel'
          ? copy.titleChannel(channelName(channel))
          : membersView
            ? copy.membersOf(teamName(team))
            : copy.titleTeam(teamName(team))
      }
      describedBy={message ? noteId : undefined}
      footer={footer}
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        {/* Always mounted, so each run's summary is announced as it lands. */}
        <div role="status" aria-live="polite">
          {summary ? (
            <Note
              tone={summary.failed ? 'warning' : 'success'}
              icon={summary.failed ? AlertTriangle : Check}
            >
              <span>{summary.text}</span>
            </Note>
          ) : null}
        </div>
        {membersView ? (
          <>
            {/* The list first: it is what this dialog is for here. Who may add people follows,
                muted, as its description. */}
            <MemberList place={place} people={members} dir={dir} label={copy.membersOf(place)} />
            <p id={noteId} className="text-supporting text-text-muted">
              {onlyWho}
            </p>
          </>
        ) : message ? (
          <>
            <Note tone="neutral" action={message.action}>
              <span id={noteId}>{message.text}</span>
            </Note>
            {inviteesLine ? (
              <p className="text-supporting text-text-muted">{inviteesLine}</p>
            ) : null}
            <MemberList place={place} people={members} dir={dir} />
          </>
        ) : (
          <>
            <div className="flex min-w-0 flex-col gap-1.5">
              <span id={labelId} className="text-label text-text-default">
                {copy.people}
              </span>
              <PersonChecklist
                candidates={offered}
                selected={selected}
                onChange={setSelected}
                label={copy.people}
                labelledBy={labelId}
                dir={dir}
                disabled={sending}
                searchRef={searchRef}
              />
            </div>
            {!directAdd && pending.length > 0 ? (
              <p className="text-supporting text-text-muted">
                {copy.waiting(usernameList(pending))}
              </p>
            ) : null}
            {inviteesLine ? (
              <p className="text-supporting text-text-muted">{inviteesLine}</p>
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
        )}
        <DialogErrorNote source={SOURCE} render={directAdd ? directAddRefusalText : refusalText} />
      </form>
    </ModalShell>
  );
}

/**
 * Who is in the team or channel already: host, then you, then by name. Under its caps "Already in
 * {place}" label; with `label`, the dialog's title already names it, so the list is named without
 * a second heading (QA Q3-44).
 */
function MemberList({
  place,
  people,
  dir,
  label,
}: {
  place: string;
  people: readonly CrewPerson[];
  dir: ReturnType<typeof useDialogView>['dir'];
  label?: string;
}) {
  const headingId = React.useId();
  if (people.length === 0) return null;
  const list = (
    <ul role="list" aria-label={label} className="crew-person-checklist flex min-w-0 flex-col">
      {people.map((person) => (
        <li
          key={person.id ?? person.username}
          className="flex min-w-0 items-center gap-2 px-1 py-1 text-label"
        >
          <Avatar
            size={20}
            fallback={person.avatar}
            name={person.displayName}
            username={person.username}
          />
          <PersonName
            person={person}
            context="header"
            dir={dir}
            you={person.isYou}
            className="min-w-0 flex-1 truncate"
          />
        </li>
      ))}
    </ul>
  );
  if (label) return list;
  return (
    <section aria-labelledby={headingId} className="flex min-w-0 flex-col gap-1.5">
      <h3 id={headingId} className="text-caps text-text-muted">
        {copy.alreadyInPlace(place)}
      </h3>
      {list}
    </section>
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
