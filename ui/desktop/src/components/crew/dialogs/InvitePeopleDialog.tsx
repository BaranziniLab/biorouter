import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { InfoTip } from '../../ui/info-tip';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { Skeleton } from '../../ui/skeleton';
import { Switch } from '../../ui/switch';
import { AlertTriangle } from '../../icons/app-icons';
import { splitFirstSentence } from '../../../utils/proseFold';
import { getInvitation, type CrewEnrollmentInvite } from '../api/join';
import { ACCOUNT_IDENTITY_CAPABILITY } from '../crewApi';
import { joinerPerson, PersonName } from '../identity';
import type { ErrorSource } from '../state/types';
import { inviteCopy as copy } from './copy';
import { AdornedInput, DialogErrorNote, Field, helpId, useDialogError } from './fields';
import { enrollmentInviteFrom, legacyTokenFrom, parseJoinRequest } from './joinRequest';
import { expiryPhrase, firstName } from './people';
import { inviteRefusal, newRouteFailureText, refusalText } from './refusals';
import { useDialogView } from './workspace';

const SOURCE: ErrorSource = 'dialog:invite-people';
const INVITE_KEY = 'mutate:enrollment.invite';

/** `@bob` → `bob`: the broker strips one leading `@` too, but the field shows it as an adornment. */
function typedUsername(value: string): string {
  return value.trim().replace(/^@/, '');
}

/**
 * What the field keeps of what was typed or pasted: without any leading `@`, because the field
 * already shows one, and "@crew_frank" read as "@ @crew_frank" with no warning (QA Q2-24).
 */
export function withoutLeadingAt(value: string): string {
  return value.replace(/^\s*@+/, '');
}

type Invitation =
  | { state: 'loading' }
  | { state: 'ready'; message: string }
  | { state: 'failed'; text: string };

export interface InvitePeopleDialogProps {
  onClose(): void;
}

/**
 * Invite people (ui-redesign-spec, "Invite and admit", naming slice S3a): the host types one
 * `@username` and presses **Invite**. The result renders the broker's own answer — the canonical
 * username and the name on that server account — never a preview of what was typed, then the
 * message to send, built by the host's daemon from its own verified connection.
 *
 * Nothing is looked up per keystroke (a name lookup can block the broker on LDAP). A refusal says
 * why in the copy deck's words where it has them, and offers the account's exact spelling rather
 * than accepting a near miss. Older brokers keep the token path under its own disclosure.
 *
 * The server is named as the person names it — "their login on lab-server", never its address
 * (QA Q4-34). The result says when the invitation expires (QA Q4-36), and what to do if the joiner
 * sees "Crew isn't set up" stays collapsed, with nothing to decide about it now (QA Q4-37).
 */
