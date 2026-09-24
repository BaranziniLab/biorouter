import { useEffect, useId, useLayoutEffect, useRef, useState, type FormEvent } from 'react';
import { useConfig } from '../../ConfigContext';
import { ModalShell } from '../../ModalShell';
import { readProviderAffiliation } from '../../privacy/providerAffiliation';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { previewInvitation, type CrewInvitationPreview } from '../api/join';
import { CREW_INVITATION_INVALID, crewErrorCode, isStaleDaemon } from '../api/errors';
import { connectionServer, isInstitutionId, sanitizeDisplayText } from '../identity';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import type { ErrorSource, PreparedDevice } from '../state/types';
import { hostCopy, INSTALL_COMMANDS, joinCopy } from './copy';
import { Field, PrivacyFields, SwitchRow, useMounted, useOpenGeneration } from './fields';
import { updateJoinContext, useJoinContext } from './joinContext';
import { advancedInvalid, WORKSPACE_KEY_PATTERN } from './JoinDialog';
import {
  groupWorkspaceFingerprint,
  hostStartCommands,
  isWorkspaceName,
  sshLoginCommand,
  sshUsername,
  workspaceSlug,
} from './joinText';
import { EmbeddedTerminal, TerminalToggle } from './parts';

type Step = 'name' | 'start' | 'create' | 'label';
type Mode = 'private' | 'public';

/** The four fields that pin a workspace, parsed by the daemon from what `biorouter-crew` printed. */
interface PinnedWorkspace {
  socket_path: string;
  owner_uid: number;
  workspace_id: string;
  workspace_public_key: string;
  fingerprint: string | null;
}

/**
 * How the Start step's paste stands. `stale`: the daemon cannot read one (404), so the person
 * types the four details. `incomplete`: it read one, but its preview lacks a detail the workspace
 * pins (the socket path, the host user ID or the key), so the person types what is missing —
 * the paste itself was fine, so it is never called bad.
 */
type ParseState = 'idle' | 'reading' | 'bad' | 'stale' | 'incomplete';

const WORKSPACE_KEY = new RegExp(`^${WORKSPACE_KEY_PATTERN}$`);

/** The pinned details the person typed, or null while one would be refused. */
function typedPinned(
  values: { socketPath: string; workspaceId: string; ownerUid: string; workspaceKey: string },
  preview: CrewInvitationPreview | null
): PinnedWorkspace | null {
  const socketPath = values.socketPath.trim();
  const workspaceId = values.workspaceId.trim();
  const ownerUid = values.ownerUid.trim();
  const key = values.workspaceKey.trim().toLowerCase();
  if (!socketPath.startsWith('/') || !workspaceId || !/^\d+$/.test(ownerUid)) return null;
  if (!WORKSPACE_KEY.test(key)) return null;
  // The daemon's fingerprint describes the key it read; a key typed differently has none.
  const fingerprint =
    preview?.workspace_public_key?.toLowerCase() === key ? preview.workspace_key_fingerprint : null;
  return {
    socket_path: socketPath,
    owner_uid: Number(ownerUid),
    workspace_id: workspaceId,
    workspace_public_key: key,
    fingerprint,
  };
}

/**
 * Where creating the workspace stands:
 * saving → connecting → (signing-in) → bootstrapping → verifying, or back to idle with a reason.
 */
type CreatePhase =
  | 'idle'
  | 'saving'
  | 'connecting'
  | 'signing-in'
  | 'bootstrapping'
  | 'verifying'
  | 'labelling';

const STEP_NUMBER: Record<Exclude<Step, 'label'>, number> = { name: 1, start: 2, create: 3 };

/** The one institution the configured providers name, when there is exactly one. */
function useSingleInstitution(open: boolean): string | null {
  const { getProviders } = useConfig();
  const [institution, setInstitution] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    getProviders(false)
      .then((providers) => {
        if (cancelled || !Array.isArray(providers)) return;
        const ids = new Set<string>();
        for (const provider of providers) {
          if (!provider?.is_configured) continue;
          const affiliation = readProviderAffiliation(provider);
          if (affiliation?.kind !== 'institutions') continue;
          for (const { id } of affiliation.institutions) if (isInstitutionId(id)) ids.add(id);
        }
        if (ids.size === 1) setInstitution([...ids][0]);
      })
      .catch(() => {
        // Only a default: without the provider list the field starts empty.
      });
    return () => {
      cancelled = true;
    };
  }, [open, getProviders]);
  return institution;
}

