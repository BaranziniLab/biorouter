import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import {
  previewInvitation,
  saveFromInvitation,
  type CrewInvitationAdvanced,
  type CrewInvitationOverrides,
  type CrewInvitationPreview,
} from '../api/join';
import { CREW_INVITATION_INVALID, crewErrorCode, isStaleDaemon } from '../api/errors';
import type { CrewConnection } from '../crewApi';
import { PersonName, personFromProjection, sanitizeDisplayText } from '../identity';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import type { SaveConnectionInput } from '../state/types';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { joinCopy } from './copy';
import { Field, PrivacyFields, SwitchRow, useMounted, useOpenGeneration } from './fields';
import { updateJoinContext } from './joinContext';
import { groupWorkspaceFingerprint } from './joinText';
import { PrivacyLabel } from './parts';

type Mode = 'private' | 'public';

type PreviewState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'ready'; preview: CrewInvitationPreview }
  | { kind: 'invalid' }
  | { kind: 'stale' }
  | { kind: 'failed'; message: string };

/** How long typing pauses before the pasted text is sent to the daemon for a preview. */
export const INVITATION_PREVIEW_DELAY_MS = 250;

/** The manual workspace key: 64 hex characters (fixes L8). */
export const WORKSPACE_KEY_PATTERN = '[a-fA-F0-9]{64}';

/**
 * A server login override: what the daemon accepts as an SSH target (`safe_atom` in
 * `crew/mod.rs`) — letters, digits and `_ . / : @ % -`, not starting with a dash. Written for the
 * `pattern` attribute, which browsers compile with the `v` flag.
 */
export const SSH_LOGIN_PATTERN = String.raw`[A-Za-z0-9_.\/:@%][A-Za-z0-9_.\/:@%\-]*`;
const SSH_LOGIN = /^[A-Za-z0-9_./:@%][A-Za-z0-9_./:@%-]*$/;

/** Whether a server login override holds a value the daemon would refuse. Empty is fine. */
export function serverLoginInvalid(value: string): boolean {
  const login = value.trim();
  return Boolean(login) && !SSH_LOGIN.test(login);
}

/** Whether an Advanced port or work folder holds a value its field would refuse. */
export function advancedInvalid(port: string, remoteRoot: string): boolean {
  const portText = port.trim();
  if (portText) {
    const value = Number(portText);
    if (!Number.isInteger(value) || value < 1 || value > 65535) return true;
  }
  const folder = remoteRoot.trim();
  return Boolean(folder) && !folder.startsWith('/');
}