export function InvitePeopleDialog({ onClose }: InvitePeopleDialogProps) {
  const { crew, dir, workspace, server } = useDialogView();
  const formId = React.useId();
  const usernameId = `${formId}-username`;
  const deviceId = `${formId}-device`;
  const [username, setUsername] = React.useState('');
  const [addDevice, setAddDevice] = React.useState(false);
  const [lastAction, setLastAction] = React.useState<'invite' | 'legacy'>('invite');
  const [result, setResult] = React.useState<CrewEnrollmentInvite | null>(null);
  const [invitation, setInvitation] = React.useState<Invitation>({ state: 'loading' });
  const inviting = crew.isPending(INVITE_KEY);
  const error = useDialogError(SOURCE);
  const refusal =
    error && lastAction === 'invite'
      ? inviteRefusal(error, typedUsername(username), workspace)
      : null;
  const typed = typedUsername(username);
  const refusalId = `${formId}-refusal`;
  // A workspace that belongs to accounts (account ownership): a member's new computer gets in by
  // signing in to the server as them, so Add another device is never offered there, and nobody is
  // told to wait for a code. The saved connection's `account_identity` (its verified hello) says
  // so too, before a state frame has carried the capabilities, as Keys reads it.
  const accountBroker =
    crew.capabilities?.includes(ACCOUNT_IDENTITY_CAPABILITY) === true ||
    crew.connection?.account_identity === true;
  // The broker refused a plain invitation for a member (F3): the way on is Add another device,
  // which the refusal's sentence names, so it is turned on for the next Invite.
  const alreadyMember = refusal?.alreadyMember === true;
  React.useEffect(() => {
    if (alreadyMember && !accountBroker) setAddDevice(true);
  }, [alreadyMember, accountBroker]);
  const usernameRef = React.useRef<HTMLInputElement>(null);
  const [again, setAgain] = React.useState(false);

  // "Invite another" puts focus back in the empty field it returns to.
  React.useEffect(() => {
    if (!again || result) return;
    usernameRef.current?.focus();
    setAgain(false);
  }, [again, result]);

  const inviteAnother = () => {
    setResult(null);
    setUsername('');
    setAddDevice(false);
    setInvitation({ state: 'loading' });
    setAgain(true);
  };

  const loadInvitation = React.useCallback(
    (invitee: string) => {
      setInvitation({ state: 'loading' });
      getInvitation(crew.connectionId, invitee)
        .then(({ message }) => setInvitation({ state: 'ready', message }))
        .catch((failure: unknown) =>
          setInvitation({
            state: 'failed',
            text: newRouteFailureText(failure, copy.invitationUnavailable),
          })
        );
    },
    [crew.connectionId]
  );

  const invite = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = typedUsername(username);
    setLastAction('invite');
    void crew
      .act(SOURCE, INVITE_KEY, async () => {
        const answer = await crew.request<unknown>(
          'enrollment.invite',
          addDevice ? { username: name, add_device: true } : { username: name },
          { mutation: true }
        );
        return enrollmentInviteFrom(answer);
      })
      .then((invited) => {
        if (!invited) return;
        setResult(invited);
        loadInvitation(invited.username);
      });
  };

  if (result) {
    const person = joinerPerson(result.username, result.full_name);
    const first = firstName(person);
    // When the invitation runs out, after what to do next (QA Q4-36).
    const expires = expiryPhrase(result.expires_at);
    return (
      <ModalShell
        open
        onOpenChange={(open) => !open && onClose()}
        size="md"
        // `form`, so a stray click on the backdrop cannot throw away the message to send.
        purpose="form"
        title={copy.title(workspace)}
        footer={
          <>
            {/* One at a time is the broker's shape; this keeps the host in the dialog for the next. */}
            <Button key="another" type="button" variant="secondary" onClick={inviteAnother}>
              {copy.inviteAnother}
            </Button>
            {/* The form that had focus is gone; land on the result's main action, not the dialog
                frame. The key makes it a new element: reconciled in place of the form's Cancel,
                React would reuse that node and never apply `autoFocus`. */}
            <Button key="done" autoFocus onClick={onClose}>
              {copy.done}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-4 pb-1">
          <p className="text-body text-text-default">
            <PersonName person={person} context="joiner" />
            <span className="text-text-muted">
              {' · '}
              {copy.invited}
            </span>
          </p>
          <div className="flex flex-col gap-1.5">
            {/* What the joiner's Biorouter does is the heading's info tip (owner, 2026-11-13).
                The server fingerprint to pass on stays a line under the invitation: the manual
                asks hosts to send it, and the dialog says so in view (F7). */}
            <p className="text-label text-text-default">
              {copy.sendInvitation(first)}{' '}
              <InfoTip
                label={copy.invitationLabel}
                help={copy.setsUp(first)}
                data-testid="crew-invite-sets-up"
              />
            </p>
            {invitation.state === 'ready' ? (
              <CopyField value={invitation.message} label={copy.invitationLabel} multiline />
            ) : invitation.state === 'loading' ? (
              <Skeleton className="h-20 w-full" />
            ) : (
              <Note
                tone="warning"
                role="alert"
                icon={AlertTriangle}
                action={
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => loadInvitation(result.username)}
                  >
                    {copy.retry}
                  </Button>
                }
              >
                <span>{invitation.text}</span>
              </Note>
            )}
            <p className="text-supporting text-text-muted">{copy.serverFingerprint(first)}</p>
          </div>
          {/* What to do if the joiner's Crew says it isn't set up: collapsed, in the joiner's own
              words, and handed to whoever runs the server (QA Q4-37). */}
          <Disclosure label={copy.installed(first)}>
            <div className="flex flex-col gap-2 pt-2">
              <p className="text-supporting text-text-muted">
                {copy.installLead(result.username, server)}
              </p>
              {/* One command per line, never broken mid-word ("biorouter–/crew" read as a hyphen to
                  type): the lines keep their shape and scroll sideways (`dialogs.css`). */}
              <CopyField
                value={copy.installCommands}
                label={copy.installCommandsLabel}
                multiline
                valueClassName="crew-command-lines"
              />
            </div>
          </Disclosure>
          <div className="flex flex-col gap-1">
            <p className="text-supporting text-text-muted" data-testid="crew-invite-next-step">
              {accountBroker ? (
                // How the joiner gets in, in view; an older Biorouter's code behind its info icon
                // (owner, 2026-11-13).
                <FirstSentenceWithHelp
                  text={copy.nextStepAccount(first)}
                  helpLabel={copy.nextStepAccountHelpLabel}
                />
              ) : (
                copy.nextStep(first)
              )}
            </p>
            {/* When the invitation runs out stays in view (QA Q4-36), as its own short meta line
                under what to do next rather than a second sentence in the same block
                (design.md 9.16). */}
            {expires ? (
              <p className="text-supporting text-text-muted" data-testid="crew-invite-expires">
                {copy.expires(expires)}
              </p>
            ) : null}
          </div>
        </div>
      </ModalShell>
    );
  }

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="md"
      purpose={inviting ? 'required' : 'form'}
      title={copy.title(workspace)}
      footer={
        <>
          <Button type="button" variant="secondary" onClick={onClose} disabled={inviting}>
            {copy.cancel}
          </Button>
          <Button type="submit" form={formId} disabled={inviting}>
            {copy.submit}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4 pb-1">
        <form id={formId} onSubmit={invite} className="flex flex-col gap-3">
          <Field
            id={usernameId}
            label={copy.username}
            helper={copy.loginHelper(server, dir.me?.username ?? null)}
          >
            <AdornedInput
              adornment="@"
              id={usernameId}
              ref={usernameRef}
              required
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              translate="no"
              placeholder={copy.loginPlaceholder(server)}
              aria-invalid={refusal ? true : undefined}
              // The refusal first, then the reminder of what a username is here (QA T-23).
              // A member's refusal describes the field with its reason and, word for word, what its
              // info icon holds, not with the icon's own name.
              aria-describedby={
                refusal?.alreadyMember
                  ? `${refusalId}-reason ${refusalId}-help ${helpId(usernameId)}`
                  : refusal
                    ? `${refusalId} ${helpId(usernameId)}`
                    : helpId(usernameId)
              }
              value={username}
              onChange={(event) => {
                setUsername(withoutLeadingAt(event.target.value));
                if (error) crew.dismissError();
              }}
            />
          </Field>
          {!accountBroker && (refusal?.alreadyMember || addDevice) ? (
            <div className="flex min-w-0 items-center justify-between gap-3">
              <label htmlFor={deviceId} className="text-label text-text-default">
                {typed ? copy.addDevice(typed) : copy.addDeviceUnnamed}
              </label>
              <Switch id={deviceId} checked={addDevice} onCheckedChange={setAddDevice} />
            </div>
          ) : null}
          <div id={refusalId}>
            {refusal?.alreadyMember ? (
              // The refusal's reason in view; what to do about it behind its info icon (owner,
              // 2026-11-13).
              <AlreadyMemberNote
                id={refusalId}
                text={
                  accountBroker ? copy.refusal.alreadyMemberAccount(typed, workspace) : refusal.text
                }
              />
            ) : (
              <DialogErrorNote
                source={SOURCE}
                render={(message) =>
                  lastAction === 'invite'
                    ? inviteRefusal(message, typedUsername(username), workspace).text
                    : refusalText(message)
                }
              />
            )}
          </div>
        </form>
        <LegacyInvite
          server={server}
          knownUsername={typedUsername(username) || null}
          onStart={() => setLastAction('legacy')}
        />
      </div>
    </ModalShell>
  );
}

