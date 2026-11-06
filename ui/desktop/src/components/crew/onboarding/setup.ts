import { crewHttp, CrewHttpError } from '../crewApi';
import { isStaleDaemon, outdatedDaemonResponse, unexpectedCrewResponse } from '../api/errors';
import { isRecord } from '../api/parse';
import { hostStartResultFrom, type HostStartResult } from './hostStart';

/**
 * Automatic setup (OW-9): the daemon installs Crew's server program in the person's own account on a
 * server, then starts a workspace there (a host) or connects (a member). The Host dialog's Start
 * step, the Not set up pane and Join use it; `biorouter crew setup` is the same service from a
 * terminal.
 *
 * The renderer never sends a command, a path or the program's bytes. It names a target (a saved
 * connection, or this computer's pending host setup and the login the person typed) and what the
 * job is for; the daemon runs fixed commands, sends the broker bundled with this build over the SSH
 * session's input, checks its SHA-256 and installs it at `~/.local/bin/biorouter-crew`. Every call
 * goes through `crewHttp`, so each carries the proof that a person asked, and the daemon refuses
 * any that does not. These routes are not in the generated client yet: every answer is checked
 * here, field by field, and a body that is not one is never read as one.
 */

export type SetupIntent = 'check' | 'install' | 'host' | 'member';
export type SetupState = 'running' | 'needs_sign_in' | 'finished' | 'failed' | 'cancelled';
export type SetupStageState = 'pending' | 'running' | 'done' | 'skipped' | 'failed';

/**
 * What a job works on: a saved connection, this computer's pending host setup, or a server login
 * with no workspace in mind (account ownership AO-8: Open a workspace on a server checks it with
 * the device it prepared, so the check and the open share one sign-in).
 */
export type SetupTarget =
  | { kind: 'connection'; connection_id: string }
  | {
      kind: 'pending';
      preparation_id: string;
      ssh_target: string;
      port?: number | null;
      identity_file?: string | null;
      proxy_jump?: string | null;
    }
  | {
      kind: 'server';
      ssh_target: string;
      preparation_id?: string | null;
      port?: number | null;
      identity_file?: string | null;
      proxy_jump?: string | null;
    };

/**
 * What a connection's agent may do on its server (OW-6): no work folder, the confined folder, or
 * commands as the person's own account. The same three levels the Join dialog and Connection
 * settings offer.
 */
export type SetupAgentAccess = 'off' | 'folder' | 'full';

export interface SetupInput {
  target: SetupTarget;
  intent: SetupIntent;
  /** A host job's workspace name (the slug the Name step previewed). */
  workspace_name?: string;
  /** Replace an older or incompatible install (the previous copy is kept beside it). */
  replace_existing?: boolean;
  /**
   * A member job: the level the person chose for this connection's agent (C10, C18). Absent, the
   * job leaves the connection's agent access exactly as it is; it never infers a level from a
   * connection that has no work folder, because Off is saved that way too.
   */
  agent_access?: SetupAgentAccess;
  /**
   * A host job: take write access for other people off the folders the server check named
   * (`crew_host_storage_unsafe`), with `chmod go-w` on exactly those, then go on. Only on the
   * person's "Fix it for me".
   */
  fix_storage_permissions?: boolean;
  /**
   * Restart the account's running workspaces that run an older Crew even when other people are
   * connected (AO-8). Absent, the daemon restarts only those nobody else is connected to; `false`
   * only reports them.
   */
  restart_workspaces?: boolean;
  /** Start these stopped workspaces the account hosts, by ID (AO-7, AO-8). */
  start_workspaces?: string[];
}

/** What a server account has to do with a workspace the setup found (AO-6). */
export type SetupWorkspaceRole = 'host' | 'member' | 'invited' | 'removed';
/** Where a found workspace stands on its server (AO-6). */
export type SetupWorkspaceState = 'running' | 'stopped' | 'deleted' | 'storage_failed' | 'unknown';

/**
 * A workspace on the server the setup signed in to, as `biorouter-crew workspaces` listed it
 * (AO-6, AO-8). Display and the way to open it; the workspace decides every join.
 */
