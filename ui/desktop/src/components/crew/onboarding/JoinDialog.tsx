import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from 'react';
import { Check } from '../../icons/app-icons';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
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
  institutionLabel,
  isInstitutionId,
  PersonName,
  personFromProjection,
  sanitizeDisplayText,
  type KnownInstitution,
} from '../identity';
import { knownInstitutions } from '../pane/presentation';
import { useConfiguredModels } from '../pane/useConfiguredModels';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import type { SaveConnectionInput } from '../state/types';
import { connectionUpdateBody } from '../state/useCrewConnections';
import { joinCopy, joinStateCopy } from './copy';
import {
  AgentAccessFields,
  Field,
  FieldErrorsProvider,
  PrivacyFields,
  remoteFolderInvalid,
  SwitchRow,
  useFormValidation,
  useInitialFocus,
  useMounted,
  useOpenGeneration,
} from './fields';
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

/**
 * Whether a port or work folder holds a value its field would refuse. The port lives in Advanced and
 * the folder in the agent row (Q2-37); pass `''` for whichever is not being asked about.
 */
export function advancedInvalid(port: string, remoteRoot: string): boolean {
  const portText = port.trim();
  if (portText) {
    const value = Number(portText);
    if (!Number.isInteger(value) || value < 1 || value > 65535) return true;
  }
  return remoteFolderInvalid(remoteRoot);
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

/** A change that put a whole invitation in the box (a paste, a drop, a programmatic set). */
function replacedWhole(event: { nativeEvent: Event }): boolean {
  const type = (event.nativeEvent as InputEvent).inputType;
  return !type || type === 'insertFromPaste' || type === 'insertFromDrop';
}

function JoinDialogView({ open, onClose }: { open: boolean; onClose: () => void }) {
  const crew = useCrew();
  const formId = useId();
  const mounted = useMounted();
  const { errors, validate, formProps } = useFormValidation();

  const [invitation, setInvitation] = useState('');
  const [previewState, setPreviewState] = useState<PreviewState>({ kind: 'idle' });
  const preview = previewState.kind === 'ready' ? previewState.preview : null;
  /**
   * The box shrinks to "Invitation read ✓ · Edit" once a pasted invitation is read, so the base64
   * wall doesn't sit above the summary (T-43). Only a paste collapses it: a box the person is
   * typing in never disappears under them.
   */
  const [invitationCollapsed, setInvitationCollapsed] = useState(false);
  const collapseOnRead = useRef(true);
  const invitationRef = useRef<HTMLTextAreaElement>(null);
  const editInvitationRef = useRef<HTMLButtonElement>(null);
  /** Where focus goes after the box changes shape: the box had it, so its replacement takes it. */
  const [invitationFocus, setInvitationFocus] = useState<'edit' | 'box' | null>(null);

  // The person's own edits win over what the invitation prefills.
  const [username, setUsername] = useState('');
  const [usernameEdited, setUsernameEdited] = useState(false);
  // Null while the invitation states no privacy and the person hasn't chosen: nothing is assumed.
  const [mode, setMode] = useState<Mode | null>('private');
  const [institution, setInstitution] = useState('');
  const [privacyEdited, setPrivacyEdited] = useState(false);
  /**
   * The privacy fields, once on screen, stay on screen (P0-4): typing an institution or choosing
   * Public must never unmount the field in use. Latched by every path that shows them.
   */
  const [privacyOpen, setPrivacyOpen] = useState(false);
  /** Move focus into the privacy fields once Change has revealed them (Change itself goes). */
  const [focusPrivacy, setFocusPrivacy] = useState(false);
  /** Move focus back to Change once Done has folded the fields away (Done itself goes). */
  const [focusChange, setFocusChange] = useState(false);
  const privacyRef = useRef<HTMLDivElement>(null);
  const changeRef = useRef<HTMLButtonElement>(null);
  /** The preview whose prefill has landed: until then `mode` and `institution` are the old ones. */
  const [prefilledFor, setPrefilledFor] = useState<CrewInvitationPreview | null>(null);

  const [advancedOpen, setAdvancedOpen] = useState(false);
  /** "Agent on {server}": its own row, folded, off unless the person turns it on (Q2-37). */
  const [agentOpen, setAgentOpen] = useState(false);
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

  // Open on the invitation box, and keep focus there while the menu that opened the dialog closes
  // (Q2-27).
  useInitialFocus(invitationRef, open);

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
          if (controller.signal.aborted) return;
          setPreviewState({ kind: 'ready', preview: result });
          if (collapseOnRead.current) {
            if (document.activeElement === invitationRef.current) setInvitationFocus('edit');
            setInvitationCollapsed(true);
          }
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
    setPrefilledFor(preview);
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
    validate();
  }, [validateHidden, validate]);

  // The invitation box changed shape while it had focus: hand focus to what replaced it.
  useLayoutEffect(() => {
    if (!invitationFocus) return;
    setInvitationFocus(null);
    (invitationFocus === 'edit' ? editInvitationRef.current : invitationRef.current)?.focus();
  }, [invitationFocus, invitationCollapsed]);

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

  /** The server's address, as a login is built from it (`{username}@{address}`). */
  const serverAddress =
    sanitizeDisplayText(preview?.ssh_host) ||
    (manual ? sanitizeDisplayText(manualLogin.slice(manualLogin.lastIndexOf('@') + 1)) : '');
  /**
   * What to call the server on screen (D-ALIAS): the person's own SSH alias for the address when
   * the daemon found one ("lab-server"), else the address. Never used to build a login.
   */
  const server = (!manual && sanitizeDisplayText(preview?.server_label)) || serverAddress;
  const workspaceLabel =
    sanitizeDisplayText(preview?.workspace_name) ||
    sanitizeDisplayText(connectionName) ||
    joinCopy.unnamedWorkspace;
  const defaultPort = preview?.ssh_port ?? 22;
  const portValue = port.trim() ? Number(port) : null;
  // A Private connection needs an institution. When the invitation has none, show the field
  // rather than letting the save dead-end. An unchosen privacy shows the choice at once. Only a
  // settled prefill counts: the render a preview arrives in still holds the previous values.
  const prefilled = manual || (preview !== null && prefilledFor === preview);
  const needsInstitution = mode === 'private' && !institution.trim();
  const privacyShown = privacyOpen || mode === null || (needsInstitution && prefilled);

  // Latch: once shown, the fields stay, however the value that showed them changes.
  useEffect(() => {
    if (privacyShown && !privacyOpen) setPrivacyOpen(true);
  }, [privacyShown, privacyOpen]);

  // Change unmounts itself: put focus on the chosen privacy instead of dropping it to the page.
  useLayoutEffect(() => {
    if (!focusPrivacy || !privacyShown) return;
    setFocusPrivacy(false);
    privacyRef.current?.querySelector<HTMLInputElement>('input[type="radio"]:checked')?.focus();
  }, [focusPrivacy, privacyShown]);

  // Done unmounts itself too: hand focus to the Change that replaced the fields.
  useLayoutEffect(() => {
    if (!focusChange || privacyShown) return;
    setFocusChange(false);
    changeRef.current?.focus();
  }, [focusChange, privacyShown]);

  // The privacy choice folds back to its line only when the line can state it: a choice made, and
  // for Private an institution the daemon would take.
  const privacyFoldable =
    mode === 'public' || (mode === 'private' && isInstitutionId(institution.trim()));

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
    // A field that would be refused inside a folded section: open it, then report under the field.
    const agentHidden = !agentOpen && remoteFolderInvalid(remoteRoot);
    const advancedHidden =
      !advancedOpen &&
      (manualIncomplete() ||
        advancedInvalid(port, '') ||
        (!manual && serverLoginInvalid(sshAlias)));
    if (agentHidden || advancedHidden) {
      if (agentHidden) setAgentOpen(true);
      if (advancedHidden) setAdvancedOpen(true);
      setValidateHidden(true);
      return;
    }
    // The form is `noValidate`: its fields are checked here and answered under each field.
    if (!validate()) return;
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
  // "Ask @alice which institution lab uses": the invitation carried none, so say whom to ask.
  const institutionHelper =
    preview && !preview.workspace_institution_id
      ? joinCopy.institutionUnknown(
          host ? `@${host.username}` : joinStateCopy.yourHost,
          workspaceLabel
        )
      : undefined;
  // The short form the host reads out (computed here, else the daemon's). Never copyable here: it
  // looks like the join code and is not something the joiner sends (Q2-04).
  const fingerprintShort =
    groupWorkspaceFingerprint(preview?.workspace_key_fingerprint) ||
    sanitizeDisplayText(preview?.fingerprint) ||
    null;
  const statedMode = preview?.workspace_mode ?? null;
  const invitationHelper =
    previewState.kind === 'invalid'
      ? joinCopy.invalid
      : previewState.kind === 'failed'
        ? previewState.message
        : previewState.kind === 'checking'
          ? joinCopy.checking
          : undefined;

  // "@alice": the person the joiner asks, by the name Crew shows them everywhere (Q2-04).
  const hostHandle = host ? `@${host.username}` : joinStateCopy.yourHost;
  const hostSubject = host ? `@${host.username}` : joinStateCopy.yourHostSubject;

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
      // A dialog portals outside `.crew-app`: the class carries Crew's focused-field edge to it
      // (Q2-25), and the top anchor keeps it from re-centring as sections open (Q2-26).
      className="crew-dialog"
      anchor="top"
      footer={
        <>
          <Button type="button" variant="secondary" disabled={locked} onClick={onClose}>
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
      <WithKnownInstitutions>
        {(known) => (
          <FieldErrorsProvider value={errors}>
            <form
              id={formId}
              {...formProps}
              className="crew-onboard-form crew-onboard-dialog-form"
              onSubmit={(event) => void submit(event)}
              aria-busy={locked}
            >
              {!manual && invitationCollapsed && preview ? (
                <div className="crew-onboard-field">
                  <span className="text-label text-text-default">{joinCopy.invitation}</span>
                  <div
                    className="crew-onboard-invitation-read"
                    data-testid="crew-join-invitation-read"
                  >
                    <span className="crew-onboard-row" role="status">
                      <Check aria-hidden className="h-4 w-4 text-text-success" />
                      <span className="text-body text-text-default">{joinCopy.invitationRead}</span>
                    </span>
                    <span aria-hidden="true" className="text-text-muted">
                      ·
                    </span>
                    <Button
                      ref={editInvitationRef}
                      type="button"
                      variant="link"
                      className="h-auto p-0"
                      disabled={locked}
                      aria-label={joinCopy.editInvitationLabel}
                      onClick={() => {
                        // Editing keeps the box open: only a new paste collapses it again.
                        collapseOnRead.current = false;
                        setInvitationCollapsed(false);
                        setInvitationFocus('box');
                      }}
                    >
                      {joinCopy.editInvitation}
                    </Button>
                  </div>
                </div>
              ) : !manual ? (
                <Field
                  label={joinCopy.invitation}
                  helper={invitationHelper}
                  invalid={previewState.kind === 'invalid' || previewState.kind === 'failed'}
                >
                  {(props) => (
                    <textarea
                      {...props}
                      ref={invitationRef}
                      autoFocus={!invitation}
                      required
                      rows={4}
                      disabled={locked}
                      value={invitation}
                      onChange={(event) => {
                        collapseOnRead.current = replacedWhole(event);
                        setInvitation(event.target.value);
                      }}
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
                    <div
                      className="text-supporting text-text-muted"
                      data-testid="crew-join-hosted-by"
                    >
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
                      <PrivacyLabel
                        mode={statedMode}
                        institutionId={preview.workspace_institution_id}
                        known={known}
                      />
                    ) : (
                      <span className="text-supporting text-text-muted">
                        {joinCopy.privacy}: {joinCopy.privacyUnstated}
                      </span>
                    )}
                  </div>
                  {fingerprintShort ? (
                    // Folded, and never copyable: four groups of four is the join code's shape, and
                    // the joiner has nothing to compare it with until the host reads theirs (Q2-04).
                    <Disclosure label={joinCopy.fingerprintCheck}>
                      <p
                        className="text-supporting text-text-muted"
                        data-testid="crew-join-fingerprint-helper"
                      >
                        {joinCopy.fingerprint}{' '}
                        <span className="font-mono text-text-default" translate="no">
                          {fingerprintShort}
                        </span>
                        . {joinCopy.fingerprintHelper(hostHandle)}
                      </p>
                    </Disclosure>
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
                <div className="crew-onboard-stack" ref={privacyRef}>
                  {/* Privacy is said once: this line while the fields are folded away, the radio
                  rows once they are open. An unchosen privacy keeps the line as its prompt. */}
                  {!privacyShown || mode === null ? (
                    <div
                      className="crew-onboard-row text-body text-text-default"
                      data-testid="crew-join-as"
                    >
                      {mode === null ? (
                        <span>{joinCopy.privacyChoose}</span>
                      ) : (
                        // What the choice governs, the models, rather than "join as" (Q2-36).
                        <span>
                          {joinCopy.privacyLine(
                            mode,
                            mode === 'private' ? institutionLabel(institution.trim(), known) : null
                          )}
                        </span>
                      )}
                      {!privacyShown ? (
                        <>
                          <span aria-hidden="true" className="text-text-muted">
                            ·
                          </span>
                          <Button
                            ref={changeRef}
                            type="button"
                            variant="link"
                            className="h-auto p-0"
                            disabled={locked}
                            onClick={() => {
                              setPrivacyOpen(true);
                              setFocusPrivacy(true);
                            }}
                          >
                            {joinCopy.change}
                          </Button>
                        </>
                      ) : null}
                    </div>
                  ) : null}
                  {privacyShown ? (
                    // One fieldset for the unmade choice and the made one, so the first pick keeps
                    // focus on the radio it landed on.
                    <PrivacyFields
                      mode={mode}
                      institution={institution}
                      disabled={locked}
                      institutionHelper={institutionHelper}
                      testId={mode === null ? 'crew-join-privacy-unchosen' : undefined}
                      onMode={(next) => {
                        // Keep the choice in view once made, so it can still be changed.
                        setPrivacyOpen(true);
                        setPrivacyEdited(true);
                        setMode(next);
                      }}
                      onInstitution={(next) => {
                        setPrivacyOpen(true);
                        setPrivacyEdited(true);
                        setInstitution(next);
                      }}
                    />
                  ) : null}
                  {privacyShown && privacyFoldable ? (
                    // Folds the choice back to its one line, once the line can say it (Q2-36).
                    <div className="crew-onboard-row">
                      <Button
                        type="button"
                        variant="link"
                        className="h-auto p-0"
                        disabled={locked}
                        onClick={() => {
                          setPrivacyOpen(false);
                          setFocusChange(true);
                        }}
                      >
                        {joinCopy.privacyDone}
                      </Button>
                    </div>
                  ) : null}
                  {statedMode === 'private' && mode === 'public' ? (
                    // Public on a Private workspace: what it does and doesn't change (Q2-36).
                    <Note tone="warning" role="status" testId="crew-join-public-consequence">
                      {joinCopy.publicConsequence(
                        workspaceLabel,
                        institutionLabel(preview?.workspace_institution_id, known),
                        hostHandle,
                        hostSubject
                      )}
                    </Note>
                  ) : (
                    <MismatchLine
                      workspace={preview}
                      workspaceLabel={workspaceLabel}
                      mode={mode}
                      institution={institution.trim()}
                      known={known}
                    />
                  )}
                </div>
              ) : null}

              {(preview && !existingId) || manual ? (
                <AgentAccessFields
                  server={server}
                  open={agentOpen}
                  onOpenChange={setAgentOpen}
                  remoteRoot={remoteRoot}
                  remoteExecution={remoteExecution}
                  disabled={locked}
                  onRemoteRoot={setRemoteRoot}
                  onRemoteExecution={setRemoteExecution}
                />
              ) : null}

              <Disclosure
                open={advancedOpen}
                onOpenChange={setAdvancedOpen}
                summary={joinCopy.advancedSummary(portValue ?? defaultPort)}
              >
                <div className="crew-onboard-form">
                  <Field
                    label={joinCopy.serverLogin}
                    required={serverMissing && !manual}
                    invalidMessage={joinCopy.serverLoginInvalid}
                    helper={
                      serverMissing && !manual
                        ? joinCopy.serverMissing
                        : // The login this would replace, as it is written: the address, never the label.
                          joinCopy.serverLoginHelper(
                            `${username.trim() || 'you'}@${serverAddress || 'server'}`
                          )
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
                  <Field label={joinCopy.port} invalidMessage={joinCopy.portInvalid}>
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
                        spellCheck={false}
                        onChange={(event) => setConnectionName(event.target.value)}
                      />
                    )}
                  </Field>
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
          </FieldErrorsProvider>
        )}
      </WithKnownInstitutions>
    </ModalShell>
  );
}

