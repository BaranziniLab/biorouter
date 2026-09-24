import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { AlertTriangle, Check } from '../../icons/app-icons';
import { identityCopy, joinerPerson, teamName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import type { ErrorSource } from '../state/types';
import { letInCopy as copy, workspaceSettingsCopy } from './copy';
import { DeviceCodeInput } from './DeviceCodeInput';
import { deviceCodeProblem } from './deviceCode';
import { DialogErrorNote, Field, helpId, useDialogError, useDismissOwnError } from './fields';
import { firstName } from './people';
import { approveRefusalText, isAlreadyApproved } from './refusals';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:let-in';
const APPROVE_KEY = 'mutate:enrollment.approve';

export interface LetInDialogProps {
  /** The pending joiner's canonical username, from the host's `pending_joins`. */
  username: string;
  onClose(): void;
}

/**
 * Let someone in (ui-redesign-spec, "Invite and admit", naming slice S3a): the host pastes the
 * code the joiner's OWN computer computed and sent them, and **Let {first} in** sends
 * `enrollment.approve {username, code}`. The broker admits only the device whose key yields that
 * code, so a relayed or substituted key cannot join under this approval.
 *
 * - The screen never shows a code it was not given: there is no code in the snapshot to show, and
 *   the field holds only what the host typed or pasted.
 * - When a device with a different code has already tried to join as this person, that warning is
 *   shown before the field, so the host checks with the person before pasting anything.
 * - When a device was already let in for this person (`already_approved`), the dialog says so and
 *   offers **Replace code**, which sends the same `{username, code}` again with `replace: true`, as
 *   `biorouter crew enroll approve --replace` does. Nothing replaces an approval unasked.
 * - Success offers one click per team the host created: **Add {first} to {team}**, available once
 *   the person has joined, since a team invitation names a member.
 */
export function LetInDialog({ username, onClose }: LetInDialogProps) {
  const { crew, snapshot, dir, workspace } = useDialogView();
  const formId = React.useId();
  const codeId = `${formId}-code`;
  const [code, setCode] = React.useState('');
  const [attempted, setAttempted] = React.useState(false);
  const [approved, setApproved] = React.useState(false);
  // The code the last approval sent, so Replace code re-sends exactly that code and no other.
  const [sentCode, setSentCode] = React.useState<string | null>(null);
  const join = snapshot?.pending_joins?.find((item) => item.username === username) ?? null;
  const person = joinerPerson(username, join?.full_name);
  // Once the person is a member (or is adding a device), the directory knows their chosen name.
  const member = dir.people.find((item) => item.username === username && !item.isFormer) ?? null;
  const first = firstName(member ?? person);
  const approving = crew.isPending(APPROVE_KEY);
  const dismissOwnError = useDismissOwnError(SOURCE);
  const problem = code ? deviceCodeProblem(code) : null;
  const showProblem = attempted && problem !== null;
  const mismatched = (join?.mismatched_attempts ?? 0) > 0;
  const error = useDialogError(SOURCE);
  // Offered only while the dialog shows `already_approved` for the code it just sent.
  const replaceCode =
    sentCode !== null && error !== null && isAlreadyApproved(error) ? sentCode : null;

  const approve = (sent: string, replace: boolean) => {
    setSentCode(sent);
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
        setSentCode(null);
        setApproved(true);
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
        @{username}
      </bdi>{' '}
      {copy.title(workspace)}
    </>
  );
  const subtitle = person.serverName ? (
    <>
      <bdi>{person.serverName}</bdi> ({identityCopy.serverAccountName})
    </>
  ) : undefined;

  if (approved) {
    // Only a team's creator can invite to it, and a team the person is already in needs nothing.
    const myTeams = (snapshot?.teams ?? []).filter(
      (team) => team.created_by === dir.me?.id && !(member?.id && team.members.includes(member.id))
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
          <Note tone="success" role="status" icon={Check}>
            <span>{copy.approved(first)}</span>
          </Note>
          {myTeams.length > 0 ? (
            <div className="flex flex-col items-start gap-2">
              {myTeams.map((team) => (
                <AddToTeamButton
                  key={team.id}
                  teamId={team.id}
                  teamLabel={teamName(team)}
                  first={first}
                  principalId={member?.id ?? null}
                  username={username}
                />
              ))}
              {!member ? (
                <p className="text-supporting text-text-muted">{copy.addAfterJoin(first)}</p>
              ) : null}
            </div>
          ) : null}
          <DialogErrorNote source={SOURCE} />
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
          <Button type="submit" form={formId} disabled={approving}>
            {copy.submit(first)}
          </Button>
        </>
      }
    >
      <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pb-1">
        {mismatched ? (
          <Note tone="warning" role="status" icon={AlertTriangle}>
            <span>{workspaceSettingsCopy.otherDevice(username)}</span>
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

function AddToTeamButton({
  teamId,
  teamLabel,
  first,
  principalId,
  username,
}: {
  teamId: string;
  teamLabel: string;
  first: string;
  principalId: string | null;
  username: string;
}) {
  const crew = useCrew();
  const [sent, setSent] = React.useState(false);
  const key = `mutate:invitation.create:${teamId}`;
  if (sent) {
    return (
      <span className="flex items-center gap-1.5 text-supporting text-text-muted">
        <Check aria-hidden className="h-icon-row w-icon-row" />
        {copy.addedToTeam(first, teamLabel)}
      </span>
    );
  }
  return (
    <Button
      variant="secondary"
      size="sm"
      disabled={!principalId || crew.isPending(key)}
      onClick={() => {
        if (!principalId) return;
        void crew
          .act(SOURCE, key, async () => {
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
            return true as const;
          })
          .then((done) => done === true && setSent(true));
      }}
    >
      {copy.addToTeam(first, teamLabel)}
    </Button>
  );
}