export interface HostDialogProps {
  /** Defaults to the controller's dialog intent (`{kind: 'host'}`). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/**
 * Host a new workspace, in three steps: Name · Start · Create (naming slices S2 and S3a).
 *
 * - **Name**: the workspace name, the host's server login, privacy and institution. Continue
 *   prepares (or recovers) this computer's hosting identity by itself.
 * - **Start**: the command to run on the server, copyable, with an optional terminal embedded
 *   below it — nothing is typed or run for the person. They paste what it printed; the daemon
 *   parses it.
 * - **Create**: the confirmation "Initialize as workspace host" never had (L3). It saves the
 *   connection with the prepared identity, connects (Sign in opens by itself if the server asks),
 *   and asks the workspace to make this computer its first admin device (`auth.bootstrap`).
 *
 * A Private workspace without an institution label is then asked once to take the host's
 * institution — an irreversible label, so it has its own "Set {id} permanently". A host setup
 * interrupted after saving is remembered on this computer and resumes at Create.
 */
export function HostDialog({ open, onOpenChange }: HostDialogProps) {
  const crew = useCrew();
  const isOpen = open ?? crew.ui.dialog?.kind === 'host';
  const generation = useOpenGeneration(isOpen);
  const close = () => {
    if (onOpenChange) onOpenChange(false);
    else crew.closeDialog();
  };
  return <HostDialogView key={generation} open={isOpen} onClose={close} />;
}

function HostDialogView({ open, onClose }: { open: boolean; onClose: () => void }) {
  const crew = useCrew();
  const mounted = useMounted();
  const formId = useId();
  const formRef = useRef<HTMLFormElement>(null);
  const suggestedInstitution = useSingleInstitution(open);

  // A host setup this computer saved but never created resumes at Create. Decided when the
  // dialog opens (this view outlives a close only for the exit animation).
  const resumeContext = useJoinContext(crew.connectionId);
  const [resumeId, setResumeId] = useState<string | null>(null);
  const [step, setStep] = useState<Step>('name');
  const opened = useRef(false);

  // Name
  const [nameText, setNameText] = useState('');
  const [serverLogin, setServerLogin] = useState('');
  const [mode, setMode] = useState<Mode>('private');
  const [institution, setInstitution] = useState('');
  const [institutionEdited, setInstitutionEdited] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [port, setPort] = useState('');
  const [identityFile, setIdentityFile] = useState('');
  const [proxyJump, setProxyJump] = useState('');
  const [connectionName, setConnectionName] = useState('');
  const [remoteRoot, setRemoteRoot] = useState('');
  const [remoteExecution, setRemoteExecution] = useState(false);
  const [prepared, setPrepared] = useState<PreparedDevice | null>(null);

  // Start
  const [pasted, setPasted] = useState('');
  const [parse, setParse] = useState<ParseState>('idle');
  /** The preview an `incomplete` paste produced, whose details prefill the typed fields. */
  const [partial, setPartial] = useState<CrewInvitationPreview | null>(null);
  const [terminal, setTerminal] = useState(false);
  const [socketPath, setSocketPath] = useState('');
  const [workspaceId, setWorkspaceId] = useState('');
  const [ownerUid, setOwnerUid] = useState('');
  const [workspaceKey, setWorkspaceKey] = useState('');
  const [pinned, setPinned] = useState<PinnedWorkspace | null>(null);

  // Create
  const [phase, setPhase] = useState<CreatePhase>('idle');
  const [savedId, setSavedId] = useState<string | null>(null);
  const [connectStarted, setConnectStarted] = useState(false);
  const [connectSettled, setConnectSettled] = useState(false);
  const [createNote, setCreateNote] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);