export interface SetupWorkspace {
  workspaceId: string;
  name: string | null;
  role: SetupWorkspaceRole;
  state: SetupWorkspaceState;
  /** The host's server account, resolved on the server from the socket's owner. */
  hostUsername: string | null;
  /** The workspace's key fingerprint, for comparing with its host's. */
  fingerprint: string | null;
  mode: 'private' | 'public' | null;
  institutionId: string | null;
  /** The running broker's own version; null when stopped or when it does not say. */
  version: string | null;
  /** The running broker is the installed program; null when not running. */
  binaryCurrent: boolean | null;
  /** How many other accounts are connected now: the host's own entries only. */
  othersConnected: number | null;
  /** The `brcrew1:` line that saves it on this computer, without the server. */
  invitation: string | null;
  /** The saved connection on this computer for it, when there is one. */
  savedConnectionId: string | null;
  thisComputer: 'connected' | 'saved' | 'not_saved';
  /** Why the entry could not be read (`state: unknown`), in the server program's words. */
  error: string | null;
}

/** A workspace restarted (or started) on the newly installed Crew. */
export interface SetupRestarted {
  workspaceId: string;
  name: string | null;
  fromVersion: string | null;
  toVersion: string | null;
}

/** Why a running workspace was left on its older Crew (AO-8). */
export type SetupOutdatedReason =
  | 'others_connected'
  | 'running_newer'
  | 'not_running_installed'
  | 'custom_state_dir';

export interface SetupOutdated {
  workspaceId: string;
  name: string | null;
  runningVersion: string | null;
  installedVersion: string | null;
  othersConnected: number | null;
  reason: SetupOutdatedReason;
}

export interface SetupRestartFailed {
  workspaceId: string;
  name: string | null;
  code: string;
  message: string;
  /** The command that starts it by hand. */
  command: string | null;
}

/** The broker this build carries, as `GET /crew/setup/artifact` describes it. */
export interface SetupArtifact {
  version: string;
  target: string;
  sha256: string;
  size: number;
  /** `bundled` in a release, `dev` from `BIOROUTER_CREW_BROKER_DIR`. */
  source: string;
}

export interface SetupStage {
  /** The daemon's stage name (`probe`, `sign_in`, `transfer`, …). */
  name: string;
  state: SetupStageState;
  detail: string | null;
}

/** What a finished job did. */
export interface SetupOutcome {
  installedVersion: string | null;
  /** The install already there was kept: nothing was sent. */
  reused: boolean;
  /** The program this job replaced stays as `~/.local/bin/biorouter-crew.previous`. */
  previousKept: boolean;
  /** The version of the program this job replaced (`upgraded_from.version`), when it named one. */
  replacedVersion: string | null;
  /** A host job: what the start commands printed, read as `/crew/host/start` reads it. */
  host: HostStartResult | null;
  /** A host job: the exact start command text it ran, for the Host dialog's tripwire. */
  hostCommand: string | null;
  /** A member job: the connection it connected. */
  connectionId: string | null;
  /** The work folder the job created on the server (`~/crew-work/<workspace>`, canonical). */
  workFolder: string | null;
  /** The agent access the job gave the connection with that folder, when it gave one. */
  remoteAccess: 'folder' | 'full' | null;
  /** A host job: the first context document, drawn from the server's fixed probe (OW-8). */
  contextDraft: string | null;
  /** Workspaces restarted on the installed Crew, or started (AO-8). Absent: none. */
  restarted?: SetupRestarted[];
  /** Running workspaces left on an older Crew, with why. Absent: none. */
  outdated?: SetupOutdated[];
  /** Workspaces whose restart or start failed. Absent: none. */
  restartFailed?: SetupRestartFailed[];
}

