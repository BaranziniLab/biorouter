import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { Skeleton } from '../../ui/skeleton';
import { Switch } from '../../ui/switch';
import { AlertTriangle } from '../../icons/app-icons';
import { getInvitation, type CrewEnrollmentInvite } from '../api/join';
import { joinerPerson, PersonName } from '../identity';
import type { ErrorSource } from '../state/types';
import { inviteCopy as copy } from './copy';
import { AdornedInput, DialogErrorNote, Field, helpId, useDialogError } from './fields';
import { enrollmentInviteFrom, legacyTokenFrom, parseJoinRequest } from './joinRequest';
import { firstName } from './people';
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
            <p className="text-label text-text-default">{copy.sendInvitation(first)}</p>
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
          </div>
          <Disclosure label={copy.installed(result.username, server)}>
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
          <p className="text-supporting text-text-muted">{copy.nextStep(first)}</p>
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
              aria-describedby={refusal ? `${refusalId} ${helpId(usernameId)}` : helpId(usernameId)}
              value={username}
              onChange={(event) => {
                setUsername(withoutLeadingAt(event.target.value));
                if (error) crew.dismissError();
              }}
            />
          </Field>
          {refusal?.alreadyMember || addDevice ? (
            <div className="flex min-w-0 items-center justify-between gap-3">
              <label htmlFor={deviceId} className="text-label text-text-default">
                {typed ? copy.addDevice(typed) : copy.addDeviceUnnamed}
              </label>
              <Switch id={deviceId} checked={addDevice} onCheckedChange={setAddDevice} />
            </div>
          ) : null}
          <div id={refusalId}>
            <DialogErrorNote
              source={SOURCE}
              render={(message) =>
                lastAction === 'invite'
                  ? inviteRefusal(message, typedUsername(username), workspace).text
                  : refusalText(message)
              }
            />
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
              className="min-w-0 rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 font-mono text-supporting placeholder:text-text-muted"
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
