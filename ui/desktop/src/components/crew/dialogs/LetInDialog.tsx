import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { AlertTriangle, Check } from '../../icons/app-icons';
import type { Snapshot } from '../crewApi';
import { identityCopy, joinerPerson, teamName, type PeopleDirectory } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import type { ErrorSource } from '../state/types';
import { ChannelChoices } from './AddPeopleDialog';
import { addPeopleCopy, deviceCodeCopy, letInCopy as copy } from './copy';
import { DeviceCodeInput } from './DeviceCodeInput';
import { deviceCodeProblem } from './deviceCode';
import { DialogErrorNote, Field, helpId, useDialogError, useDismissOwnError } from './fields';
import {
  channelsSeenAfterTeamAdd,
  directAddChannels,
  directAddResultFrom,
  directAddSupported,
  firstName,
  type ChannelChoice,
} from './people';
import {
  approveRefusalText,
  directAddRefusalText,
  isAlreadyApproved,
  refusalText,
} from './refusals';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:let-in';
const APPROVE_KEY = 'mutate:enrollment.approve';

export interface LetInDialogProps {
  /** The pending joiner's canonical username, from the host's `pending_joins`. */
  username: string;
  onClose(): void;
}

/** What the saved-code view compares against: who and what the directory showed at approval. */
interface Approval {
  /** The person was already a member (adding a device), so "joined" can never be inferred. */
  memberBefore: boolean;
  /** Mismatched attempts already counted, so only a NEW one warns. */
  mismatchesBefore: number;
}

/**
 * Let someone in (ui-redesign-spec, "Invite and admit", naming slice S3a): the host pastes the
 * code the joiner's OWN computer computed and sent them, and **Let {first} in** sends
 * `enrollment.approve {username, code}`. The broker admits only the device whose key yields that
 * code, so a relayed or substituted key cannot join under this approval.
 *
 * - The screen never shows a code it was not given: there is no code in the snapshot to show, and
 *   the field holds only what the host typed or pasted. **Let {first} in** stays disabled until
 *   the field holds a whole, well-formed code.
 * - When a computer with a different code has already tried to join as this person, that warning
 *   is shown before the field — with what to do about it — so the host checks with the person
 *   before pasting anything.
 * - When a device was already let in for this person (`already_approved`), the dialog says so and
 *   offers **Replace code**, which sends the same `{username, code}` again with `replace: true`, as
 *   `biorouter crew enroll approve --replace` does. Nothing replaces an approval unasked.
 * - Success is worded as what it is: the broker only saved the code, and cannot yet tell whether it
 *   is the right one (QA T-13). The line becomes "@x joined {workspace}" once the directory shows
 *   them, and a mismatch reported after saving brings the warning back with a way to re-enter it.
 * - Then one control per team the host may add them to, available once they have joined: a broker
 *   that adds members directly (`direct_add_v1`) adds them — with the team's channels to choose
 *   from — and an older one invites them, which they accept in Crew. Each says which it did.
 */