export interface SetupRun {
  jobId: string;
  intent: SetupIntent;
  state: SetupState;
  /** The stage under way (or the last one, once the job ended). */
  stage: string;
  stages: SetupStage[];
  /** The server account the job reached, when the probe said. Display only. */
  server: { login: string | null; user: string | null } | null;
  /** The broker it installs, or would. */
  artifact: SetupArtifact | null;
  outcome: SetupOutcome | null;
  /** Once failed or cancelled: a typed code and the daemon's sentence. */
  error: { code: string; message: string; detail: string | null } | null;
  /** What the server printed, when the daemon kept it, control characters removed. */
  output: string;
  /**
   * The workspaces the job found on the server (AO-8); empty or absent before the workspaces
   * stage, and from a daemon before account ownership.
   */
  workspaces?: SetupWorkspace[];
  /** Why the server's workspaces could not be listed, when they could not. */
  workspacesNote?: string | null;
}

/** How often a running job is read again. */
export const SETUP_POLL_MS = 500;

const INTENTS: readonly string[] = ['check', 'install', 'host', 'member'] satisfies SetupIntent[];
const STATES: readonly string[] = [
  'running',
  'needs_sign_in',
  'finished',
  'failed',
  'cancelled',
] satisfies SetupState[];
const STAGE_STATES: readonly string[] = [
  'pending',
  'running',
  'done',
  'skipped',
  'failed',
] satisfies SetupStageState[];

/** A version and a digest become command text in the manual fallback: only these shapes pass. */
const VERSION = /^\d+\.\d+\.\d+$/;
const SHA256 = /^[0-9a-f]{64}$/;

function text(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function filled(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null;
}

/** The artifact as the daemon described it, or null for anything that is not one. */
export function setupArtifactFrom(wire: unknown): SetupArtifact | null {
  if (!isRecord(wire)) return null;
  const version = text(wire.version);
  const target = text(wire.target);
  const sha256 = text(wire.sha256)?.toLowerCase() ?? null;
  const size = wire.size;
  const source = filled(wire.source);
  if (!version || !VERSION.test(version) || !sha256 || !SHA256.test(sha256)) return null;
  if (typeof size !== 'number' || !Number.isSafeInteger(size) || size <= 0) return null;
  if (!target || !source) return null;
  return { version, target, sha256, size, source };
}

function stagesFrom(wire: unknown): SetupStage[] {
  if (wire === undefined || wire === null) return [];
  if (!Array.isArray(wire)) throw unexpectedCrewResponse('a setup');
  return wire.map((entry) => {
    const name = isRecord(entry) ? filled(entry.name) : null;
    const state = isRecord(entry) ? text(entry.state) : null;
    if (!name || !state || !STAGE_STATES.includes(state)) throw unexpectedCrewResponse('a setup');
    return { name, state: state as SetupStageState, detail: text(entry.detail) };
  });
}

const WORKSPACE_ROLES: readonly string[] = [
  'host',
  'member',
  'invited',
  'removed',
] satisfies SetupWorkspaceRole[];
const WORKSPACE_STATES: readonly string[] = [
  'running',
  'stopped',
  'deleted',
  'storage_failed',
  'unknown',
] satisfies SetupWorkspaceState[];
const OUTDATED_REASONS: readonly string[] = [
  'others_connected',
  'running_newer',
  'not_running_installed',
  'custom_state_dir',
] satisfies SetupOutdatedReason[];

/** A count from the wire, or null. */
function countOf(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

/** A version shaped as one, or null: it is shown and compared, never run. */
function versionOf(value: unknown): string | null {
  const version = filled(value);
  return version && /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]{1,32})?$/.test(version) ? version : null;
}