  useLayoutEffect(() => {
    if (!open || opened.current) return;
    opened.current = true;
    if (resumeContext.hostSetup && crew.connection) {
      setResumeId(crew.connectionId);
      setSavedId(crew.connectionId);
      setStep('create');
    }
  }, [open, resumeContext.hostSetup, crew.connection, crew.connectionId]);

  useEffect(() => {
    if (suggestedInstitution && !institutionEdited && !institution)
      setInstitution(suggestedInstitution);
  }, [suggestedInstitution, institutionEdited, institution]);

  const slug = workspaceSlug(nameText);
  const resumeConnection = resumeId && crew.connection?.id === resumeId ? crew.connection : null;
  const workspaceLabel =
    slug || sanitizeDisplayText(resumeContext.workspaceName) || resumeConnection?.name || '';
  const loginForServer = resumeConnection?.ssh_target ?? serverLogin.trim();
  const server = connectionServer({ id: '', ssh_target: loginForServer }) || loginForServer;
  // Once bootstrapped the workspace exists: waiting for its first verified view never traps the
  // person in the dialog, so `verifying` (like the label question) is not busy.
  const busy =
    preparing ||
    parse === 'reading' ||
    (phase !== 'idle' && phase !== 'labelling' && phase !== 'verifying');
  const portValue = port.trim() ? Number(port) : undefined;

  // ── Create, driven by the controller's state ──────────────────────────────────────────────
  // The controller's actions are bound to the selection of the render they came from, so each
  // stage runs from an effect once the controller shows the saved connection selected.
  const selected = savedId !== null && crew.connectionId === savedId;
  const listed = savedId !== null && crew.connections.some((item) => item.id === savedId);
  const verified = Boolean(
    selected && crew.snapshot && crew.observedPrivacy?.connectionId === savedId
  );

  useEffect(() => {
    if (phase !== 'connecting' || !selected || !listed || connectStarted) return;
    setConnectStarted(true);
    if (crew.connection?.status === 'connected') {
      setConnectSettled(true);
      return;
    }
    void crew.connect({ userInitiated: true }).then(() => {
      if (mounted.current) setConnectSettled(true);
    });
  }, [phase, selected, listed, connectStarted, crew, mounted]);

  useEffect(() => {
    if (phase !== 'connecting' || !connectSettled) return;
    if (crew.signIn.open) {
      setPhase('signing-in');
      return;
    }
    if (crew.lastConnectFailure || crew.connection?.status !== 'connected') {
      // The connect error renders in this dialog's `connect` slot; Create tries again.
      setPhase('idle');
      setConnectStarted(false);
      setConnectSettled(false);
      return;
    }
    setPhase('bootstrapping');
  }, [phase, connectSettled, crew]);

  useEffect(() => {
    if (phase !== 'signing-in' || crew.signIn.open || crew.isPending('sign-in')) return;
    if (crew.lastConnectFailure) {
      setPhase('idle');
      setConnectStarted(false);
      setConnectSettled(false);
      setCreateNote(hostCopy.signInEnded);
      return;
    }
    setPhase('bootstrapping');
  }, [phase, crew]);

  const bootstrapStarted = useRef(false);
  useEffect(() => {
    if (phase !== 'bootstrapping' || !selected || bootstrapStarted.current) return;
    bootstrapStarted.current = true;
    const publicKey = crew.connection?.public_key ?? '';
    const connectionId = crew.connectionId;
    void crew
      .act('dialog:host', 'auth.bootstrap', async () => {
        await crew.request('auth.bootstrap', { public_key: publicKey }, { mutation: true });
        return true;
      })
      .then(async (ok) => {
        bootstrapStarted.current = false;
        if (!mounted.current) return;
        if (!ok) {
          setPhase('idle');
          setConnectStarted(false);
          setConnectSettled(false);
          return;
        }
        updateJoinContext(connectionId, { hostSetup: false, joining: false });
        crew.setJoinStatus('joined');
        setPhase('verifying');
        await crew.refresh();
      });
  }, [phase, selected, crew, mounted]);

  const labelInstitution =
    mode === 'private' && isInstitutionId(institution.trim())
      ? institution.trim()
      : crew.connection?.institution_id && isInstitutionId(crew.connection.institution_id)
        ? crew.connection.institution_id
        : null;