export function LetInDialog({ username, onClose }: LetInDialogProps) {
  const { crew, snapshot, dir, workspace } = useDialogView();
  const formId = React.useId();
  const codeId = `${formId}-code`;
  const [code, setCode] = React.useState('');
  const [attempted, setAttempted] = React.useState(false);
  const [approval, setApproval] = React.useState<Approval | null>(null);
  // The code the last approval sent, so Replace code re-sends exactly that code and no other.
  const [sentCode, setSentCode] = React.useState<string | null>(null);
  const join = snapshot?.pending_joins?.find((item) => item.username === username) ?? null;
  const person = joinerPerson(username, join?.full_name);
  // Once the person is a member (or is adding a device), the directory knows their chosen name.
  const member = dir.people.find((item) => item.username === username && !item.isFormer) ?? null;
  const first = firstName(member ?? person);
  const handle = `@${username}`;
  const approving = crew.isPending(APPROVE_KEY);
  const dismissOwnError = useDismissOwnError(SOURCE);
  const problem = code ? deviceCodeProblem(code) : null;
  // A character that can never be in a code is wrong the moment it is typed; a short code only
  // once the person has finished (left the field, or pressed Return).
  const showProblem = problem !== null && (attempted || problem !== deviceCodeCopy.wrongLength);
  const complete = code.length > 0 && problem === null;
  const mismatches = join?.mismatched_attempts ?? 0;
  const error = useDialogError(SOURCE);
  // Offered only while the dialog shows `already_approved` for the code it just sent.
  const replaceCode =
    sentCode !== null && error !== null && isAlreadyApproved(error) ? sentCode : null;

  const approve = (sent: string, replace: boolean) => {
    setSentCode(sent);
    const before: Approval = { memberBefore: member !== null, mismatchesBefore: mismatches };
    void crew
      .act(SOURCE, APPROVE_KEY, async () => {
        await crew.request(
          'enrollment.approve',
          replace ? { username, code: sent, replace: true } : { username, code: sent },
          { mutation: true }
        );
        return true as const;
      })
      .then((done) => {
        if (done !== true) return;
        // The code has done its job; drop it rather than keep it on screen.
        setCode('');
        setAttempted(false);
        setSentCode(null);
        setApproval(before);
      });
  };

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (deviceCodeProblem(code)) {
      setAttempted(true);
      return;
    }
    approve(code, false);
  };

  const title = (
    <>
      {copy.titlePrefix}{' '}
      <bdi className="font-mono" translate="no">
        {handle}
      </bdi>{' '}
      {copy.title(workspace)}
    </>
  );
  const subtitle = person.serverName ? (
    <>
      <bdi>{person.serverName}</bdi> ({identityCopy.serverAccountName})
    </>
  ) : undefined;

  if (approval) {
    const joined = !approval.memberBefore && member !== null;
    const newMismatch = !joined && join !== null && mismatches > approval.mismatchesBefore;
    const directAdd = directAddSupported(crew.capabilities);
    const teams = (snapshot?.teams ?? []).filter(
      (team) =>
        // Adding directly, the team's owner or the host may add; inviting, only its creator can.
        (team.created_by === dir.me?.id || (directAdd && dir.viewerIsHost)) &&
        !(member?.id && team.members.includes(member.id))
    );
    return (
      <ModalShell
        open
        onOpenChange={(open) => !open && onClose()}
        size="sm"
        purpose="info"
        title={title}
        subtitle={subtitle}
        footer={
          // The form that had focus is gone; land on the result's one action, not the dialog frame.
          // The key makes it a new element: reconciled in place of the form's Cancel, React would
          // reuse that node and never apply `autoFocus`.
          <Button key="done" autoFocus onClick={onClose}>
            {copy.done}
          </Button>
        }
      >
        <div className="flex flex-col gap-3 pb-1">
          {newMismatch ? (
            <Note
              tone="warning"
              role="alert"
              icon={AlertTriangle}
              action={
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    dismissOwnError();
                    setApproval(null);
                  }}
                >
                  {copy.enterAgain}
                </Button>
              }
            >
              <span>{copy.mismatch(username)}</span>
            </Note>
          ) : (
            <Note tone={joined ? 'success' : 'neutral'} role="status" icon={Check}>
              <span>{joined ? copy.joined(handle, workspace) : copy.approved(handle)}</span>
            </Note>
          )}
          {teams.length > 0 ? (
            <div className="flex flex-col items-start gap-3">
              {teams.map((team) => (
                <AddToTeam
                  key={team.id}
                  teamId={team.id}
                  teamLabel={teamName(team)}
                  who={handle}
                  principalId={member?.id ?? null}
                  username={username}
                  directAdd={directAdd}
                  snapshot={snapshot}
                  dir={dir}
                />
              ))}
              {!member ? (
                <p className="text-supporting text-text-muted">{copy.addAfterJoin(first)}</p>
              ) : null}
            </div>
          ) : null}
          {/* Here only the team additions can fail. */}
          <DialogErrorNote
            source={SOURCE}
            render={directAdd ? directAddRefusalText : refusalText}
          />
        </div>
      </ModalShell>
    );
  }

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="sm"
      purpose={approving ? 'required' : 'form'}
      title={title}
      subtitle={subtitle}
      footer={
        <>
          <Button type="button" variant="outline" onClick={onClose} disabled={approving}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={approving || !complete}>
            {copy.submit(first)}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        {mismatches > 0 ? (
          <Note tone="warning" role="status" icon={AlertTriangle}>
            <span>{copy.mismatch(username)}</span>
          </Note>
        ) : null}
        <Field
          id={codeId}
          label={copy.code(first)}
          helper={copy.helper(first)}
          error={showProblem ? problem : undefined}
        >
          <DeviceCodeInput
            id={codeId}
            required
            value={code}
            aria-invalid={showProblem || undefined}
            aria-describedby={helpId(codeId)}
            onBlur={() => setAttempted(code.length > 0)}
            onInvalid={() => setAttempted(true)}
            onKeyDown={(event) => {
              // Return with an incomplete code: the disabled submit ignores it, so say why here.
              if (event.key === 'Enter' && !complete) setAttempted(true);
            }}
            onChange={(next) => {
              setCode(next);
              dismissOwnError();
            }}
          />
        </Field>
        <DialogErrorNote
          source={SOURCE}
          render={(message) => approveRefusalText(message, username)}
        />
        {replaceCode !== null ? (
          <div className="flex flex-col items-start gap-2">
            <p className="text-supporting text-text-muted">{copy.replaceHelp}</p>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={approving}
              onClick={() => approve(replaceCode, true)}
            >
              {copy.replace}
            </Button>
          </div>
        ) : null}
      </form>
    </ModalShell>
  );
}