/**
 * A sentence's first sentence in view and the rest behind an info icon beside it (design.md 9.16):
 * the words are the copy's own, cut where `splitFirstSentence` cuts them.
 */
function FirstSentenceWithHelp({
  text,
  helpLabel,
  id,
}: {
  text: string;
  helpLabel: string;
  /** When given, the first sentence is `{id}-reason` and the help's hidden node `{id}-help`. */
  id?: string;
}) {
  const { first, rest } = splitFirstSentence(text);
  return (
    <>
      <span id={id ? `${id}-reason` : undefined}>{first}</span>
      {rest ? (
        <>
          {' '}
          <InfoTip label={helpLabel} help={rest} id={id ? `${id}-help` : undefined} />
        </>
      ) : null}
    </>
  );
}

/**
 * The broker's refusal of a member (F3, AO-1), as `DialogErrorNote` draws a refusal: the reason in
 * view, and what to do about it behind the info icon beside it. Brought into view as it appears, so
 * Invite does not look like it did nothing.
 */
function AlreadyMemberNote({ text, id }: { text: string; id: string }) {
  const note = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    note.current?.scrollIntoView?.({ block: 'nearest' });
  }, [text]);
  return (
    <div ref={note}>
      <Note tone="danger" role="alert" icon={AlertTriangle} testId="crew-invite-already-member">
        <FirstSentenceWithHelp
          text={text}
          helpLabel={copy.refusal.alreadyMemberHelpLabel}
          id={id}
        />
      </Note>
    </div>
  );
}

