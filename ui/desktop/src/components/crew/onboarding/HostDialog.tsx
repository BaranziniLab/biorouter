import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
} from 'react';
import { useConfig } from '../../ConfigContext';
import { Check } from '../../icons/app-icons';
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
import { focusIsLost } from '../state/focusReturn';
import type { ErrorSource, PreparedDevice } from '../state/types';
import { hostCopy, INSTALL_COMMANDS, joinCopy } from './copy';
import {
  AgentAccessFields,
  Field,
  FieldErrorsProvider,
  PrivacyFields,
  remoteFolderInvalid,
  useFormValidation,
  useInitialFocus,
  useMounted,
  useOpenGeneration,
} from './fields';
import {
  HOST_START_POLL_MS,
  readHostRun,
  startHostRun,
  stopHostRun,
  type HostStartRun,
} from './hostStart';
import { updateJoinContext, useJoinContext } from './joinContext';
import { advancedInvalid, WORKSPACE_KEY_PATTERN } from './JoinDialog';
import {
  connectionServerLabel,
  groupWorkspaceFingerprint,
  hostStartCommands,
  isWorkspaceName,
  readStartOutput,
  sshLoginCommand,
  sshUsername,
  workspaceSlug,
  type StartOutput,
} from './joinText';
import { EmbeddedTerminal, Spinner, TerminalToggle } from './parts';

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
 * How the Start step's paste stands. It is read as soon as it is pasted (T-27), so Continue only
 * moves on. `checking`: that inline read is running; the form is not busy. `reading`: Continue is
 * reading it, and the form waits. `found`: the daemon read every detail the workspace pins.
 * `bad`: the paste can't be used, and `problem` says exactly why. `stale`: the daemon cannot read
 * one (404), so the person types the four details. `incomplete`: it read one, but its preview
 * lacks a detail the workspace pins (the socket path, the host user ID or the key), so the person
 * types what is missing — the paste itself was fine, so it is never called bad.
 */
type ParseState = 'idle' | 'checking' | 'reading' | 'found' | 'bad' | 'stale' | 'incomplete';

/** How long typing pauses before the Start step's paste is read. */
export const START_PASTE_CHECK_DELAY_MS = 250;

/** What reading the Start step's paste came to. */
type PasteOutcome =
  | { kind: 'found'; preview: CrewInvitationPreview }
  | { kind: 'incomplete'; preview: CrewInvitationPreview }
  | { kind: 'bad'; message: string }
  | { kind: 'stale' }
  | { kind: 'failed'; message: string };

function problemText(output: Extract<StartOutput, { kind: 'problem' }>): string {
  switch (output.problem) {
    case 'starting':
      return hostCopy.pasteStarting;
    case 'cut-off':
      return hostCopy.pasteCutOff;
    case 'not-installed':
      return hostCopy.pasteNotInstalled;
    case 'server-error':
      return hostCopy.pasteServerError(sanitizeDisplayText(output.detail) || hostCopy.bad);
  }
}

/**
 * Read a paste: find what the daemon reads inside the terminal text (`readStartOutput`), then ask
 * the daemon. Nothing is saved.
 */
async function readPaste(pasted: string, signal?: AbortSignal): Promise<PasteOutcome> {
  const output = readStartOutput(pasted);
  if (!output) return { kind: 'bad', message: hostCopy.bad };
  if (output.kind === 'problem') return { kind: 'bad', message: problemText(output) };
  try {
    const preview = signal
      ? await previewInvitation(output.text, {}, signal)
      : await previewInvitation(output.text);
    if (!preview.workspace_public_key || !preview.socket_path || preview.owner_uid === null) {
      return { kind: 'incomplete', preview };
    }
    return { kind: 'found', preview };
  } catch (failure) {
    if (crewErrorCode(failure) === CREW_INVITATION_INVALID) {
      return { kind: 'bad', message: hostCopy.bad };
    }
    if (isStaleDaemon(failure)) return { kind: 'stale' };
    return { kind: 'failed', message: failure instanceof Error ? failure.message : hostCopy.bad };
  }
}

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