/**
 * One team the admitted person can be put in. A broker that adds directly gets `team.add_member`
 * with the checked channels; an older one gets a team invitation the person accepts in Crew. The
 * control is replaced by the outcome in words — added (and to which channels), or invited and
 * waiting for them — never a bare check mark.
 */
function AddToTeam({
  teamId,
  teamLabel,
  who,
  principalId,
  username,
  directAdd,
  snapshot,
  dir,
}: {
  teamId: string;
  teamLabel: string;
  who: string;
  principalId: string | null;
  username: string;
  directAdd: boolean;
  snapshot: Snapshot | null;
  dir: PeopleDirectory;
}) {
  const crew = useCrew();
  const [outcome, setOutcome] = React.useState<string | null>(null);
  const [checked, setChecked] = React.useState<Record<string, boolean>>({});
  const key = `mutate:${directAdd ? 'team.add_member' : 'invitation.create'}:${teamId}`;
  const pending = crew.isPending(key);
  const choices = directAdd ? directAddChannels(snapshot, teamId, dir) : [];
  const isChecked = (choice: ChannelChoice) =>
    choice.always || (checked[choice.id] ?? choice.checked);

  if (outcome) {
    return (
      <p role="status" className="flex items-start gap-1.5 text-supporting text-text-default">
        <Check aria-hidden className="mt-0.5 h-icon-row w-icon-row shrink-0 text-text-muted" />
        <span>{outcome}</span>
      </p>
    );
  }

  const add = () => {
    if (!principalId) return;
    const channelIds = choices
      .filter((choice) => !choice.always && isChecked(choice))
      .map((choice) => choice.id);
    void crew
      .act(SOURCE, key, async () => {
        if (!directAdd) {
          await crew.request(
            'invitation.create',
            {
              kind: 'team',
              target_id: teamId,
              principal_id: principalId,
              expected_username: username,
            },
            { mutation: true }
          );
          return copy.addedToTeam(who);
        }
        const result = directAddResultFrom(
          await crew.request(
            'team.add_member',
            {
              team_id: teamId,
              principal_id: principalId,
              expected_username: username,
              ...(channelIds.length > 0 ? { channel_ids: channelIds } : {}),
            },
            { mutation: true }
          )
        );
        return result.alreadyMember && result.addedChannels.length === 0
          ? addPeopleCopy.alreadyIn(who, teamLabel)
          : addPeopleCopy.added(
              who,
              channelsSeenAfterTeamAdd(snapshot, teamId, result.addedChannels)
            );
      })
      .then((said) => {
        if (said !== undefined) setOutcome(said);
      });
  };

  return (
    <div className="flex min-w-0 flex-col items-start gap-2">
      {choices.length > 1 ? (
        <ChannelChoices
          label={addPeopleCopy.channels}
          choices={choices}
          isChecked={isChecked}
          disabled={pending || !principalId}
          onChange={(id, next) => setChecked((current) => ({ ...current, [id]: next }))}
        />
      ) : null}
      <Button variant="secondary" size="sm" disabled={!principalId || pending} onClick={add}>
        {directAdd ? copy.directAddToTeam(who, teamLabel) : copy.addToTeam(who, teamLabel)}
      </Button>
    </div>
  );
}