/** Records of a list, or nothing for anything that is not one. */
function recordsOf(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

/**
 * The workspaces a job found (AO-6, AO-8), entry by entry: one without an ID, a role or a state
 * this build knows is left out rather than refusing the rest.
 */
export function setupWorkspacesFrom(wire: unknown): SetupWorkspace[] {
  return recordsOf(wire).flatMap((entry): SetupWorkspace[] => {
    const workspaceId = filled(entry.workspace_id);
    const role = text(entry.role);
    const state = text(entry.state);
    if (!workspaceId || !role || !WORKSPACE_ROLES.includes(role)) return [];
    if (!state || !WORKSPACE_STATES.includes(state)) return [];
    const invitation = filled(entry.invitation);
    const here = entry.this_computer;
    return [
      {
        workspaceId,
        name: filled(entry.name),
        role: role as SetupWorkspaceRole,
        state: state as SetupWorkspaceState,
        hostUsername: filled(entry.host_username),
        fingerprint: filled(entry.workspace_key_fingerprint),
        mode: entry.mode === 'private' || entry.mode === 'public' ? entry.mode : null,
        institutionId: filled(entry.institution_id),
        version: versionOf(entry.version),
        binaryCurrent: typeof entry.binary_current === 'boolean' ? entry.binary_current : null,
        othersConnected: countOf(entry.others_connected),
        invitation: invitation?.startsWith('brcrew1:') ? invitation : null,
        savedConnectionId: filled(entry.saved_connection_id),
        thisComputer: here === 'connected' || here === 'saved' ? here : 'not_saved',
        error: filled(entry.error),
      },
    ];
  });
}

function outcomeFrom(wire: unknown): SetupOutcome | null {
  if (!isRecord(wire)) return null;
  const replaced = isRecord(wire.upgraded_from) ? text(wire.upgraded_from.version) : null;
  return {
    restarted: recordsOf(wire.restarted).flatMap((entry) => {
      const workspaceId = filled(entry.workspace_id);
      return workspaceId
        ? [
            {
              workspaceId,
              name: filled(entry.name),
              fromVersion: versionOf(entry.from_version),
              toVersion: versionOf(entry.to_version),
            },
          ]
        : [];
    }),
    outdated: recordsOf(wire.outdated).flatMap((entry) => {
      const workspaceId = filled(entry.workspace_id);
      const reason = text(entry.reason);
      return workspaceId && reason && OUTDATED_REASONS.includes(reason)
        ? [
            {
              workspaceId,
              name: filled(entry.name),
              runningVersion: versionOf(entry.running_version),
              installedVersion: versionOf(entry.installed_version),
              othersConnected: countOf(entry.others_connected),
              reason: reason as SetupOutdatedReason,
            },
          ]
        : [];
    }),
    restartFailed: recordsOf(wire.restart_failed).flatMap((entry) => {
      const workspaceId = filled(entry.workspace_id);
      const code = filled(entry.code);
      const message = filled(entry.message);
      return workspaceId && code && message
        ? [{ workspaceId, name: filled(entry.name), code, message, command: filled(entry.command) }]
        : [];
    }),
    installedVersion: filled(wire.installed_version),
    reused: wire.reused === true,
    previousKept: wire.previous_kept === true,
    replacedVersion: replaced && VERSION.test(replaced) ? replaced : null,
    host: hostStartResultFrom(wire.host),
    hostCommand: text(wire.host_command),
    connectionId: filled(wire.connection_id),
    workFolder: filled(wire.work_folder),
    remoteAccess:
      wire.remote_access === 'full' || wire.remote_access === 'folder' ? wire.remote_access : null,
    contextDraft: filled(wire.context_draft),
  };
}

/**
 * A job as the daemon answered it, or a refusal: a body that is not a job is never read as one. An
 * older daemon that answers a route it does not have with a web page is reported as outdated.
 */
export function setupRunFrom(wire: unknown): SetupRun {
  if (!isRecord(wire)) throw outdatedDaemonResponse();
  const jobId = filled(wire.job_id);
  const intent = text(wire.intent);
  const state = text(wire.state);
  const stage = filled(wire.stage);
  if (!jobId || !intent || !INTENTS.includes(intent) || !state || !STATES.includes(state)) {
    throw unexpectedCrewResponse('a setup');
  }
  if (!stage) throw unexpectedCrewResponse('a setup');
  const failure = isRecord(wire.error) ? wire.error : null;
  const code = failure ? filled(failure.code) : null;
  const message = failure ? filled(failure.message) : null;
  const server = isRecord(wire.server)
    ? { login: filled(wire.server.login), user: filled(wire.server.user) }
    : null;
  return {
    jobId,
    intent: intent as SetupIntent,
    state: state as SetupState,
    stage,
    stages: stagesFrom(wire.stages),
    server,
    artifact: setupArtifactFrom(wire.artifact),
    outcome: outcomeFrom(wire.outcome),
    error:
      code && message ? { code, message, detail: failure ? text(failure.detail) : null } : null,
    output: text(wire.output) ?? '',
    workspaces: setupWorkspacesFrom(wire.workspaces),
    workspacesNote: filled(wire.workspaces_note),
  };
}

/**
 * Every setup job this computer's daemon knows, newest first (`GET /crew/setup`): Crew shows one a
 * chat or a terminal started, with its Sign in button (AO-8). A job that does not read as one is
 * left out; anything that is not a list reads as none.
 */
export async function listSetupJobs(signal?: AbortSignal): Promise<SetupRun[]> {
  const wire = await crewHttp<unknown>('/setup', 'GET', undefined, signal);
  if (!Array.isArray(wire)) return [];
  return wire.flatMap((entry) => {
    try {
      return [setupRunFrom(entry)];
    } catch {
      return [];
    }
  });
}

/** The broker this build would install. Refused with `crew_install_artifact_unavailable` when none. */
export async function readSetupArtifact(signal?: AbortSignal): Promise<SetupArtifact> {
  const wire = await crewHttp<unknown>('/setup/artifact', 'GET', undefined, signal);
  if (!isRecord(wire)) throw outdatedDaemonResponse();
  const artifact = setupArtifactFrom(wire);
  if (!artifact) throw unexpectedCrewResponse('a server program');
  return artifact;
}

function targetBody(target: SetupTarget): SetupTarget {
  if (target.kind === 'connection') {
    return { kind: 'connection', connection_id: target.connection_id };
  }
  // The route refuses a field it does not take, so an absent choice is left out, never null.
  const route = {
    ssh_target: target.ssh_target,
    ...(target.port ? { port: target.port } : {}),
    ...(target.identity_file ? { identity_file: target.identity_file } : {}),
    ...(target.proxy_jump ? { proxy_jump: target.proxy_jump } : {}),
  };
  if (target.kind === 'server') {
    return {
      kind: 'server',
      ...route,
      ...(target.preparation_id ? { preparation_id: target.preparation_id } : {}),
    };
  }
  return { kind: 'pending', preparation_id: target.preparation_id, ...route };
}

/**
 * The jobs this window started, by ID. Their own surface shows their steps and opens their Sign in
 * window, so the notice for jobs started elsewhere (a chat, a terminal) leaves them out. Memory
 * only.
 */
const startedHere = new Set<string>();

/** Whether this window started the job (`startSetup`). */
export function setupStartedHere(jobId: string): boolean {
  return startedHere.has(jobId);
}

export function resetSetupStartedHereForTests(): void {
  startedHere.clear();
}

/** Start a setup job. Answers the job already under way for the same target, if one is. */
export async function startSetup(input: SetupInput): Promise<SetupRun> {
  const body = {
    target: targetBody(input.target),
    intent: input.intent,
    ...(input.workspace_name ? { workspace_name: input.workspace_name } : {}),
    ...(input.replace_existing ? { replace_existing: true } : {}),
    ...(input.agent_access ? { agent_access: input.agent_access } : {}),
    ...(input.fix_storage_permissions ? { fix_storage_permissions: true } : {}),
    ...(typeof input.restart_workspaces === 'boolean'
      ? { restart_workspaces: input.restart_workspaces }
      : {}),
    ...(input.start_workspaces?.length ? { start_workspaces: input.start_workspaces } : {}),
  };
  const status = setupRunFrom(await crewHttp<unknown>('/setup', 'POST', body));
  startedHere.add(status.jobId);
  return status;
}

/** Where a job stands. */
export async function readSetup(jobId: string, signal?: AbortSignal): Promise<SetupRun> {
  return setupRunFrom(
    await crewHttp<unknown>(`/setup/${encodeURIComponent(jobId)}`, 'GET', undefined, signal)
  );
}

/** Stop a job. Stopping one that already ended changes nothing. */
export async function stopSetup(jobId: string): Promise<void> {
  await crewHttp<unknown>(`/setup/${encodeURIComponent(jobId)}`, 'DELETE');
}

/** Whether a refusal says the daemon predates automatic setup, so the older path is the way on. */
export function setupUnsupported(failure: unknown): boolean {
  return isStaleDaemon(failure);
}

/** The code of a refused start, when the daemon gave one. */
export function setupRefusalCode(failure: unknown): string | null {
  return failure instanceof CrewHttpError ? (failure.code ?? null) : null;
}

/** Whether a job is still the daemon's to finish. */
export function setupActive(run: SetupRun | null): boolean {
  return run !== null && (run.state === 'running' || run.state === 'needs_sign_in');
}

/**
 * The steps a person sees, in plain words (`setupCopy.stages`): the daemon's stages folded into
 * five. A host starts a workspace; a member connects.
 */
export type PlainSetupStage =
  | 'checking'
  | 'signing-in'
  | 'installing'
  /** Looking for the account's workspaces on the server (AO-8's `workspaces` stage). */
  | 'finding'
  /** Restarting running workspaces on the installed Crew (AO-8's `restart` stage). */
  | 'updating'
  | 'starting'
  | 'connecting'
  | 'ready';

/**
 * The daemon's stages (`SetupStageName`), and the finer names a daemon may report for the same
 * steps. The work folder and the context draft come at the end of a job, so they read as its last
 * step before Ready (`plainStage`).
 */
const PLAIN_STAGE: Readonly<Record<string, PlainSetupStage>> = {
  check: 'checking',
  preflight: 'checking',
  probe: 'checking',
  check_installation: 'checking',
  sign_in: 'signing-in',
  install: 'installing',
  prepare_artifact: 'installing',
  transfer: 'installing',
  verify_installed: 'installing',
  workspaces: 'finding',
  restart: 'updating',
  start: 'starting',
  connect: 'connecting',
  done: 'ready',
};
const LAST_STEPS = new Set(['work_folder', 'context', 'save', 'join', 'configure']);

/** The plain steps a job of `intent` goes through, in order. */
export function plainStages(intent: SetupIntent): PlainSetupStage[] {
  if (intent === 'host') return ['checking', 'signing-in', 'installing', 'starting', 'ready'];
  if (intent === 'member') return ['checking', 'signing-in', 'installing', 'connecting', 'ready'];
  // A check installs nothing: it signs in and looks for the account's workspaces (AO-8).
  if (intent === 'check') return ['checking', 'signing-in', 'finding', 'ready'];
  return ['checking', 'signing-in', 'installing', 'finding', 'updating', 'ready'];
}

/**
 * The plain steps of `run`: a host's or member's job adds Looking for workspaces and Updating
 * running workspaces when its daemon reports those stages (AO-8), and every job leaves out one of
 * them that it skipped, so a step that did nothing never reads as done.
 */
export function plainStagesFor(intent: SetupIntent, run: SetupRun | null): PlainSetupStage[] {
  const reported = (name: string) => run?.stages.find((stage) => stage.name === name);
  const shown = (name: string) => {
    const stage = reported(name);
    if (intent === 'check' || intent === 'install') return stage?.state !== 'skipped';
    return stage !== undefined && stage.state !== 'skipped';
  };
  const stages = plainStages(intent).filter(
    (stage) =>
      (stage !== 'finding' || shown('workspaces')) && (stage !== 'updating' || shown('restart'))
  );
  if (intent !== 'host' && intent !== 'member') return stages;
  const extra: PlainSetupStage[] = [
    ...(shown('workspaces') ? (['finding'] as const) : []),
    ...(shown('restart') ? (['updating'] as const) : []),
  ];
  const after = stages.indexOf('installing') + 1;
  return [...stages.slice(0, after), ...extra, ...stages.slice(after)];
}

/** Where a job stands in plain steps. A stage name this build does not know reads as checking. */
export function plainStage(run: SetupRun): PlainSetupStage {
  if (run.state === 'needs_sign_in') return 'signing-in';
  if (run.state === 'finished') return 'ready';
  if (LAST_STEPS.has(run.stage)) {
    if (run.intent === 'host') return 'starting';
    if (run.intent === 'member') return 'connecting';
    return 'installing';
  }
  return PLAIN_STAGE[run.stage] ?? 'checking';
}

/**
 * Whether version `a` is older than `b`, comparing major, minor and patch. Anything that is not a
 * version is not older: Crew never offers an update on a version it cannot read.
 */
export function versionOlder(a: string | null | undefined, b: string | null | undefined): boolean {
  const parts = (value: string | null | undefined) =>
    value && /^\d+\.\d+\.\d+/.test(value)
      ? value
          .split(/[-+]/)[0]
          .split('.')
          .map((part) => Number(part))
      : null;
  const left = parts(a);
  const right = parts(b);
  if (!left || !right) return false;
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] < right[index];
  }
  return false;
}