  useEffect(() => {
    if (phase !== 'verifying') return;
    if (verified && crew.snapshot) {
      const workspace = crew.snapshot.workspace;
      if (workspace.mode === 'private' && !workspace.institution_id && labelInstitution) {
        setPhase('labelling');
        setStep('label');
      } else {
        setPhase('idle');
        onClose();
      }
      return;
    }
    // The workspace was created; if it cannot be observed right now, the connection bar says why.
    if (crew.refreshError) {
      setPhase('idle');
      onClose();
    }
  }, [phase, verified, crew.snapshot, crew.refreshError, labelInstitution, onClose]);

  // ── Step actions ──────────────────────────────────────────────────────────────────────────
  const continueFromName = async () => {
    setPreparing(true);
    const result = await crew.act('dialog:host', 'device.prepare', () =>
      crew.prepareHostingDevice()
    );
    if (!mounted.current) return;
    setPreparing(false);
    if (!result) return;
    setPrepared(result);
    setStep('start');
  };

  const continueFromStart = async () => {
    if (parse === 'stale' || parse === 'incomplete') {
      // The fields are required and patterned, so the form refuses a bad value before this.
      const typed = typedPinned(
        { socketPath, workspaceId, ownerUid, workspaceKey },
        parse === 'incomplete' ? partial : null
      );
      if (!typed) return;
      setPinned(typed);
      setStep('create');
      return;
    }
    setParse('reading');
    try {
      const preview = await previewInvitation(pasted);
      if (!mounted.current) return;
      if (!preview.workspace_public_key || !preview.socket_path || preview.owner_uid === null) {
        // Read, but without every detail a new workspace pins (not every daemon's preview
        // carries the socket path and host user ID): ask for the rest, prefilled with what it had.
        setPartial(preview);
        setSocketPath(preview.socket_path ?? '');
        setWorkspaceId(preview.workspace_id);
        setOwnerUid(preview.owner_uid === null ? '' : String(preview.owner_uid));
        setWorkspaceKey(preview.workspace_public_key ?? '');
        setParse('incomplete');
        return;
      }
      setPinned({
        socket_path: preview.socket_path,
        owner_uid: preview.owner_uid,
        workspace_id: preview.workspace_id,
        workspace_public_key: preview.workspace_public_key,
        fingerprint: preview.workspace_key_fingerprint,
      });
      setParse('idle');
      setStep('create');
    } catch (failure) {
      if (!mounted.current) return;
      if (crewErrorCode(failure) === CREW_INVITATION_INVALID) setParse('bad');
      else if (isStaleDaemon(failure)) setParse('stale');
      else {
        setParse('idle');
        crew.reportError(failure instanceof Error ? failure.message : hostCopy.bad, 'dialog:host');
      }
    }
  };

  const create = async () => {
    setCreateNote(null);
    if (savedId) {
      // Saved already (this dialog, or an earlier visit): connect and create.
      if (!selected) crew.selectConnection(savedId);
      setConnectStarted(false);
      setConnectSettled(false);
      setPhase('connecting');
      return;
    }
    if (!prepared || !pinned) return;
    setPhase('saving');
    const institutionId = institution.trim() || null;
    const saved = await crew.act('dialog:host', 'connection.save', () =>
      crew.saveConnection({
        name: connectionName.trim() || slug,
        ssh_target: serverLogin.trim(),
        port: portValue,
        identity_file: identityFile.trim() || undefined,
        proxy_jump: proxyJump.trim() || undefined,
        socket_path: pinned.socket_path,
        owner_uid: pinned.owner_uid,
        workspace_id: pinned.workspace_id,
        workspace_public_key: pinned.workspace_public_key,
        remote_root: remoteRoot.trim() || undefined,
        remote_execution: remoteRoot.trim() ? remoteExecution : false,
        mode,
        institution_id: institutionId,
        preparation_id: prepared.preparation_id,
      })
    );
    if (!mounted.current) return;
    if (!saved) {
      setPhase('idle');
      return;
    }
    updateJoinContext(saved.id, { hostSetup: true, workspaceName: slug });
    setSavedId(saved.id);
    setConnectStarted(false);
    setConnectSettled(false);
    setPhase('connecting');
  };