/**
 * Where "Start it for me" stands (D-HOST). `starting`: the request is on its way. `running`: the
 * daemon is running the commands; `output` grows as they print. `reading`: they finished and
 * printed Crew's answer, which is being read as a paste is. `problem`: it didn't get there, and
 * `message` says why and what to do instead.
 */
type StartRun =
  | { phase: 'idle' }
  | { phase: 'starting' }
  /** `reads`: how many answers have come back, so each one schedules the next read. */
  | { phase: 'running'; jobId: string; output: string; reads: number }
  | { phase: 'reading'; output: string }
  | { phase: 'problem'; output: string; message: string };

/** The sentence for what a finished run printed when it was not Crew's answer. */
function startProblemText(problem: string, detail: string | null): string {
  switch (problem) {
    case 'starting':
      return hostCopy.startStarting;
    case 'not_installed':
      return hostCopy.pasteNotInstalled;
    case 'server_error':
      return hostCopy.pasteServerError(sanitizeDisplayText(detail) || hostCopy.startUnreadable);
    default:
      return hostCopy.startUnreadable;
  }
}

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
  const { errors, validate, formProps } = useFormValidation();
  const suggestedInstitution = useSingleInstitution(open);
  // The controller as it stands now, for work that continues after an await.
  const latest = useRef(crew);
  useEffect(() => {
    latest.current = crew;
  });

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
  /** "Agent on {server}": its own row, folded, off unless the host turns it on (Q2-37). */
  const [agentOpen, setAgentOpen] = useState(false);
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
  /** The paste the daemon read in full, and what it read: Continue uses it as it is. */
  const [found, setFound] = useState<{ text: string; preview: CrewInvitationPreview } | null>(null);
  /** Why the paste can't be used, when `parse` is `bad`. */
  const [problem, setProblem] = useState<string | null>(null);
  /** Cancels the inline read that is waiting or running, so Continue reads instead. */
  const cancelCheck = useRef<() => void>(() => {});
  const pasteRef = useRef<HTMLTextAreaElement>(null);
  const staleFieldsRef = useRef<HTMLDivElement>(null);
  /** The paste box had focus when an older daemon replaced it with typed fields: follow it. */
  const [focusTyped, setFocusTyped] = useState(false);
  /** The preview an `incomplete` paste produced, whose details prefill the typed fields. */
  const [partial, setPartial] = useState<CrewInvitationPreview | null>(null);
  const [terminal, setTerminal] = useState(false);
  /** "Run it yourself in a terminal": the manual path, folded under "Start it for me" (D-HOST). */
  const [manualOpen, setManualOpen] = useState(false);
  const [run, setRun] = useState<StartRun>({ phase: 'idle' });
  const startRef = useRef<HTMLButtonElement>(null);
  const outputRef = useRef<HTMLPreElement>(null);
  const submitRef = useRef<HTMLButtonElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  /** Where focus goes once the step it was on has gone: Start it for me, or Create workspace. */
  const [focusNext, setFocusNext] = useState<'start' | 'submit' | null>(null);
  const startHintId = useId();
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

  // Open on the workspace name, and keep focus there while the menu that opened the dialog closes
  // (Q2-27). A host setup that resumes at Create opens on Create's own content instead.
  useInitialFocus(nameRef, open && step === 'name');

  const slug = workspaceSlug(nameText);
  const resumeConnection = resumeId && crew.connection?.id === resumeId ? crew.connection : null;
  const workspaceLabel =
    slug || sanitizeDisplayText(resumeContext.workspaceName) || resumeConnection?.name || '';
  const loginForServer = resumeConnection?.ssh_target ?? serverLogin.trim();
  // What to call the server (D-ALIAS): once the connection is saved, the daemon's name for it (the
  // host's own SSH alias for the address, when one maps to it); before that, the login's host as
  // typed, which is already the alias when the host typed one.
  const savedConnection =
    resumeConnection ?? crew.connections.find((item) => item.id === savedId) ?? null;
  const server =
    connectionServerLabel(savedConnection) ||
    connectionServer({ id: '', ssh_target: loginForServer }) ||
    loginForServer;
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
  /** The observation error that stood when verifying began (T-09). */
  const staleRefreshError = useRef<string | null>(null);
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
        // An observation error from before the workspace existed is not a verdict on it: only a
        // new one closes the dialog while it verifies (T-09).
        staleRefreshError.current = latest.current.refreshError;
        setPhase('verifying');
        // The workspace exists now. Open it connected: a connection that dropped (or whose
        // observer gave up while the workspace did not exist yet) is connected again first, so
        // the host never lands on an offline workspace they just created.
        const now = latest.current;
        if (now.connectionId === connectionId && now.connection?.status !== 'connected') {
          await now.connect({ userInitiated: true }).catch(() => undefined);
        }
        await latest.current.refresh().catch(() => undefined);
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
    // Only an error that arose while verifying counts: one that was already there when the
    // workspace was created described the connection before it existed.
    if (!crew.refreshError) {
      staleRefreshError.current = null;
      return;
    }
    if (crew.refreshError !== staleRefreshError.current) {
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
    // A run from an earlier visit to Start described the login as it was then.
    setRun({ phase: 'idle' });
    setStep('start');
    // Continue leaves with the Name step's footer; the Start step's own action takes focus.
    setFocusNext('start');
  };

  /** Hand the Start step's outcome to the form: pin, ask for the rest, or say what's wrong. */
  const applyOutcome = useCallback((outcome: PasteOutcome, text: string) => {
    setFound(null);
    setProblem(null);
    switch (outcome.kind) {
      case 'found':
        setFound({ text, preview: outcome.preview });
        setParse('found');
        return;
      case 'incomplete': {
        // Read, but without every detail a new workspace pins (not every daemon's preview
        // carries the socket path and host user ID): ask for the rest, prefilled with what it had.
        const { preview } = outcome;
        setPartial(preview);
        setSocketPath(preview.socket_path ?? '');
        setWorkspaceId(preview.workspace_id);
        setOwnerUid(preview.owner_uid === null ? '' : String(preview.owner_uid));
        setWorkspaceKey(preview.workspace_public_key ?? '');
        setParse('incomplete');
        return;
      }
      case 'bad':
        setProblem(outcome.message);
        setParse('bad');
        return;
      case 'stale':
        if (document.activeElement === pasteRef.current) setFocusTyped(true);
        setParse('stale');
        return;
      case 'failed':
        setParse('idle');
        return;
    }
  }, []);

  const pinFound = (preview: CrewInvitationPreview) => {
    if (!preview.workspace_public_key || !preview.socket_path || preview.owner_uid === null) return;
    setPinned({
      socket_path: preview.socket_path,
      owner_uid: preview.owner_uid,
      workspace_id: preview.workspace_id,
      workspace_public_key: preview.workspace_public_key,
      fingerprint: preview.workspace_key_fingerprint,
    });
    setStep('create');
  };

  // ── Start it for me (D-HOST) ───────────────────────────────────────────────────────────────
  /** The commands the Start step shows: what the daemon must run, character for character. */
  const shownCommands = prepared ? hostStartCommands(slug, prepared.public_key) : '';
  const runActive = run.phase === 'starting' || run.phase === 'running' || run.phase === 'reading';

  /** A run that didn't get there: say why, and open the manual path, which always works. */
  const runProblem = useCallback((message: string, output = '') => {
    setRun({ phase: 'problem', output, message });
    setManualOpen(true);
  }, []);

  /**
   * On the person's click only. The request names the host setup, the workspace name and the
   * login they typed; the daemon builds the command from its own hosting key, runs it over SSH as
   * that login and nothing else, and refuses without proof that a person asked.
   */
  const startForMe = async () => {
    if (!prepared || runActive || busy) return;
    const shown = shownCommands;
    setRun({ phase: 'starting' });
    let status: HostStartRun;
    try {
      status = await startHostRun({
        preparation_id: prepared.preparation_id,
        workspace_name: slug,
        ssh_target: serverLogin.trim(),
        port: portValue ?? null,
        identity_file: identityFile.trim() || null,
        proxy_jump: proxyJump.trim() || null,
      });
    } catch (failure) {
      if (!mounted.current) return;
      runProblem(
        isStaleDaemon(failure)
          ? hostCopy.startStaleDaemon
          : failure instanceof Error && failure.message
            ? failure.message
            : hostCopy.startFailed
      );
      return;
    }
    if (!mounted.current) return;
    // The daemon runs its own copy of the commands. One that differs from what the person saw is
    // stopped at once and never read (a tripwire: the two are pinned together by tests).
    if (status.command !== shown) {
      void stopHostRun(status.jobId).catch(() => undefined);
      runProblem(hostCopy.startCommandChanged);
      return;
    }
    void settleRun(status);
  };

  /** Follow a run to its end, then read what it printed as a paste is read. */
  const settleRun = async (status: HostStartRun) => {
    if (status.state === 'running') {
      setRun((current) => ({
        phase: 'running',
        jobId: status.jobId,
        output: status.output,
        reads: current.phase === 'running' ? current.reads + 1 : 0,
      }));
      return;
    }
    if (status.state === 'failed' || !status.result) {
      runProblem(
        sanitizeDisplayText(status.error?.message) ||
          (status.result ? hostCopy.startFailed : hostCopy.startUnreadable),
        status.output
      );
      return;
    }
    if (status.result.kind === 'problem') {
      runProblem(startProblemText(status.result.problem, status.result.detail), status.output);
      return;
    }
    setRun({ phase: 'reading', output: status.output });
    const outcome = await readPaste(status.result.text);
    if (!mounted.current) return;
    switch (outcome.kind) {
      case 'found':
        setRun({ phase: 'idle' });
        pinFound(outcome.preview);
        // Start it for me leaves with the Start step: Create workspace takes focus.
        setFocusNext('submit');
        return;
      case 'incomplete':
      case 'stale':
        // Read, but the rest has to be typed: the fields appear below, prefilled where they can.
        setRun({ phase: 'idle' });
        applyOutcome(outcome, '');
        return;
      case 'bad':
        runProblem(hostCopy.startUnreadable, status.output);
        return;
      case 'failed':
        runProblem(outcome.message, status.output);
        return;
    }
  };

  // Read a running start again until it ends. Closing the dialog stops reading, not the run: a
  // start stopped halfway could leave the server half set up, and choosing Start it for me again
  // answers the same run.
  const runningJob = run.phase === 'running' ? run.jobId : null;
  const runningOutput = run.phase === 'running' ? run.output : null;
  const runningReads = run.phase === 'running' ? run.reads : null;
  useEffect(() => {
    if (!runningJob) return;
    const controller = new AbortController();
    const timer = setTimeout(() => {
      readHostRun(runningJob, controller.signal).then(
        (status) => {
          if (controller.signal.aborted || !mounted.current) return;
          void settleRun(status);
        },
        (failure: unknown) => {
          if (controller.signal.aborted || !mounted.current) return;
          runProblem(
            failure instanceof Error && failure.message ? failure.message : hostCopy.startFailed,
            runningOutput ?? ''
          );
        }
      );
    }, HOST_START_POLL_MS);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
    // `settleRun` is recreated every render; each answer that comes back schedules the next read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runningJob, runningReads, mounted, runProblem]);

  const stopRun = () => {
    if (run.phase !== 'running') return;
    void stopHostRun(run.jobId).catch(() => undefined);
  };

  // Stop leaves with the run: pressing it ends the run, and a run that ends by itself takes it too.
  // Focus that was on it would fall to the page or the dialog's frame (Q2-20, Q2-27), so it goes to
  // Start it for me, which stays. Only when focus is lost: a person who moved on keeps their place.
  const runPhase = run.phase;
  const lastRunPhase = useRef(runPhase);
  useLayoutEffect(() => {
    const left = lastRunPhase.current === 'running' && runPhase !== 'running';
    lastRunPhase.current = runPhase;
    if (left && focusIsLost()) startRef.current?.focus();
  }, [runPhase]);

  // Keep the newest output in view as it arrives.
  const shownOutput = run.phase === 'idle' || run.phase === 'starting' ? '' : run.output;
  useEffect(() => {
    const box = outputRef.current;
    if (box) box.scrollTop = box.scrollHeight;
  }, [shownOutput]);

  // Hand focus to the step's own action once the control that had it has gone with its step.
  useLayoutEffect(() => {
    if (!focusNext) return;
    const target = focusNext === 'start' ? startRef.current : submitRef.current;
    if (!target) return;
    setFocusNext(null);
    target.focus();
  }, [focusNext, step]);

  // Read the paste as soon as it is pasted, so the person sees "Found lab on hpc ✓" or exactly
  // what is wrong before Continue (T-27). Nothing is saved; the daemon still does the reading.
  useEffect(() => {
    if (step !== 'start' || !pasted.trim()) return;
    const controller = new AbortController();
    const timer = setTimeout(() => {
      void readPaste(pasted, controller.signal).then((outcome) => {
        if (controller.signal.aborted || !mounted.current) return;
        applyOutcome(outcome, pasted);
      });
    }, START_PASTE_CHECK_DELAY_MS);
    const cancel = () => {
      clearTimeout(timer);
      controller.abort();
    };
    cancelCheck.current = cancel;
    return cancel;
  }, [pasted, step, mounted, applyOutcome]);

  useLayoutEffect(() => {
    if (!focusTyped || parse !== 'stale') return;
    setFocusTyped(false);
    staleFieldsRef.current?.querySelector<HTMLInputElement>('input')?.focus();
  }, [focusTyped, parse]);

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
    // Already read in full: move on without asking again.
    if (parse === 'found' && found?.text === pasted) {
      pinFound(found.preview);
      return;
    }
    cancelCheck.current();
    setParse('reading');
    const outcome = await readPaste(pasted);
    if (!mounted.current) return;
    if (outcome.kind === 'failed') {
      setParse('idle');
      crew.reportError(outcome.message, 'dialog:host');
      return;
    }
    applyOutcome(outcome, pasted);
    if (outcome.kind === 'found') pinFound(outcome.preview);
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

  // A submit with an invalid field inside a folded section (Advanced, the agent row, the manual
  // path) opens it, then reports.
  const [validateHidden, setValidateHidden] = useState(false);
  useEffect(() => {
    if (!validateHidden) return;
    setValidateHidden(false);
    validate();
  }, [validateHidden, validate]);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    if (step === 'name') {
      const agentHidden = !agentOpen && remoteFolderInvalid(remoteRoot);
      const advancedHidden = !advancedOpen && advancedInvalid(port, '');
      if (agentHidden || advancedHidden) {
        if (agentHidden) setAgentOpen(true);
        if (advancedHidden) setAdvancedOpen(true);
        setValidateHidden(true);
        return;
      }
    }
    if (step === 'start') {
      // Start it for me is running: it moves on by itself.
      if (runActive) return;
      // Continue belongs to the manual path: a submit while it is folded opens it and asks for
      // the paste there.
      if (!manualOpen && parse !== 'stale' && parse !== 'incomplete') {
        setManualOpen(true);
        setValidateHidden(true);
        return;
      }
    }
    // The form is `noValidate`: its fields are checked here and answered under each field.
    if (!validate()) return;
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
  // One numbered stepper row, in the header so it never scrolls away (T-42). It is also the
  // dialog's description: a screen reader hears "Step 2 of 3 · Start", the eye sees the row.
  const current = step === 'label' ? null : STEP_NUMBER[step];
  const subtitle =
    current === null ? undefined : (
      <>
        <span className="sr-only">
          {hostCopy.stepOf(current, hostCopy.steps.length, hostCopy.steps[current - 1])}
        </span>
        <span className="crew-onboard-steps" aria-hidden="true" data-testid="crew-host-steps">
          {hostCopy.steps.map((name, index) => {
            const number = index + 1;
            const state = number < current ? 'done' : number === current ? 'current' : 'next';
            return (
              <span key={name} className="crew-onboard-step" data-state={state}>
                <span className="crew-onboard-step-number">
                  {state === 'done' ? <Check className="h-3 w-3" /> : number}
                </span>
                {name}
              </span>
            );
          })}
        </span>
      </>
    );
  const foundServer = server || hostCopy.theServer;

  const footer = (() => {
    if (step === 'label') {
      return (
        <>
          <Button
            type="button"
            variant="secondary"
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
    // On Start, Continue belongs to the manual path (and to the details typed by hand): Start it
    // for me is the step's action, and moves on by itself.
    const showSubmit =
      step !== 'start' || manualOpen || parse === 'stale' || parse === 'incomplete';
    return (
      <>
        {back ? (
          <Button
            type="button"
            variant="secondary"
            disabled={busy || runActive}
            onClick={() => setStep(back)}
          >
            {hostCopy.back}
          </Button>
        ) : (
          <Button type="button" variant="secondary" disabled={busy} onClick={onClose}>
            {joinCopy.cancel}
          </Button>
        )}
        {showSubmit ? (
          <Button ref={submitRef} type="submit" form={formId} disabled={busy || verifying}>
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
        ) : null}
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
      // A dialog portals outside `.crew-app`: the class carries Crew's focused-field edge to it
      // (Q2-25), and the top anchor keeps it from re-centring as each step changes its height
      // (Q2-26).
      className="crew-dialog"
      anchor="top"
      footer={footer}
    >
      <FieldErrorsProvider value={errors}>
        <form
          id={formId}
          {...formProps}
          className="crew-onboard-form crew-onboard-dialog-form"
          onSubmit={submit}
          aria-busy={busy}
        >
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
                        ref={nameRef}
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
                      institutionHelper={hostCopy.institutionHelper}
                      onMode={setMode}
                      onInstitution={(value) => {
                        setInstitutionEdited(true);
                        setInstitution(value);
                      }}
                    />
                  </div>
                  <AgentAccessFields
                    server={server}
                    open={agentOpen}
                    onOpenChange={setAgentOpen}
                    remoteRoot={remoteRoot}
                    remoteExecution={remoteExecution}
                    disabled={busy}
                    onRemoteRoot={setRemoteRoot}
                    onRemoteExecution={setRemoteExecution}
                  />
                  <Disclosure
                    open={advancedOpen}
                    onOpenChange={setAdvancedOpen}
                    summary={hostCopy.advancedSummary}
                  >
                    <div className="crew-onboard-form">
                      <Field label={joinCopy.port} invalidMessage={joinCopy.portInvalid}>
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
                            spellCheck={false}
                            onChange={(event) => setConnectionName(event.target.value)}
                          />
                        )}
                      </Field>
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
                  {/* One command per line, scrolling sideways rather than wrapping mid-flag. The
                      same text both ways: Start it for me runs exactly this (D-HOST). */}
                  <CopyField
                    multiline
                    className="crew-onboard-command"
                    label={hostCopy.commandLabel}
                    value={shownCommands}
                  />
                  <div className="crew-onboard-stack" data-testid="crew-host-start-for-me">
                    <div className="crew-onboard-row">
                      <Button
                        ref={startRef}
                        type="button"
                        // The step's action, until the person takes the manual path instead.
                        variant={manualOpen ? 'secondary' : 'default'}
                        // Not `disabled` while it runs: a disabled control drops keyboard focus
                        // to the page (Q2-20). It says so, and does nothing.
                        aria-disabled={runActive || busy || undefined}
                        className="crew-onboard-waiting"
                        aria-describedby={startHintId}
                        onClick={() => void startForMe()}
                      >
                        {hostCopy.startForMe}
                      </Button>
                      {run.phase === 'running' ? (
                        <Button type="button" variant="secondary" onClick={stopRun}>
                          {hostCopy.stop}
                        </Button>
                      ) : null}
                    </div>
                    <p id={startHintId} className="text-supporting text-text-muted">
                      {hostCopy.startForMeHint(server, sshUsername(serverLogin))}
                    </p>
                    <p className="crew-onboard-row text-supporting text-text-muted" role="status">
                      {runActive ? (
                        <>
                          <Spinner />
                          {run.phase === 'reading'
                            ? hostCopy.startReading
                            : hostCopy.startRunning(server)}
                        </>
                      ) : null}
                    </p>
                    {shownOutput ? (
                      // What the commands printed, as it arrives. A scrolling region a keyboard
                      // can reach; not a live region, which would read every line aloud.
                      <pre
                        ref={outputRef}
                        tabIndex={0}
                        aria-label={hostCopy.startOutput}
                        className="crew-onboard-output font-mono text-supporting text-text-default"
                        data-testid="crew-host-start-output"
                      >
                        {shownOutput}
                      </pre>
                    ) : null}
                    {run.phase === 'problem' ? (
                      <Note tone="warning" role="alert" testId="crew-host-start-problem">
                        {run.message}
                      </Note>
                    ) : null}
                  </div>
                  {parse === 'stale' ? (
                    <div className="crew-onboard-form" ref={staleFieldsRef}>
                      <Note tone="warning" role="status">
                        {hostCopy.staleDaemon}
                      </Note>
                      <PinnedFields
                        disabled={busy}
                        values={pinnedValues}
                        onChange={pinnedSetters}
                      />
                    </div>
                  ) : parse === 'incomplete' ? (
                    <div className="crew-onboard-form">
                      <Note tone="warning" role="status">
                        {hostCopy.detailsMissing}
                      </Note>
                      <PinnedFields
                        disabled={busy}
                        values={pinnedValues}
                        onChange={pinnedSetters}
                      />
                    </div>
                  ) : null}
                  <Disclosure
                    label={hostCopy.runYourself}
                    open={manualOpen}
                    onOpenChange={setManualOpen}
                  >
                    <div className="crew-onboard-form" data-testid="crew-host-run-yourself">
                      <p className="text-body text-text-default">
                        {hostCopy.runYourselfBody(server, sshUsername(serverLogin))}
                      </p>
                      <div className="crew-onboard-stack">
                        <p className="text-supporting text-text-muted">{hostCopy.notSignedIn}</p>
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
                      <div className="crew-onboard-row">
                        <TerminalToggle
                          open={terminal}
                          onToggle={() => setTerminal((value) => !value)}
                        />
                      </div>
                      {terminal ? <EmbeddedTerminal onClose={() => setTerminal(false)} /> : null}
                      {parse !== 'stale' ? (
                        <Field
                          label={hostCopy.pasted}
                          live
                          helper={
                            parse === 'bad' ? (
                              (problem ?? hostCopy.bad)
                            ) : parse === 'checking' ? (
                              hostCopy.checkingPaste
                            ) : parse === 'found' && found ? (
                              <span
                                className="crew-onboard-found text-text-success"
                                data-testid="crew-host-paste-found"
                              >
                                {hostCopy.found(
                                  sanitizeDisplayText(found.preview.workspace_name) ||
                                    workspaceLabel,
                                  foundServer
                                )}
                                <Check aria-hidden className="h-3.5 w-3.5" />
                              </span>
                            ) : undefined
                          }
                          invalid={parse === 'bad'}
                        >
                          {(props) => (
                            <textarea
                              {...props}
                              required
                              rows={4}
                              disabled={busy}
                              value={pasted}
                              ref={pasteRef}
                              placeholder={hostCopy.pastedPlaceholder}
                              spellCheck={false}
                              onChange={(event) => {
                                setPasted(event.target.value);
                                // A new paste is read again, a moment after typing stops.
                                setFound(null);
                                setProblem(null);
                                setParse(event.target.value.trim() ? 'checking' : 'idle');
                              }}
                              className="crew-onboard-textarea w-full rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 font-mono text-label placeholder:text-text-muted"
                            />
                          )}
                        </Field>
                      ) : null}
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
                      <span className="text-supporting text-text-muted">
                        {joinCopy.fingerprint}
                      </span>
                      <CopyField
                        value={pinned.fingerprint}
                        display={
                          groupWorkspaceFingerprint(pinned.fingerprint) ?? pinned.fingerprint
                        }
                        label={joinCopy.fingerprintLabel}
                      />
                      <p className="text-supporting text-text-muted">
                        {hostCopy.fingerprintHelper}
                      </p>
                    </div>
                  ) : null}
                  <p className="text-body text-text-default">
                    {hostCopy.createBody(workspaceLabel)}
                  </p>
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
      </FieldErrorsProvider>
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
      <Field label={joinCopy.socketPath} invalidMessage={joinCopy.socketPathInvalid}>
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