/** The file name the manual fallback saves the broker under, and copies to the server. */
export function brokerFileName(version: string): string {
  return `biorouter-crew-${version}-linux-x86_64`;
}

function shellWord(value: string): string {
  return /^[A-Za-z0-9@%+=:,./_~-]+$/.test(value) ? value : `'${value.replace(/'/g, `'\\''`)}'`;
}

/**
 * "Run it yourself": the commands that install the broker this build carries by hand, generated
 * from the artifact so the version and digest are the ones the daemon would send. The first line
 * runs on this computer (the file "Save Crew's server program…" wrote, `~/Downloads` until then);
 * the rest run on the server, in the account Crew is for. Never a placeholder path.
 */
export function manualInstallCommands(
  artifact: SetupArtifact,
  route: {
    ssh_target: string;
    port?: number | null;
    proxy_jump?: string | null;
    identity_file?: string | null;
  },
  savedPath: string | null = null
): string {
  const file = brokerFileName(artifact.version);
  const local = savedPath ? shellWord(savedPath) : `~/Downloads/${file}`;
  const scp = ['scp'];
  if (route.port && route.port !== 22) scp.push('-P', String(route.port));
  if (route.proxy_jump?.trim()) scp.push('-J', shellWord(route.proxy_jump.trim()));
  if (route.identity_file?.trim()) scp.push('-i', shellWord(route.identity_file.trim()));
  const login = route.ssh_target.trim() || 'you@server';
  scp.push(local, `${shellWord(login)}:${file}`);
  return [
    scp.join(' '),
    `echo '${artifact.sha256}  ${file}' | sha256sum -c -`,
    `mkdir -p "$HOME/.local/bin" && install -m 0755 ${file} "$HOME/.local/bin/biorouter-crew" && "$HOME/.local/bin/biorouter-crew" --version`,
  ].join('\n');
}

/**
 * Join asked for a member setup if connecting finds Crew missing for the account (OW-9): the Not
 * set up pane starts it by itself, once. In memory only, and only for a while: pressing Join is
 * the consent, so a request never outlives this app session, and it lapses after the time a
 * sign-in may take (the daemon waits as long for one). It carries the level the person chose in
 * Join (C18), so the job sets exactly that one: Off stays off.
 */
export const MEMBER_SETUP_REQUEST_MS = 10 * 60 * 1000;

/** What Join asked of the member setup the Not set up pane starts for it. */
export interface MemberSetupRequest {
  /**
   * The agent access the person chose in Join, sent as the job's `agent_access`; null for a
   * connection that was already on this computer, whose access the job leaves as it is.
   */
  agentAccess: SetupAgentAccess | null;
}

const memberSetupRequests = new Map<string, { at: number; request: MemberSetupRequest }>();

export function requestMemberSetup(
  connectionId: string,
  request: MemberSetupRequest,
  now: number = Date.now()
): void {
  if (connectionId) memberSetupRequests.set(connectionId, { at: now, request });
}

/** Take the request for `connectionId`: the request once, while it stands, then null. */
export function takeMemberSetupRequest(
  connectionId: string,
  now: number = Date.now()
): MemberSetupRequest | null {
  const entry = memberSetupRequests.get(connectionId);
  memberSetupRequests.delete(connectionId);
  return entry !== undefined && now - entry.at <= MEMBER_SETUP_REQUEST_MS ? entry.request : null;
}

export function resetMemberSetupRequestsForTests(): void {
  memberSetupRequests.clear();
}