export interface JoinDialogProps {
  /** Defaults to the controller's dialog intent (`{kind: 'join'}`). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/**
 * Join a workspace (naming slice S3a). One visible field: the invitation the host sent. The daemon
 * parses it — this dialog never decodes an invitation itself — and the summary shows who hosts the
 * workspace, where, its privacy and its fingerprint. "Join {workspace}" saves the connection pinned
 * exactly as the invitation says, then connects; a password or code prompt opens Sign in by itself,
 * and the channel column carries the join from there.
 */
export function JoinDialog({ open, onOpenChange }: JoinDialogProps) {
  const crew = useCrew();
  const isOpen = open ?? crew.ui.dialog?.kind === 'join';
  const generation = useOpenGeneration(isOpen);
  const close = () => {
    if (onOpenChange) onOpenChange(false);
    else crew.closeDialog();
  };
  return <JoinDialogView key={generation} open={isOpen} onClose={close} />;
}

function JoinDialogView({ open, onClose }: { open: boolean; onClose: () => void }) {
  const crew = useCrew();
  const formId = useId();
  const formRef = useRef<HTMLFormElement>(null);
  const mounted = useMounted();

  const [invitation, setInvitation] = useState('');
  const [previewState, setPreviewState] = useState<PreviewState>({ kind: 'idle' });
  const preview = previewState.kind === 'ready' ? previewState.preview : null;

  // The person's own edits win over what the invitation prefills.
  const [username, setUsername] = useState('');
  const [usernameEdited, setUsernameEdited] = useState(false);
  const [mode, setMode] = useState<Mode>('private');
  const [institution, setInstitution] = useState('');
  const [privacyEdited, setPrivacyEdited] = useState(false);
  const [privacyOpen, setPrivacyOpen] = useState(false);

  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [sshAlias, setSshAlias] = useState('');
  const [port, setPort] = useState('');
  const [identityFile, setIdentityFile] = useState('');
  const [proxyJump, setProxyJump] = useState('');
  const [connectionName, setConnectionName] = useState('');
  const [remoteRoot, setRemoteRoot] = useState('');
  const [remoteExecution, setRemoteExecution] = useState(false);

  const [manual, setManual] = useState(false);
  const [manualLogin, setManualLogin] = useState('');
  const [socketPath, setSocketPath] = useState('');
  const [workspaceId, setWorkspaceId] = useState('');
  const [ownerUid, setOwnerUid] = useState('');
  const [workspaceKey, setWorkspaceKey] = useState('');

  const [phase, setPhase] = useState<'idle' | 'saving' | 'connecting'>('idle');
  const [pendingConnect, setPendingConnect] = useState<string | null>(null);
  const [validateHidden, setValidateHidden] = useState(false);
  const locked = phase !== 'idle';

  // Ask the daemon to read the paste a moment after typing stops. Nothing is saved.
  useEffect(() => {
    const text = invitation.trim();
    if (manual || !text) {
      // Keep saying why the manual details are showing.
      setPreviewState((current) =>
        manual && current.kind === 'stale' ? current : { kind: 'idle' }
      );
      return;
    }
    const controller = new AbortController();
    setPreviewState({ kind: 'checking' });
    const timer = setTimeout(() => {
      previewInvitation(invitation, {}, controller.signal).then(
        (result) => {
          if (!controller.signal.aborted) setPreviewState({ kind: 'ready', preview: result });
        },
        (failure: unknown) => {
          if (controller.signal.aborted) return;
          if (crewErrorCode(failure) === CREW_INVITATION_INVALID) {
            setPreviewState({ kind: 'invalid' });
          } else if (isStaleDaemon(failure)) {
            // An older background service cannot read invitations: offer the manual details,
            // which every daemon accepts, and say how to get the newer one.
            setPreviewState({ kind: 'stale' });
            setAdvancedOpen(true);
            setManual(true);
          } else {
            setPreviewState({
              kind: 'failed',
              message: failure instanceof Error ? failure.message : joinCopy.invalid,
            });
          }
        }
      );
    }, INVITATION_PREVIEW_DELAY_MS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [invitation, manual]);

  // Prefill from the invitation until the person changes a value.
  useEffect(() => {
    if (!preview) return;
    if (!usernameEdited) setUsername(preview.invitee_username ?? '');
    if (!privacyEdited) {
      setMode(preview.mode ?? 'private');
      setInstitution(preview.institution_id ?? '');
    }
  }, [preview, usernameEdited, privacyEdited]);

  // A submit with an invalid field inside a closed section opens it first, then reports.
  useEffect(() => {
    if (!validateHidden) return;
    setValidateHidden(false);
    formRef.current?.reportValidity();
  }, [validateHidden]);

  // Connect only once the controller has selected the saved connection and lists it: the
  // controller's `connect` is bound to the selection of the render it came from.
  useEffect(() => {
    if (!pendingConnect || crew.connectionId !== pendingConnect) return;
    if (!crew.connections.some((item) => item.id === pendingConnect)) return;
    setPendingConnect(null);
    void crew.connect({ userInitiated: true }).then(() => {
      if (!mounted.current) return;
      setPhase('idle');
      onClose();
    });
  }, [pendingConnect, crew, mounted, onClose]);

  const server =
    sanitizeDisplayText(preview?.ssh_host) ||
    (manual ? sanitizeDisplayText(manualLogin.slice(manualLogin.lastIndexOf('@') + 1)) : '');
  const workspaceLabel =
    sanitizeDisplayText(preview?.workspace_name) ||
    sanitizeDisplayText(connectionName) ||
    joinCopy.unnamedWorkspace;
  const defaultPort = preview?.ssh_port ?? 22;
  const portValue = port.trim() ? Number(port) : null;
  // A Private connection needs an institution. When the invitation has none, show the field
  // rather than letting the save dead-end.
  const needsInstitution = mode === 'private' && !institution.trim();
  const privacyShown = privacyOpen || (needsInstitution && (Boolean(preview) || manual));

  const submitLabel =
    phase === 'connecting'
      ? joinCopy.connecting(server || workspaceLabel)
      : preview?.workspace_name
        ? joinCopy.submit(workspaceLabel)
        : joinCopy.submitFallback;

  const advanced = (): CrewInvitationAdvanced | undefined => {
    const value: CrewInvitationAdvanced = {};
    if (portValue !== null && portValue !== defaultPort) value.port = portValue;
    if (identityFile.trim()) value.identity_file = identityFile.trim();
    if (proxyJump.trim()) value.proxy_jump = proxyJump.trim();
    if (connectionName.trim()) value.name = connectionName.trim();
    if (remoteRoot.trim()) {
      value.remote_root = remoteRoot.trim();
      value.remote_execution = remoteExecution;
    }
    return Object.keys(value).length ? value : undefined;
  };

  /**
   * The invitation route saves `{username}@{server}`. A server login override (an alias from the
   * person's SSH config) replaces it through the ordinary full-body update every daemon accepts,
   * so the join never depends on the invitation route knowing the override. If that update fails,
   * the connection this dialog just saved is removed again: a retry starts clean instead of
   * leaving a connection that would sign in as someone the person did not choose.
   */
  const applyServerLogin = async (connection: CrewConnection, login: string) => {
    try {
      return await crew.updateConnection(connection.id, {
        ...connectionUpdateBody(connection),
        ssh_target: login,
      });
    } catch (failure) {
      // The update's failure is the one to show; a leftover connection stays removable.
      await crew.removeConnection(connection.id).catch(() => undefined);
      throw failure;
    }
  };

  // The manual fields live inside Advanced, which unmounts its fields while closed.
  const manualIncomplete = () =>
    manual &&
    (!manualLogin.trim() ||
      !socketPath.trim().startsWith('/') ||
      !workspaceId.trim() ||
      !/^\d+$/.test(ownerUid.trim()) ||
      !new RegExp(`^${WORKSPACE_KEY_PATTERN}$`).test(workspaceKey.trim()));

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (locked) return;
    if (
      !advancedOpen &&
      (manualIncomplete() ||
        advancedInvalid(port, remoteRoot) ||
        (!manual && serverLoginInvalid(sshAlias)))
    ) {
      setAdvancedOpen(true);
      setValidateHidden(true);
      return;
    }
    if (!manual && !preview) return;
    const institutionId = institution.trim() || null;
    setPhase('saving');
    const saved = await crew.act('dialog:join', 'connection.save', async () => {
      if (manual) {
        const input: SaveConnectionInput = {
          name: connectionName.trim() || workspaceId.trim(),
          ssh_target: manualLogin.trim(),
          port: portValue ?? undefined,
          identity_file: identityFile.trim() || undefined,
          proxy_jump: proxyJump.trim() || undefined,
          socket_path: socketPath.trim(),
          owner_uid: Number(ownerUid.trim()),
          workspace_id: workspaceId.trim(),
          workspace_public_key: workspaceKey.trim().toLowerCase(),
          remote_root: remoteRoot.trim() || undefined,
          remote_execution: remoteRoot.trim() ? remoteExecution : false,
          mode,
          institution_id: institutionId,
        };
        // The controller's save reloads the list and selects the saved connection.
        return crew.saveConnection(input);
      }
      const overrides: CrewInvitationOverrides = { mode, institution_id: institutionId };
      if (username.trim()) overrides.username = username.trim();
      const extra = advanced();
      if (extra) overrides.advanced = extra;
      let connection = await saveFromInvitation(invitation, overrides);
      const login = sshAlias.trim();
      if (login && connection.ssh_target !== login)
        connection = await applyServerLogin(connection, login);
      // Reload the list before selecting, so the controller knows the connection it connects.
      await crew.refresh();
      crew.selectConnection(connection.id);
      return connection;
    });
    if (!mounted.current) return;
    if (!saved) {
      setPhase('idle');
      return;
    }
    updateJoinContext(saved.id, {
      workspaceName: preview?.workspace_name ?? null,
      hostUsername: preview?.host_username ?? null,
      hostDisplayName: preview?.host_display_name ?? null,
      username: username.trim() || null,
      joining: true,
      suggestName: true,
    });
    setPhase('connecting');
    setPendingConnect(saved.id);
  };

  const host = preview?.host_username
    ? personFromProjection({
        username: preview.host_username,
        display_name: preview.host_display_name,
      })
    : null;
  const fingerprint = preview?.workspace_key_fingerprint ?? null;
  const invitationHelper =
    previewState.kind === 'invalid'
      ? joinCopy.invalid
      : previewState.kind === 'failed'
        ? previewState.message
        : previewState.kind === 'checking'
          ? joinCopy.checking
          : undefined;

  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => {
        if (!next && !locked) onClose();
      }}
      size="md"
      purpose={locked ? 'required' : 'form'}
      title={joinCopy.title}
      scrollBody
      footer={
        <>
          <Button type="button" variant="ghost" disabled={locked} onClick={onClose}>
            {joinCopy.cancel}
          </Button>
          <Button
            type="submit"
            form={formId}
            disabled={locked || (!manual && !preview) || previewState.kind === 'checking'}
          >
            {submitLabel}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        ref={formRef}
        className="crew-onboard-form pb-3"
        onSubmit={(event) => void submit(event)}
        aria-busy={locked}
      >
        {!manual ? (
          <Field
            label={joinCopy.invitation}
            helper={invitationHelper}
            invalid={previewState.kind === 'invalid' || previewState.kind === 'failed'}
          >
            {(props) => (
              <textarea
                {...props}
                autoFocus
                required
                rows={4}
                disabled={locked}
                value={invitation}
                onChange={(event) => setInvitation(event.target.value)}
                placeholder={joinCopy.invitationPlaceholder}
                spellCheck={false}
                className="crew-onboard-textarea w-full rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 font-mono text-label placeholder:text-text-muted"
              />
            )}
          </Field>
        ) : null}

        {previewState.kind === 'stale' ? (
          <Note tone="warning" role="status">
            {joinCopy.staleDaemon}
          </Note>
        ) : null}

        {preview ? (
          <div className="crew-onboard-summary" data-testid="crew-join-summary">
            <div className="text-label text-text-default">
              <bdi>{workspaceLabel}</bdi>
            </div>
            {host || server ? (
              <div className="text-supporting text-text-muted" data-testid="crew-join-hosted-by">
                {host ? (
                  <>
                    {joinCopy.hostedBy} <PersonName person={host} context="inline" />
                  </>
                ) : null}
                {server ? (
                  <>
                    {host ? ` ${joinCopy.on} ` : ''}
                    <bdi>{server}</bdi>
                  </>
                ) : null}
              </div>
            ) : null}
            {preview.mode ? (
              <div data-testid="crew-join-workspace-privacy">
                <PrivacyLabel mode={preview.mode} institutionId={preview.institution_id} />
              </div>
            ) : null}
            {fingerprint ? (
              <div className="crew-onboard-field">
                <span className="text-supporting text-text-muted">{joinCopy.fingerprint}</span>
                <CopyField
                  value={fingerprint}
                  display={groupWorkspaceFingerprint(fingerprint) ?? fingerprint}
                  label={joinCopy.fingerprintLabel}
                />
              </div>
            ) : null}
          </div>
        ) : null}

        {preview ? (
          <Field label={server ? joinCopy.username(server) : joinCopy.usernameFallback}>
            {(props) => (
              <Input
                {...props}
                required
                disabled={locked}
                value={username}
                autoComplete="username"
                spellCheck={false}
                onChange={(event) => {
                  setUsernameEdited(true);
                  setUsername(event.target.value);
                }}
              />
            )}
          </Field>
        ) : null}

        {preview || manual ? (
          <div className="crew-onboard-stack">
            <div
              className="crew-onboard-row text-body text-text-default"
              data-testid="crew-join-as"
            >
              <span>{joinCopy.privacyLine}</span>
              <PrivacyLabel
                mode={mode}
                institutionId={mode === 'private' ? institution.trim() : null}
              />
              {!privacyShown ? (
                <Button
                  type="button"
                  variant="link"
                  className="h-auto p-0"
                  disabled={locked}
                  onClick={() => setPrivacyOpen(true)}
                >
                  {joinCopy.change}
                </Button>
              ) : null}
            </div>
            <MismatchLine
              workspace={preview}
              workspaceLabel={workspaceLabel}
              mode={mode}
              institution={institution.trim()}
            />
            {privacyShown ? (
              <PrivacyFields
                mode={mode}
                institution={institution}
                disabled={locked}
                onMode={(next) => {
                  setPrivacyEdited(true);
                  setMode(next);
                }}
                onInstitution={(next) => {
                  setPrivacyEdited(true);
                  setInstitution(next);
                }}
              />
            ) : null}
          </div>
        ) : null}

        <Disclosure
          open={advancedOpen}
          onOpenChange={setAdvancedOpen}
          summary={joinCopy.advancedSummary(portValue ?? defaultPort)}
        >
          <div className="crew-onboard-form">
            <Field
              label={joinCopy.serverLogin}
              helper={joinCopy.serverLoginHelper(
                `${username.trim() || 'you'}@${server || 'server'}`
              )}
            >
              {(props) => (
                <Input
                  {...props}
                  disabled={locked || manual}
                  pattern={SSH_LOGIN_PATTERN}
                  value={sshAlias}
                  spellCheck={false}
                  onChange={(event) => setSshAlias(event.target.value)}
                />
              )}
            </Field>
            <Field label={joinCopy.port}>
              {(props) => (
                <Input
                  {...props}
                  type="number"
                  min={1}
                  max={65535}
                  disabled={locked}
                  placeholder={String(defaultPort)}
                  value={port}
                  onChange={(event) => setPort(event.target.value)}
                />
              )}
            </Field>
            <Field label={joinCopy.identityFile} helper={joinCopy.identityFileHelper}>
              {(props) => (
                <Input
                  {...props}
                  disabled={locked}
                  value={identityFile}
                  spellCheck={false}
                  onChange={(event) => setIdentityFile(event.target.value)}
                />
              )}
            </Field>
            <Field label={joinCopy.jumpHost}>
              {(props) => (
                <Input
                  {...props}
                  disabled={locked}
                  placeholder={preview?.proxy_jump ?? ''}
                  value={proxyJump}
                  spellCheck={false}
                  onChange={(event) => setProxyJump(event.target.value)}
                />
              )}
            </Field>
            <Field label={joinCopy.connectionName}>
              {(props) => (
                <Input
                  {...props}
                  disabled={locked}
                  placeholder={preview?.workspace_name ?? ''}
                  value={connectionName}
                  onChange={(event) => setConnectionName(event.target.value)}
                />
              )}
            </Field>
            <Field label={joinCopy.remoteFolder} helper={joinCopy.remoteFolderHelper}>
              {(props) => (
                <Input
                  {...props}
                  disabled={locked}
                  pattern="/.*"
                  value={remoteRoot}
                  spellCheck={false}
                  onChange={(event) => {
                    setRemoteRoot(event.target.value);
                    if (!event.target.value.trim()) setRemoteExecution(false);
                  }}
                />
              )}
            </Field>
            <SwitchRow
              label={joinCopy.remoteExecution}
              checked={remoteExecution}
              disabled={locked || !remoteRoot.trim()}
              onCheckedChange={setRemoteExecution}
            />
            <SwitchRow
              label={joinCopy.manual}
              checked={manual}
              disabled={locked}
              onCheckedChange={setManual}
            />
            {manual ? (
              <ManualDetails
                disabled={locked}
                values={{ manualLogin, socketPath, workspaceId, ownerUid, workspaceKey }}
                onChange={{
                  manualLogin: setManualLogin,
                  socketPath: setSocketPath,
                  workspaceId: setWorkspaceId,
                  ownerUid: setOwnerUid,
                  workspaceKey: setWorkspaceKey,
                }}
              />
            ) : null}
          </div>
        </Disclosure>

        <JoinErrorSlot />
      </form>
    </ModalShell>
  );
}

/** "{workspace} is Private for ucsf. Your connection will be Public." — only when they differ. */
function MismatchLine({
  workspace,
  workspaceLabel,
  mode,
  institution,
}: {
  workspace: CrewInvitationPreview | null;
  workspaceLabel: string;
  mode: Mode;
  institution: string;
}) {
  const workspaceMode = workspace?.mode ?? null;
  if (!workspace || workspaceMode === null) return null;
  const workspaceInstitution = workspace.institution_id ?? '';
  const differs =
    workspaceMode !== mode || (mode === 'private' && workspaceInstitution !== institution);
  if (!differs) return null;
  const words = (value: Mode, id: string) =>
    value === 'public' ? joinCopy.public : id ? joinCopy.privateFor(id) : joinCopy.private;
  return (
    <p className="text-supporting text-text-muted" data-testid="crew-join-mismatch">
      {joinCopy.mismatch(
        workspaceLabel,
        words(workspaceMode, workspaceInstitution),
        words(mode, institution)
      )}
    </p>
  );
}

type ManualKey = 'manualLogin' | 'socketPath' | 'workspaceId' | 'ownerUid' | 'workspaceKey';

/** "Enter workspace details manually": the four pinned fields and the server login. */
function ManualDetails({
  disabled,
  values,
  onChange,
}: {
  disabled: boolean;
  values: Record<ManualKey, string>;
  onChange: Record<ManualKey, (value: string) => void>;
}) {
  return (
    <>
      <Field label={joinCopy.manualServerLogin}>
        {(props) => (
          <Input
            {...props}
            required
            disabled={disabled}
            placeholder={joinCopy.manualServerLoginPlaceholder}
            value={values.manualLogin}
            spellCheck={false}
            onChange={(event) => onChange.manualLogin(event.target.value)}
          />
        )}
      </Field>
      <Field label={joinCopy.socketPath}>
        {(props) => (
          <Input
            {...props}
            required
            disabled={disabled}
            pattern="/.*"
            value={values.socketPath}
            spellCheck={false}
            onChange={(event) => onChange.socketPath(event.target.value)}
          />
        )}
      </Field>
      <Field label={joinCopy.workspaceId}>
        {(props) => (
          <Input
            {...props}
            required
            disabled={disabled}
            value={values.workspaceId}
            spellCheck={false}
            onChange={(event) => onChange.workspaceId(event.target.value)}
          />
        )}
      </Field>
      <Field label={joinCopy.hostUserId}>
        {(props) => (
          <Input
            {...props}
            required
            type="number"
            min={0}
            disabled={disabled}
            value={values.ownerUid}
            onChange={(event) => onChange.ownerUid(event.target.value)}
          />
        )}
      </Field>
      <Field label={joinCopy.workspaceKey} helper={joinCopy.workspaceKeyHelper}>
        {(props) => (
          <Input
            {...props}
            required
            disabled={disabled}
            pattern={WORKSPACE_KEY_PATTERN}
            value={values.workspaceKey}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => onChange.workspaceKey(event.target.value)}
          />
        )}
      </Field>
    </>
  );
}

/** The dialog's own error slot: an error from saving renders here, once, while the dialog is open. */
function JoinErrorSlot() {
  const { error } = useCrew();
  const here = useCrewErrorSlot('dialog:join');
  if (!here || !error) return null;
  return (
    <Note tone="danger" role="alert">
      {error.message}
    </Note>
  );
}