  const setLabel = async () => {
    const snapshot = crew.snapshot;
    if (!snapshot || !labelInstitution) return;
    const done = await crew.act('dialog:host', 'mutate:policy.set', async () => {
      await crew.mutate('policy.set', {
        mode: snapshot.workspace.mode,
        institution_id: labelInstitution,
      });
      return true;
    });
    if (done && mounted.current) {
      setPhase('idle');
      onClose();
    }
  };

  // A submit with an invalid field inside the closed Advanced opens it, then reports.
  const [validateHidden, setValidateHidden] = useState(false);
  useEffect(() => {
    if (!validateHidden) return;
    setValidateHidden(false);
    formRef.current?.reportValidity();
  }, [validateHidden]);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    if (step === 'name' && !advancedOpen && advancedInvalid(port, remoteRoot)) {
      setAdvancedOpen(true);
      setValidateHidden(true);
      return;
    }
    if (step === 'name') void continueFromName();
    else if (step === 'start') void continueFromStart();
    else if (step === 'create') void create();
    else if (step === 'label') void setLabel();
  };

  // ── Rendering ─────────────────────────────────────────────────────────────────────────────
  const pinnedValues = { socketPath, workspaceId, ownerUid, workspaceKey };
  const pinnedSetters = {
    socketPath: setSocketPath,
    workspaceId: setWorkspaceId,
    ownerUid: setOwnerUid,
    workspaceKey: setWorkspaceKey,
  };
  const stepName = step === 'label' ? null : hostCopy.steps[STEP_NUMBER[step] - 1];
  const subtitle = stepName
    ? hostCopy.stepOf(STEP_NUMBER[step as Exclude<Step, 'label'>], 3, stepName)
    : undefined;

  const footer = (() => {
    if (step === 'label') {
      return (
        <>
          <Button
            type="button"
            variant="ghost"
            disabled={crew.isPending('mutate:policy.set')}
            onClick={onClose}
          >
            {hostCopy.labelLater}
          </Button>
          <Button type="submit" form={formId} disabled={crew.isPending('mutate:policy.set')}>
            {hostCopy.labelSet(labelInstitution ?? '')}
          </Button>
        </>
      );
    }
    const verifying = phase === 'verifying';
    const back =
      step === 'name' || verifying
        ? null
        : step === 'start'
          ? 'name'
          : resumeId || savedId
            ? null
            : ('start' as const);
    return (
      <>
        {back ? (
          <Button type="button" variant="ghost" disabled={busy} onClick={() => setStep(back)}>
            {hostCopy.back}
          </Button>
        ) : (
          <Button type="button" variant="ghost" disabled={busy} onClick={onClose}>
            {joinCopy.cancel}
          </Button>
        )}
        <Button type="submit" form={formId} disabled={busy || verifying}>
          {step === 'name'
            ? preparing
              ? hostCopy.preparing
              : hostCopy.continue
            : step === 'start'
              ? parse === 'reading'
                ? hostCopy.reading
                : hostCopy.continue
              : phase === 'signing-in'
                ? hostCopy.signingIn
                : phase !== 'idle'
                  ? hostCopy.creating
                  : hostCopy.create}
        </Button>
      </>
    );
  })();

  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => {
        if (!next && !busy) onClose();
      }}
      size="lg"
      purpose={busy ? 'required' : 'form'}
      title={
        step === 'label'
          ? hostCopy.labelTitle(workspaceLabel, labelInstitution ?? '')
          : hostCopy.title
      }
      subtitle={subtitle}
      scrollBody
      footer={footer}
    >
      <form
        id={formId}
        ref={formRef}
        className="crew-onboard-form pb-3"
        onSubmit={submit}
        aria-busy={busy}
      >
        {step !== 'label' ? (
          <ol className="crew-onboard-steps text-supporting" aria-label={hostCopy.stepsLabel}>
            {hostCopy.steps.map((name, index) => (
              <li key={name} aria-current={STEP_NUMBER[step] === index + 1 ? 'step' : undefined}>
                {name}
              </li>
            ))}
          </ol>
        ) : null}

        <div className="crew-crossfade">
          <div key={step} className="crew-crossfade-item crew-onboard-form" data-state="open">
            {step === 'name' ? (
              <>
                <Field
                  label={hostCopy.workspaceName}
                  helper={
                    slug ? (
                      <>
                        {hostCopy.workspaceNameHelper} {hostCopy.preview(slug)}
                      </>
                    ) : (
                      hostCopy.workspaceNameHelper
                    )
                  }
                >
                  {(props) => (
                    <Input
                      {...props}
                      autoFocus
                      required
                      disabled={busy}
                      placeholder={hostCopy.workspaceNamePlaceholder}
                      value={nameText}
                      spellCheck={false}
                      autoComplete="off"
                      onChange={(event) => {
                        setNameText(event.target.value);
                        const next = workspaceSlug(event.target.value);
                        event.target.setCustomValidity(
                          event.target.value.trim() && !isWorkspaceName(next)
                            ? hostCopy.workspaceNameInvalid
                            : ''
                        );
                      }}
                    />
                  )}
                </Field>
                <Field label={hostCopy.serverLogin}>
                  {(props) => (
                    <Input
                      {...props}
                      required
                      disabled={busy}
                      placeholder={hostCopy.serverLoginPlaceholder}
                      value={serverLogin}
                      spellCheck={false}
                      autoComplete="off"
                      onChange={(event) => setServerLogin(event.target.value)}
                    />
                  )}
                </Field>
                <div className="crew-onboard-stack">
                  <PrivacyFields
                    mode={mode}
                    institution={institution}
                    disabled={busy}
                    onMode={setMode}
                    onInstitution={(value) => {
                      setInstitutionEdited(true);
                      setInstitution(value);
                    }}
                  />
                </div>
                <Disclosure
                  open={advancedOpen}
                  onOpenChange={setAdvancedOpen}
                  summary={hostCopy.advancedSummary}
                >
                  <div className="crew-onboard-form">
                    <Field label={joinCopy.port}>
                      {(props) => (
                        <Input
                          {...props}
                          type="number"
                          min={1}
                          max={65535}
                          placeholder="22"
                          disabled={busy}
                          value={port}
                          onChange={(event) => setPort(event.target.value)}
                        />
                      )}
                    </Field>
                    <Field label={joinCopy.identityFile} helper={joinCopy.identityFileHelper}>
                      {(props) => (
                        <Input
                          {...props}
                          disabled={busy}
                          value={identityFile}
                          spellCheck={false}
                          onChange={(event) => setIdentityFile(event.target.value)}
                        />
                      )}
                    </Field>
                    <Field label={hostCopy.jumpHosts}>
                      {(props) => (
                        <Input
                          {...props}
                          disabled={busy}
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
                          disabled={busy}
                          placeholder={slug}
                          value={connectionName}
                          onChange={(event) => setConnectionName(event.target.value)}
                        />
                      )}
                    </Field>
                    <Field label={joinCopy.remoteFolder} helper={joinCopy.remoteFolderHelper}>
                      {(props) => (
                        <Input
                          {...props}
                          disabled={busy}
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
                      disabled={busy || !remoteRoot.trim()}
                      onCheckedChange={setRemoteExecution}
                    />
                  </div>
                </Disclosure>
              </>
            ) : null}

            {step === 'start' && prepared ? (
              <>
                <h3 className="text-label text-text-default">{hostCopy.startHeading(server)}</h3>
                <p className="text-body text-text-default">
                  {hostCopy.runThis(server, sshUsername(serverLogin))}
                </p>
                <CopyField
                  multiline
                  label={hostCopy.commandLabel}
                  value={hostStartCommands(slug, prepared.public_key)}
                />
                <div className="crew-onboard-row">
                  <TerminalToggle open={terminal} onToggle={() => setTerminal((value) => !value)} />
                </div>
                {terminal ? <EmbeddedTerminal onClose={() => setTerminal(false)} /> : null}
                {parse === 'stale' ? (
                  <>
                    <Note tone="warning" role="status">
                      {hostCopy.staleDaemon}
                    </Note>
                    <PinnedFields disabled={busy} values={pinnedValues} onChange={pinnedSetters} />
                  </>
                ) : (
                  <>
                    <Field
                      label={hostCopy.pasted}
                      helper={parse === 'bad' ? hostCopy.bad : undefined}
                      invalid={parse === 'bad'}
                    >
                      {(props) => (
                        <textarea
                          {...props}
                          required
                          rows={4}
                          disabled={busy}
                          value={pasted}
                          placeholder={hostCopy.pastedPlaceholder}
                          spellCheck={false}
                          onChange={(event) => {
                            setPasted(event.target.value);
                            // A new paste is read again on Continue.
                            if (parse === 'bad' || parse === 'incomplete') setParse('idle');
                          }}
                          className="crew-onboard-textarea w-full rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 font-mono text-label placeholder:text-text-muted"
                        />
                      )}
                    </Field>
                    {parse === 'incomplete' ? (
                      <>
                        <Note tone="warning" role="status">
                          {hostCopy.detailsMissing}
                        </Note>
                        <PinnedFields
                          disabled={busy}
                          values={pinnedValues}
                          onChange={pinnedSetters}
                        />
                      </>
                    ) : null}
                  </>
                )}
                <Disclosure label={hostCopy.notSignedIn}>
                  <div className="crew-onboard-stack">
                    <CopyField
                      label={hostCopy.sshCommandLabel}
                      value={sshLoginCommand({
                        ssh_target: serverLogin,
                        port: portValue ?? null,
                        proxy_jump: proxyJump,
                        identity_file: identityFile,
                      })}
                    />
                    <p className="text-supporting text-text-muted">{hostCopy.confirmServer}</p>
                  </div>
                </Disclosure>
                <Disclosure label={hostCopy.notInstalled}>
                  <CopyField multiline label={hostCopy.installLabel} value={INSTALL_COMMANDS} />
                </Disclosure>
                <p className="text-supporting text-text-muted">{hostCopy.consequence}</p>
              </>
            ) : null}

            {step === 'create' ? (
              <>
                <h3 className="text-label text-text-default">
                  {hostCopy.createHeading(workspaceLabel, server)}
                </h3>
                {pinned?.fingerprint ? (
                  <div className="crew-onboard-field">
                    <span className="text-supporting text-text-muted">{joinCopy.fingerprint}</span>
                    <CopyField
                      value={pinned.fingerprint}
                      display={groupWorkspaceFingerprint(pinned.fingerprint) ?? pinned.fingerprint}
                      label={joinCopy.fingerprintLabel}
                    />
                  </div>
                ) : null}
                <p className="text-body text-text-default">{hostCopy.createBody(workspaceLabel)}</p>
                {createNote ? (
                  <Note tone="warning" role="status">
                    {createNote}
                  </Note>
                ) : null}
                <ErrorSlot source="connect" />
              </>
            ) : null}

            {step === 'label' ? (
              <p className="text-body text-text-default">{hostCopy.labelBody}</p>
            ) : null}
          </div>
        </div>
        <ErrorSlot source="dialog:host" />
      </form>
    </ModalShell>
  );
}

type PinnedKey = 'socketPath' | 'workspaceId' | 'ownerUid' | 'workspaceKey';

/** The four details that pin a workspace, typed by the person (all required). */
function PinnedFields({
  disabled,
  values,
  onChange,
}: {
  disabled: boolean;
  values: Record<PinnedKey, string>;
  onChange: Record<PinnedKey, (value: string) => void>;
}) {
  return (
    <>
      <Field label={joinCopy.socketPath}>
        {(props) => (
          <Input
            {...props}
            required
            pattern="/.*"
            disabled={disabled}
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
            pattern={WORKSPACE_KEY_PATTERN}
            disabled={disabled}
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

/** A dialog's own error slot: an error from `source` renders here, once, while it is mounted. */
function ErrorSlot({ source }: { source: ErrorSource }) {
  const { error } = useCrew();
  const here = useCrewErrorSlot(source);
  if (!here || !error) return null;
  return (
    <Note tone="danger" role="alert">
      {error.message}
    </Note>
  );
}
