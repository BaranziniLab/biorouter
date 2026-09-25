import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { AlertTriangle, Check } from '../../icons/app-icons';
import type { Team } from '../crewApi';
import { displayNameIsUsername, identityCopy, joinerPerson, teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { ChannelChoices } from './AddPeopleDialog';
import { addPeopleCopy, deviceCodeCopy, letInCopy as copy } from './copy';
import { DeviceCodeInput } from './DeviceCodeInput';
import { deviceCodeProblem } from './deviceCode';
import { DialogErrorNote, Field, helpId, useDialogError, useDismissOwnError } from './fields';
import { groupedFingerprint, useWorkspaceKeyFingerprint } from './fingerprint';
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
import './dialogs.css';

const SOURCE: ErrorSource = 'dialog:let-in';
const APPROVE_KEY = 'mutate:enrollment.approve';

/** The pending key of one team's addition. */
const teamKey = (teamId: string, directAdd: boolean) =>
  `mutate:${directAdd ? 'team.add_member' : 'invitation.create'}:${teamId}` as const;

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
  /**
   * The teams the person was already in when this view first knew them as a member — the teams it
   * does not offer. Fixed at that moment, never re-read: a team this dialog adds them to must keep
   * its row and the outcome that row says, not vanish on the next state frame because the person
   * is now in it (QA P0-2 / T-22). `null` until they are a member.
   */
  teamsAlreadyIn: ReadonlySet<string> | null;
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
 *   is the right one (QA T-13). The line becomes "{first} joined {workspace}" once the directory shows
 *   them, and a mismatch reported after saving brings the warning back with a way to re-enter it.
 * - Then the teams the host may add them to, available once they have joined: a broker that adds
 *   members directly (`direct_add_v1`) adds them — with the team's channels to choose from — and an
 *   older one invites them, which they accept in Crew. Each says which it did. The dialog's primary,
 *   focused action is that addition while it is still to do: "Add to {team}" in the footer for one
 *   team, or each team's own button for several, with "Not now" beside it. Done — which ignored the
 *   ticked channels and left the joiner in no team at all (QA Q2-03) — becomes the primary only
 *   once nothing is left to add. A person adding a device is already a member, so there Done stays
 *   primary and no optional channel starts ticked.
 * - Both views show the workspace key's fingerprint the joiner's Crew shows, to read to them if
 *   they ask (QA Q2-04), with nothing to copy: it is compared by eye, never sent.
 * - One shape and one name throughout (QA Q3-35, Q3-36). The joiner's `pending_joins` row — the
 *   only place the name on their server account comes from — goes away the moment they join, so
 *   the dialog remembers that name once it has seen it: the title ("Let Gina Rossi (@crew_gina)
 *   into …", the authority form), the "(name on the server account)" subtitle, the fingerprint and
 *   the hint row all stay when "joined" arrives. The two blocks whose words change then — the
 *   status note ("Code saved. …" → "{first} joined …", a sentence of two lines becoming one) and
 *   the hint row ("You can add … once … joins." → "Ticked channels …" or nothing) — are each laid
 *   out as tall as the taller of their sentences in both views, so nothing above the footer
 *   shrinks and "Add to {team}" stays under the host's pointer while it waits for them.
 *   Every sentence then calls them `{first}` — the first word of a name they chose, else of that
 *   server-account name, else `@username` — and never "they".
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
  // Per team: the outcome of adding them (which replaces the team's controls), and the channel
  // boxes the host changed from their starting state.
  const [outcomes, setOutcomes] = React.useState<Record<string, string>>({});
  const [checks, setChecks] = React.useState<Record<string, Record<string, boolean>>>({});
  const saved = crew.connections.find((item) => item.id === crew.connectionId) ?? null;
  const fingerprintHex = useWorkspaceKeyFingerprint(saved?.workspace_public_key);
  const fingerprint = fingerprintHex ? groupedFingerprint(fingerprintHex) : null;
  const join = snapshot?.pending_joins?.find((item) => item.username === username) ?? null;
  const offered = joinerPerson(username, join?.full_name);
  // The name on their server account, kept once seen: their `pending_joins` row — the only place
  // it comes from — is gone the moment they join, and the dialog must not lose it then (QA Q3-35).
  // Adjusted during render, so no frame draws the joined view without it.
  const [serverName, setServerName] = React.useState<string | null>(offered.serverName);
  if (offered.serverName && offered.serverName !== serverName) setServerName(offered.serverName);
  const person = offered.serverName ? offered : joinerPerson(username, serverName);
  // Once the person is a member (or is adding a device), the directory knows their chosen name.
  const member = dir.people.find((item) => item.username === username && !item.isFormer) ?? null;
  const named =
    member && !displayNameIsUsername(member.displayName, member.username) ? member : null;
  // One name for them everywhere in the dialog (QA Q3-36): a name they chose, else the name on
  // their server account, else `@username`.
  const fullName = named?.displayName ?? person.serverName;
  const first = firstName(named ?? person);
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
    const before: Approval = {
      memberBefore: member !== null,
      mismatchesBefore: mismatches,
      teamsAlreadyIn: null,
    };
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

  // A computer with a different code has tried already, so a code is saved: a plain approval
  // would only be refused as already approved. The form's own submit replaces it (QA Q2-23).
  const replacing = mismatches > 0;

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (deviceCodeProblem(code)) {
      setAttempted(true);
      return;
    }
    approve(code, replacing);
  };

  const handleText = (
    <bdi className="font-mono" translate="no">
      {handle}
    </bdi>
  );
  // The authority form, `Gina Rossi (@crew_gina)`, whenever a name is known: the title is where the
  // host checks WHO this is, so it names them in full beside the one name nobody can choose.
  const title = (
    <>
      {copy.titlePrefix}{' '}
      {fullName ? (
        <>
          <bdi>{fullName}</bdi> ({handleText})
        </>
      ) : (
        handleText
      )}{' '}
      {copy.title(workspace)}
    </>
  );
  const subtitle = person.serverName ? (
    <>
      <bdi>{person.serverName}</bdi> ({identityCopy.serverAccountName})
    </>
  ) : undefined;

  const fingerprintCheck = fingerprint ? (
    <FingerprintCheck who={first} fingerprint={fingerprint} />
  ) : null;

  if (approval) {
    const joined = !approval.memberBefore && member !== null;
    const newMismatch = !joined && join !== null && mismatches > approval.mismatchesBefore;
    const directAdd = directAddSupported(crew.capabilities);
    const memberId = member?.id ?? null;
    const inTeamsNow =
      memberId !== null
        ? new Set(
            (snapshot?.teams ?? [])
              .filter((team) => team.members.includes(memberId))
              .map((team) => team.id)
          )
        : null;
    // Fixed the first time the member id is known, which is before any team control can act (they
    // all wait for it), so nothing this dialog did can be in the set. Adjusting state during
    // render, not in an effect, so no frame ever draws the list from live membership.
    if (inTeamsNow && approval.teamsAlreadyIn === null) {
      setApproval({ ...approval, teamsAlreadyIn: inTeamsNow });
    }
    const alreadyIn = approval.teamsAlreadyIn ?? inTeamsNow;
    const teams = (snapshot?.teams ?? []).filter(
      (team) =>
        // Adding directly, the team's owner or the host may add; inviting, only its creator can.
        (team.created_by === dir.me?.id || (directAdd && dir.viewerIsHost)) &&
        !alreadyIn?.has(team.id)
    );
    // Someone adding a device is a member already: the team offers stay, but are not the point.
    const promote = !approval.memberBefore;
    const toDo = teams.filter((team) => outcomes[team.id] === undefined);
    const single = promote && teams.length === 1 && toDo.length === 1 ? toDo[0] : null;
    const several = promote && teams.length > 1 && toDo.length > 0;

    const choicesFor = (team: Team) => {
      const choices = directAdd ? directAddChannels(snapshot, team.id, dir) : [];
      // Never tick a box that the primary action ignores: where Done is the primary, an optional
      // channel starts unticked.
      return promote ? choices : choices.map((choice) => ({ ...choice, checked: choice.always }));
    };
    const isChecked = (team: Team, choice: ChannelChoice) =>
      choice.always || (checks[team.id]?.[choice.id] ?? choice.checked);
    const pendingFor = (team: Team) => crew.isPending(teamKey(team.id, directAdd));

    const addToTeam = (team: Team) => {
      if (!memberId || pendingFor(team)) return;
      const channelIds = choicesFor(team)
        .filter((choice) => !choice.always && isChecked(team, choice))
        .map((choice) => choice.id);
      const label = teamName(team);
      void crew
        .act(SOURCE, teamKey(team.id, directAdd), async () => {
          if (!directAdd) {
            await crew.request(
              'invitation.create',
              {
                kind: 'team',
                target_id: team.id,
                principal_id: memberId,
                expected_username: username,
              },
              { mutation: true }
            );
            return copy.addedToTeam(first);
          }
          const result = directAddResultFrom(
            await crew.request(
              'team.add_member',
              {
                team_id: team.id,
                principal_id: memberId,
                expected_username: username,
                ...(channelIds.length > 0 ? { channel_ids: channelIds } : {}),
              },
              { mutation: true }
            )
          );
          return result.alreadyMember && result.addedChannels.length === 0
            ? addPeopleCopy.alreadyIn(first, label)
            : copy.directAdded(
                first,
                label,
                channelsSeenAfterTeamAdd(snapshot, team.id, result.addedChannels)
              );
        })
        .then((said) => {
          if (said !== undefined) setOutcomes((current) => ({ ...current, [team.id]: said }));
        });
    };

    const teamLabel = (team: Team) =>
      directAdd
        ? copy.directAddToTeam(first, teamName(team))
        : copy.addToTeam(first, teamName(team));
    const waitingToJoin = !memberId;
    // Under the teams: what the host can do next with them.
    const hint = waitingToJoin
      ? copy.addAfterJoin(first)
      : toDo.some((team) => choicesFor(team).length > 1)
        ? copy.channelsWithTeam
        : null;
    // For a joiner, the row keeps its place while an addition is still to do, whatever it says —
    // "joined" can leave it nothing to say (one channel per team), and it must not go then.
    const holdHint = promote && toDo.length > 0;
    // Each footer layout is its own set of keyed elements: reconciled in place of the form's
    // Cancel (or of the add it replaces), React would reuse that node and never apply `autoFocus`.
    // The addition is `aria-disabled` rather than `disabled` while they have not joined yet, so it
    // can hold focus: the next state frame enables it where the host is already waiting.
    const footer = single ? (
      <>
        <Button key="not-now" type="button" variant="secondary" onClick={onClose}>
          {copy.notNow}
        </Button>
        <Button
          key={`add-${single.id}`}
          autoFocus
          className="crew-waiting-action"
          aria-disabled={waitingToJoin || pendingFor(single) || undefined}
          onClick={() => addToTeam(single)}
        >
          {directAdd ? copy.footerAdd(teamName(single)) : copy.footerInvite(teamName(single))}
        </Button>
      </>
    ) : several ? (
      <Button key="not-now" type="button" variant="secondary" onClick={onClose}>
        {toDo.length === teams.length ? copy.notNow : copy.done}
      </Button>
    ) : (
      <Button key="done" autoFocus onClick={onClose}>
        {copy.done}
      </Button>
    );

    return (
      <ModalShell
        open
        onOpenChange={(open) => !open && onClose()}
        size="sm"
        purpose="info"
        title={title}
        subtitle={subtitle}
        footer={footer}
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
            // Someone adding a device is a member already: no "joined" is coming to make room for.
            <SavedCodeStatus joined={joined} first={first} workspace={workspace} steady={promote} />
          )}
          {/* In both views, so it never vanishes while the host is reading it out (QA Q3-35). */}
          {fingerprintCheck}
          {teams.length > 0 ? (
            <div className="flex flex-col items-start gap-3">
              {teams.map((team) => {
                const outcome = outcomes[team.id];
                if (outcome !== undefined) return <TeamOutcome key={team.id} text={outcome} />;
                const choices = choicesFor(team);
                const pending = pendingFor(team);
                return (
                  <div key={team.id} className="flex min-w-0 flex-col items-start gap-2">
                    {choices.length > 1 ? (
                      <ChannelChoices
                        label={single ? addPeopleCopy.channels : copy.channelsIn(teamName(team))}
                        choices={choices}
                        isChecked={(choice) => isChecked(team, choice)}
                        disabled={pending || waitingToJoin}
                        onChange={(id, next) =>
                          setChecks((current) => ({
                            ...current,
                            [team.id]: { ...current[team.id], [id]: next },
                          }))
                        }
                      />
                    ) : null}
                    {single ? null : (
                      <Button
                        // With several, the first still to do takes the focus: the form's at first,
                        // then that of the team just added, whose button its outcome replaced. The
                        // key remounts the button as it becomes first, so `autoFocus` applies.
                        key={several && team.id === toDo[0]?.id ? 'first' : 'rest'}
                        variant={several ? 'default' : 'secondary'}
                        size="sm"
                        className="crew-waiting-action"
                        autoFocus={several && team.id === toDo[0]?.id}
                        aria-disabled={waitingToJoin || pending || undefined}
                        onClick={() => addToTeam(team)}
                      >
                        {teamLabel(team)}
                      </Button>
                    )}
                  </div>
                );
              })}
              {hint !== null || holdHint ? (
                <NextStepHint text={hint} reserve={holdHint ? nextStepReserve(first) : null} />
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
          <Button type="button" variant="secondary" onClick={onClose} disabled={approving}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={approving || !complete}>
            {replacing ? copy.replace : copy.submit(first)}
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
        {fingerprintCheck}
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
          render={(message) =>
            // In the dialog's one name for them, never "they" (QA Q3-36).
            isAlreadyApproved(message)
              ? copy.alreadyApproved(username, first)
              : approveRefusalText(message, username)
          }
        />
        {replaceCode !== null ? (
          <div className="flex flex-col items-start gap-2">
            <p className="text-supporting text-text-muted">{copy.replaceHelp(first)}</p>
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
 * The saved-code view's status: "Code saved. …" until the directory shows the joiner, then
 * "{first} joined {workspace}" (QA T-13). That is two lines becoming one, and the neutral note's
 * border becoming the success wash's none, while the host's pointer waits on the footer for "Add
 * to {team}" to enable (QA Q3-35). So when `steady` — a joiner is expected — the note shares one
 * grid cell with an unseen copy of its tallest form, the bordered neutral tone laid out with both
 * sentences, and the cell is as tall in both views.
 */
export function SavedCodeStatus({
  joined,
  first,
  workspace,
  steady,
}: {
  joined: boolean;
  first: string;
  workspace: string;
  steady: boolean;
}) {
  const approvedText = copy.approved(first);
  const joinedText = copy.joined(first, workspace);
  return (
    <div className="crew-steady">
      <Note
        tone={joined ? 'success' : 'neutral'}
        role="status"
        icon={Check}
        className="crew-steady-item"
      >
        <span>{joined ? joinedText : approvedText}</span>
      </Note>
      {steady ? (
        <div aria-hidden className="crew-steady-item crew-steady-sizer">
          <Note icon={Check}>
            <span
              className="crew-reserve"
              data-reserve-a={approvedText}
              data-reserve-b={joinedText}
            />
          </Note>
        </div>
      ) : null}
    </div>
  );
}

/** The two sentences the next-step row says for a joiner: waiting for them, then after they join. */
export function nextStepReserve(first: string): readonly [string, string] {
  return [copy.addAfterJoin(first), copy.channelsWithTeam];
}

/**
 * The row under the teams, saying what the host can do next. With a `reserve`, it is laid out as
 * tall as the taller of those sentences whichever it shows, or when it shows none (QA Q3-35).
 */
export function NextStepHint({
  text,
  reserve,
}: {
  text: string | null;
  reserve: readonly [string, string] | null;
}) {
  return (
    <p
      className="crew-reserve text-supporting text-text-muted"
      data-reserve-a={reserve?.[0]}
      data-reserve-b={reserve?.[1]}
    >
      <span className="crew-reserve-shown">{text}</span>
    </p>
  );
}

/** A team's addition, in words, where its controls were: added (and to what), or invited. */
function TeamOutcome({ text }: { text: string }) {
  return (
    <p role="status" className="flex items-start gap-1.5 text-supporting text-text-default">
      <Check aria-hidden className="mt-0.5 h-icon-row w-icon-row shrink-0 text-text-muted" />
      <span>{text}</span>
    </p>
  );
}

/**
 * The workspace key's fingerprint as the joiner's Crew shows it, for the host to read to them if
 * they ask (QA Q2-04). Read-only and never a Copy button: it is compared by eye, not sent — the
 * code is what the joiner sends.
 */
function FingerprintCheck({ who, fingerprint }: { who: string; fingerprint: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <p className="text-supporting text-text-default">
        {copy.fingerprintFor(who)}{' '}
        <span className="whitespace-nowrap font-mono" translate="no">
          {fingerprint}
        </span>
      </p>
      <p className="text-supporting text-text-muted">{copy.fingerprintHelper(who)}</p>
    </div>
  );
}