/**
 * The token path for a joiner whose Biorouter (or whose host's broker) predates joining by name:
 * their join request plus their user ID on the server, which the host looks up themselves.
 */
function LegacyInvite({
  server,
  knownUsername,
  onStart,
}: {
  server: string;
  knownUsername: string | null;
  onStart(): void;
}) {
  const { crew, snapshot } = useDialogView();
  const formId = React.useId();
  const requestId = `${formId}-request`;
  const uidId = `${formId}-uid`;
  const deviceId = `${formId}-device`;
  const [request, setRequest] = React.useState('');
  const [uid, setUid] = React.useState('');
  const [addDevice, setAddDevice] = React.useState(false);
  const [token, setToken] = React.useState<string | null>(null);
  const requestRef = React.useRef<HTMLTextAreaElement>(null);
  const parsed = parseJoinRequest(request);
  const username = parsed?.username ?? knownUsername;
  const key = 'mutate:enrollment.invite:legacy';
  const pending = crew.isPending(key);
  const uidNumber = Number(uid);
  // Only an existing member's own UID can add a device; the broker checks the pairing again.
  const existing =
    uid && Number.isSafeInteger(uidNumber)
      ? (snapshot?.principals.find(
          (principal) => principal.uid === uidNumber && principal.active !== false
        ) ?? null)
      : null;

  React.useEffect(() => {
    requestRef.current?.setCustomValidity(
      request.trim() && !parsed ? copy.legacy.joinRequestInvalid : ''
    );
  }, [request, parsed]);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!parsed) return;
    onStart();
    void crew
      .act(SOURCE, key, async () => {
        const answer = await crew.request<unknown>(
          'enrollment.invite',
          {
            uid: uidNumber,
            public_key: parsed.publicKey,
            ...(existing && addDevice ? { existing_principal_id: existing.id } : {}),
          },
          { mutation: true }
        );
        return legacyTokenFrom(answer);
      })
      .then((created) => {
        if (created) setToken(created);
      });
  };

  const who = username ? `@${username}` : copy.legacy.someone;
  return (
    <Disclosure label={copy.legacy.toggle}>
      {token ? (
        <div className="flex flex-col gap-2 pt-2">
          <CopyField value={token} label={copy.legacy.tokenLabel} secret />
          <p className="text-supporting text-text-muted">{copy.legacy.sendToken(who)}</p>
        </div>
      ) : (
        <form id={formId} onSubmit={submit} className="flex flex-col gap-3 pt-2">
          <Field id={requestId} label={copy.legacy.joinRequest}>
            <textarea
              id={requestId}
              ref={requestRef}
              required
              rows={3}
              spellCheck={false}
              placeholder={copy.legacy.joinRequestPlaceholder}
              className="min-w-0 rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 font-mono text-code placeholder:text-text-muted"
              value={request}
              onChange={(event) => setRequest(event.target.value)}
            />
          </Field>
          <Field id={uidId} label={copy.legacy.userId(server)}>
            <Input
              id={uidId}
              type="number"
              inputMode="numeric"
              required
              min={1}
              step={1}
              aria-describedby={helpId(uidId)}
              value={uid}
              onChange={(event) => {
                setUid(event.target.value);
                setAddDevice(false);
              }}
            />
            <p id={helpId(uidId)} className="text-supporting text-text-muted">
              {copy.legacy.userIdHelp}
            </p>
            <CopyField
              value={copy.legacy.userIdCommand(username ?? 'USERNAME')}
              label={copy.legacy.userIdCommandLabel}
            />
          </Field>
          {existing ? (
            <div className="flex min-w-0 items-center justify-between gap-3">
              <label htmlFor={deviceId} className="text-label text-text-default">
                {copy.addDevice(existing.username)}
              </label>
              <Switch id={deviceId} checked={addDevice} onCheckedChange={setAddDevice} />
            </div>
          ) : null}
          <div>
            <Button type="submit" variant="secondary" disabled={pending}>
              {copy.legacy.submit}
            </Button>
          </div>
        </form>
      )}
    </Disclosure>
  );
}