/**
 * The institutions configured providers publish names for, so `ucsf` reads as "UCSF" (Q2-38). Read
 * only while the dialog's content is mounted, that is while it is open.
 */
function WithKnownInstitutions({
  children,
}: {
  children: (known: readonly KnownInstitution[]) => ReactNode;
}) {
  const { providers } = useConfiguredModels();
  const known = useMemo(() => knownInstitutions(providers), [providers]);
  return <>{children(known)}</>;
}

/**
 * "{workspace} is Private for ucsf. Your connection will be Public." — only when they differ, and
 * only against the privacy the invitation STATES. An invitation that names no institution states
 * nothing to differ from, so a Private choice with an institution is not a mismatch.
 */
function MismatchLine({
  workspace,
  workspaceLabel,
  mode,
  institution,
  known,
}: {
  workspace: CrewInvitationPreview | null;
  workspaceLabel: string;
  mode: Mode | null;
  institution: string;
  known: readonly KnownInstitution[];
}) {
  const workspaceMode = workspace?.workspace_mode ?? null;
  if (!workspace || workspaceMode === null || mode === null) return null;
  const workspaceInstitution = workspace.workspace_institution_id ?? '';
  const differs =
    workspaceMode !== mode ||
    (mode === 'private' && Boolean(workspaceInstitution) && workspaceInstitution !== institution);
  if (!differs) return null;
  const words = (value: Mode, id: string) =>
    value === 'public'
      ? joinCopy.public
      : id
        ? joinCopy.privateFor(institutionLabel(id, known) ?? id)
        : joinCopy.private;
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
      <Field label={joinCopy.socketPath} invalidMessage={joinCopy.socketPathInvalid}>
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
      <Field label={joinCopy.hostUserId} invalidMessage={joinCopy.hostUserIdInvalid}>
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
      <Field
        label={joinCopy.workspaceKey}
        helper={joinCopy.workspaceKeyHelper}
        invalidMessage={joinCopy.workspaceKeyHelper}
      >
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
