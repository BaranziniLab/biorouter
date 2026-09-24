import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import CustomRadio from '../../ui/CustomRadio';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import {
  previewInvitation,
  refusalConnectionId,
  saveFromInvitation,
  savedConnectionIds,
  type CrewInvitationAdvanced,
  type CrewInvitationOverrides,
  type CrewInvitationPreview,
} from '../api/join';
import { CREW_INVITATION_INVALID, crewErrorCode, isStaleDaemon } from '../api/errors';
import type { CrewConnection } from '../crewApi';
import {
  connectionNames,
  PersonName,
  personFromProjection,
  sanitizeDisplayText,
} from '../identity';
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
  /** `connectionId`: the saved connection a 409 conflict concerns, to offer opening it. */
  | { kind: 'failed'; message: string; connectionId: string | null };

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

/** Advanced choices a joiner may make that the invitation route's contract does not carry. */
type LocalSettings = Partial<
  Pick<SaveConnectionInput, 'ssh_target' | 'name' | 'remote_root' | 'remote_execution'>
>;

function settingsDiffer(connection: CrewConnection, settings: LocalSettings): boolean {
  return (Object.keys(settings) as (keyof LocalSettings)[]).some(
    (key) => connection[key] !== settings[key]
  );
}

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
  // Null while the invitation states no privacy and the person hasn't chosen: nothing is assumed.
  const [mode, setMode] = useState<Mode | null>('private');
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
  /** The saved connection a refused save concerns (`connection_id` of a 409), to offer opening. */
  const [saveConflictId, setSaveConflictId] = useState<string | null>(null);
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
    setSaveConflictId(null);
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
              connectionId: refusalConnectionId(failure),
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

  // Prefill from the invitation until the person changes a value. The privacy is what the
  // invitation itself STATES (`workspace_mode`), never the daemon's planned default (`mode`,
  // Private for a paste that states nothing): an unstated privacy stays unchosen.
  useEffect(() => {
    if (!preview) return;
    if (!usernameEdited) setUsername(preview.invitee_username ?? '');
    if (!privacyEdited) {
      setMode(preview.workspace_mode);
      setInstitution(preview.workspace_institution_id ?? '');
    }
  }, [preview, usernameEdited, privacyEdited]);

  // An invitation that names no server needs the login from Advanced: open it, once per paste.
  const serverMissing = Boolean(preview?.missing.includes('server'));
  useEffect(() => {
    if (serverMissing) setAdvancedOpen(true);
  }, [serverMissing]);

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
  // rather than letting the save dead-end. An unchosen privacy shows the choice at once.
  const needsInstitution = mode === 'private' && !institution.trim();
  const privacyShown =
    privacyOpen || mode === null || (needsInstitution && (Boolean(preview) || manual));

  // A connection this computer already has for the workspace: offer it instead of saving again.
  const existingId = !manual ? (preview?.existing_connection_id ?? null) : null;
  // A paste this computer pins differently (409 `crew_invitation_conflict` at preview).
  const previewConflictId =
    !manual && previewState.kind === 'failed' ? previewState.connectionId : null;
  const savedNames = connectionNames(crew.connections);
  const openLabel = (id: string) => joinCopy.openExisting(savedNames.get(id) || workspaceLabel);

  const submitLabel =
    phase === 'connecting'
      ? joinCopy.connecting(server || workspaceLabel)
      : preview?.workspace_name
        ? joinCopy.submit(workspaceLabel)
        : joinCopy.submitFallback;

  /**
   * The overrides the invitation route's contract names (naming-design "Joiner": port, identity
   * file, jump host). Anything else the person chose is applied afterwards (`localSettings`).
   */
  const advanced = (): CrewInvitationAdvanced | undefined => {
    const value: CrewInvitationAdvanced = {};
    if (portValue !== null && portValue !== defaultPort) value.port = portValue;
    if (identityFile.trim()) value.identity_file = identityFile.trim();
    if (proxyJump.trim()) value.proxy_jump = proxyJump.trim();
    return Object.keys(value).length ? value : undefined;
  };

  /** The Advanced choices the invitation route does not take: server login, name, work folder. */
  const localSettings = (): LocalSettings => {
    const value: LocalSettings = {};
    if (sshAlias.trim()) value.ssh_target = sshAlias.trim();
    if (connectionName.trim()) value.name = connectionName.trim();
    if (remoteRoot.trim()) {
      value.remote_root = remoteRoot.trim();
      value.remote_execution = remoteExecution;
    }
    return value;
  };

  /**
   * The invitation route saves `{username}@{server}`, named for the workspace, with no work
   * folder. What the person chose instead (an SSH alias from their own config, a connection name,
   * a work folder) is applied through the ordinary full-body update every daemon accepts, so the
   * join never depends on the invitation route knowing a field its contract does not name. It is
   * applied only to a connection this submit certainly created (`createdHere`), and if the update
   * fails that connection is removed again: a retry starts clean instead of leaving a connection
   * that would sign in as someone the person did not choose. A connection that was already on
   * this computer, or may have been, is never updated and never removed.
   */
  const applyLocalSettings = async (
    connection: CrewConnection,
    settings: LocalSettings,
    createdHere: boolean
  ) => {
    try {
      return await crew.updateConnection(connection.id, {
        ...connectionUpdateBody(connection),
        ...settings,
      });
    } catch (failure) {
      // Only a connection this submit created is removed: removing one that was already here
      // would take a joined workspace and its device key with it. The update's failure is the one
      // to show; a leftover connection stays removable.
      if (createdHere) await crew.removeConnection(connection.id).catch(() => undefined);
      throw failure;
    }
  };

  /** Open a connection this computer already has, instead of saving the invitation again. */
  const openConnection = async (id: string) => {
    if (locked) return;
    if (!crew.connections.some((item) => item.id === id))
      await crew.refresh().catch(() => undefined);
    if (!mounted.current) return;
    crew.selectConnection(id);
    onClose();
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
    // Never save a privacy the person did not choose.
    if (mode === null) return;
    const institutionId = institution.trim() || null;
    setPhase('saving');
    setSaveConflictId(null);
    /** False once the save returned a connection that was already on this computer. */
    let isNew = true;
    /** True only when this submit certainly created the connection: the one it may remove. */
    let createdHere = false;
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
      // What was saved before this submit. Saving an invitation for a workspace this computer
      // already has returns THAT connection, so only an id missing from this list was created
      // here. When the list can't be read (`before` is null), nothing counts as created here, so
      // nothing is updated or removed: the saved connection is opened as it is, and its settings
      // are changed in Connection settings. A leftover connection is removable; a joined one that
      // was rewritten (an update disconnects it) or removed is not recoverable.
      const before = await savedConnectionIds().then(
        (ids) => new Set([...ids, ...crew.connections.map((item) => item.id)]),
        () => null
      );
      let connection: CrewConnection;
      try {
        connection = await saveFromInvitation(invitation, overrides);
      } catch (failure) {
        if (mounted.current)
          setSaveConflictId(refusalConnectionId(failure, preview?.existing_connection_id));
        throw failure;
      }
      // Without the daemon's list, the controller's own list still says which ones it knew.
      const known = before ?? new Set(crew.connections.map((item) => item.id));
      const preexisting =
        connection.id === preview?.existing_connection_id || known.has(connection.id);
      isNew = !preexisting;
      createdHere = before !== null && isNew;
      // Only a connection this submit certainly created is updated. One that was already here,
      // or may have been, is opened as it is: its settings are changed in Connection settings,
      // never by re-pasting an invitation (an update disconnects it).
      const settings = localSettings();
      if (createdHere && settingsDiffer(connection, settings))
        connection = await applyLocalSettings(connection, settings, createdHere);
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
    // A connection that was already here keeps what it remembers about its own join.
    if (isNew)
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
  // Copy the full hex; show the short form the host reads out (computed here, else the daemon's).
  const fingerprint = preview?.workspace_key_fingerprint ?? null;
  const fingerprintShort =
    groupWorkspaceFingerprint(fingerprint) || sanitizeDisplayText(preview?.fingerprint) || null;
  const statedMode = preview?.workspace_mode ?? null;
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
          {existingId ? (
            <Button type="button" disabled={locked} onClick={() => void openConnection(existingId)}>
              {openLabel(existingId)}
            </Button>
          ) : (
            <Button
              type="submit"
              form={formId}
              disabled={
                locked || (!manual && !preview) || previewState.kind === 'checking' || mode === null
              }
            >
              {submitLabel}
            </Button>
          )}
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

        {previewConflictId ? (
          <div className="crew-onboard-actions">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={locked}
              onClick={() => void openConnection(previewConflictId)}
            >
              {openLabel(previewConflictId)}
            </Button>
          </div>
        ) : null}

        {previewState.kind === 'stale' ? (
          <Note tone="warning" role="status">
            {joinCopy.staleDaemon}
          </Note>
        ) : null}

        {existingId ? (
          <Note tone="info" role="status" testId="crew-join-existing">
            {joinCopy.existing(workspaceLabel)}
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
            {/* What the invitation states, never the privacy saving would default to. */}
            <div data-testid="crew-join-workspace-privacy">
              {statedMode ? (
                <PrivacyLabel mode={statedMode} institutionId={preview.workspace_institution_id} />
              ) : (
                <span className="text-supporting text-text-muted">
                  {joinCopy.privacy}: {joinCopy.privacyUnstated}
                </span>
              )}
            </div>
            {fingerprint || fingerprintShort ? (
              <div className="crew-onboard-field">
                <span className="text-supporting text-text-muted">{joinCopy.fingerprint}</span>
                <CopyField
                  value={fingerprint ?? fingerprintShort ?? ''}
                  display={fingerprintShort ?? fingerprint ?? ''}
                  label={joinCopy.fingerprintLabel}
                />
              </div>
            ) : null}
          </div>
        ) : null}

        {preview && !existingId ? (
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

        {(preview && !existingId) || manual ? (
          <div className="crew-onboard-stack">
            <div
              className="crew-onboard-row text-body text-text-default"
              data-testid="crew-join-as"
            >
              {mode === null ? (
                <span>{joinCopy.privacyChoose}</span>
              ) : (
                <>
                  <span>{joinCopy.privacyLine}</span>
                  <PrivacyLabel
                    mode={mode}
                    institutionId={mode === 'private' ? institution.trim() : null}
                  />
                </>
              )}
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
            {privacyShown && mode === null ? (
              <UnchosenPrivacy
                disabled={locked}
                onMode={(next) => {
                  // Keep the choice in view once made, so it can still be changed.
                  setPrivacyOpen(true);
                  setPrivacyEdited(true);
                  setMode(next);
                }}
              />
            ) : privacyShown && mode !== null ? (
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
              helper={
                serverMissing && !manual
                  ? joinCopy.serverMissing
                  : joinCopy.serverLoginHelper(`${username.trim() || 'you'}@${server || 'server'}`)
              }
            >
              {(props) => (
                <Input
                  {...props}
                  required={serverMissing && !manual}
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

        <JoinErrorSlot
          action={
            saveConflictId ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={locked}
                onClick={() => void openConnection(saveConflictId)}
              >
                {openLabel(saveConflictId)}
              </Button>
            ) : undefined
          }
        />
      </form>
    </ModalShell>
  );
}

/**
 * Private / Public with neither chosen, for an invitation that doesn't state the workspace's
 * privacy. Choosing one hands over to `PrivacyFields`; nothing is assumed before that.
 */
function UnchosenPrivacy({
  disabled,
  onMode,
}: {
  disabled: boolean;
  onMode: (mode: Mode) => void;
}) {
  const name = useId();
  return (
    <fieldset className="crew-onboard-radios" data-testid="crew-join-privacy-unchosen">
      <legend className="text-label text-text-default">{joinCopy.privacy}</legend>
      <CustomRadio
        id={`${name}-private`}
        name={name}
        value="private"
        checked={false}
        disabled={disabled}
        onChange={() => onMode('private')}
        label={joinCopy.private}
        secondaryLabel={joinCopy.privateHint}
      />
      <CustomRadio
        id={`${name}-public`}
        name={name}
        value="public"
        checked={false}
        disabled={disabled}
        onChange={() => onMode('public')}
        label={joinCopy.public}
        secondaryLabel={joinCopy.publicHint}
      />
    </fieldset>
  );
}

/**
 * "{workspace} is Private for ucsf. Your connection will be Public." — only when they differ, and
 * only against the privacy the invitation STATES.
 */
function MismatchLine({
  workspace,
  workspaceLabel,
  mode,
  institution,
}: {
  workspace: CrewInvitationPreview | null;
  workspaceLabel: string;
  mode: Mode | null;
  institution: string;
}) {
  const workspaceMode = workspace?.workspace_mode ?? null;
  if (!workspace || workspaceMode === null || mode === null) return null;
  const workspaceInstitution = workspace.workspace_institution_id ?? '';
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

/**
 * The dialog's own error slot: an error from saving renders here, once, while the dialog is open.
 * `action` offers the saved connection a refusal concerns.
 */
function JoinErrorSlot({ action }: { action?: ReactNode }) {
  const { error } = useCrew();
  const here = useCrewErrorSlot('dialog:join');
  if (!here || !error) return null;
  return (
    <Note tone="danger" role="alert" action={action}>
      {error.message}
    </Note>
  );
}
