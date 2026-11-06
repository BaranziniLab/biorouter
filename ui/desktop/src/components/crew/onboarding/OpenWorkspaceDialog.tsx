import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Input } from '../../ui/input';
import { Note } from '../../ui/note';
import { saveFromInvitation } from '../api/join';
import { connectionServer, sanitizeDisplayText } from '../identity';
import { useCrew, useCrewErrorSlot } from '../state/CrewControllerContext';
import { canFocus, focusIsLost } from '../state/focusReturn';
import type { OpenWorkspaceRoute } from '../state/types';
import { joinCopy, openCopy } from './copy';
import {
  Field,
  FieldErrorsProvider,
  useFormValidation,
  useInitialFocus,
  useMounted,
  useOpenGeneration,
} from './fields';
import { updateJoinContext } from './joinContext';
import { advancedInvalid, SSH_LOGIN_PATTERN } from './JoinDialog';
import { sshUsername } from './joinText';
import {
  setupWorkspaceKey,
  versionOlder,
  type SetupIntent,
  type SetupOutdatedReason,
  type SetupRun,
  type SetupWorkspace,
} from './setup';
import { SetupProgress, useSetupArtifact, useSetupJob } from './SetupProgress';
import { foundWorkspaceTitle, SetupWorkspaceList, workspacesNoteText } from './SetupWorkspaces';

const SOURCE = 'dialog:open-workspace' as const;

/** Whether `entry` can be opened on this computer: the account is in it and it is running. */
export function canOpenWorkspace(entry: SetupWorkspace): boolean {
  return (
    entry.role !== 'removed' &&
    entry.state === 'running' &&
    (entry.invitation !== null || entry.savedConnectionId !== null)
  );
}

/** A workspace the account hosts that is stopped: Start runs it (AO-7, AO-8). */
export function canStartWorkspace(entry: SetupWorkspace): boolean {
  return entry.role === 'host' && entry.state === 'stopped';
}

/**
 * A running workspace the account hosts whose Crew is older than this computer's (AO-8): the job
 * listed it as outdated, it runs another program than the one installed, it names no version (a
 * broker from before AO-4, which counts as older), or a version below the one this Biorouter
 * carries.
 */
export function workspaceOutdated(
  entry: SetupWorkspace,
  available: string | null,
  listedOutdated = false
): boolean {
  if (entry.role !== 'host' || entry.state !== 'running') return false;
  return (
    listedOutdated ||
    entry.binaryCurrent === false ||
    entry.version === null ||
    versionOlder(entry.version, available)
  );
}

/**
 * Why a restart was left that Update Crew cannot change (AO-8): a workspace running a newer Crew
 * than the one installed, or one keeping its state where Crew does not restart it. Such a row is
 * told why and is not offered Update Crew again.
 */
const LEFT_FOR_GOOD: readonly SetupOutdatedReason[] = ['running_newer', 'custom_state_dir'];

/** Two hosts have a workspace of one name here (AO-6): the dialog says so, and names each host. */
function sharedNames(workspaces: readonly SetupWorkspace[]): string[] {
  const hosts = new Map<string, Set<string>>();
  for (const entry of workspaces) {
    const name = sanitizeDisplayText(entry.name);
    if (!name) continue;
    const host = entry.role === 'host' ? '' : (entry.hostUsername ?? '?');
    hosts.set(name, (hosts.get(name) ?? new Set()).add(host));
  }
  return [...hosts.entries()].filter(([, set]) => set.size > 1).map(([name]) => name);
}

export interface OpenWorkspaceDialogProps {
  /** Defaults to the controller's dialog intent (`{kind: 'open-workspace'}`). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/**
 * Open a workspace on a server (account ownership, "Desktop"; AO-6, AO-8, AO-9).
 *
 * The person types the SSH login they sign in to the server with (and, under Advanced, its port,
 * key and jump host). Look for workspaces prepares a device on this computer and runs a check job
 * on the server with it, so looking and opening need one sign-in; a server that asks for a password
 * or code opens the job's Sign in window, and a host key Crew can't verify is said as Join's trust
 * pane says it (the job's own steps, `SetupProgress`). The workspaces found are listed with their
 * host named, the account's role and where each stands, and offer:
 *
 * - **Open**: the ordinary join. The connection is saved from the entry's own invitation line with
 *   the login the person typed (or the one already saved is used), connected, and the join screen
 *   joins by account at once, because the person asked for it here.
 * - **Start**: a stopped workspace the account hosts, started on the installed Crew.
 * - **Update Crew**: a running workspace the account hosts that runs an older Crew. With other
 *   people connected, or a count the server did not give (a Crew from before account ownership),
 *   it asks first: their running agent commands stop when it restarts. Either way it restarts
 *   that workspace and no other (`restart_workspace_ids` with `restart_workspaces: false`).
 *
 * Rows are told apart by workspace ID, host UID and key fingerprint, because two hosts can list
 * one ID (AO-6). When a job ends, focus that a disabled button dropped goes to the row's first
 * action, else to the first one found, else to Look again.
 */
export function OpenWorkspaceDialog({ open, onOpenChange }: OpenWorkspaceDialogProps) {
  const crew = useCrew();
  const intent = crew.ui.dialog?.kind === 'open-workspace' ? crew.ui.dialog : null;
  const isOpen = open ?? intent !== null;
  const generation = useOpenGeneration(isOpen);
  const close = () => {
    if (onOpenChange) onOpenChange(false);
    else crew.closeDialog();
  };
  return (
    <OpenWorkspaceView
      key={generation}
      open={isOpen}
      route={intent?.route ?? null}
      checkAtOnce={intent?.check === true}
      onClose={close}
    />
  );
}

function OpenWorkspaceView({
  open,
  route: given,
  checkAtOnce,
  onClose,
}: {
  open: boolean;
  route: OpenWorkspaceRoute | null;
  checkAtOnce: boolean;
  onClose: () => void;
}) {
  const crew = useCrew();
  const mounted = useMounted();
  const formId = useId();
  const loginRef = useRef<HTMLInputElement>(null);
  const { errors, validate, formProps } = useFormValidation();
  const [login, setLogin] = useState(given?.ssh_target ?? '');
  const [port, setPort] = useState(given?.port ? String(given.port) : '');
  const [identityFile, setIdentityFile] = useState(given?.identity_file ?? '');
  const [proxyJump, setProxyJump] = useState(given?.proxy_jump ?? '');
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [preparing, setPreparing] = useState(false);
  /** The route and device the last look used: every action on its list uses the same. */
  const [looked, setLooked] = useState<{
    route: OpenWorkspaceRoute;
    preparationId: string;
  } | null>(null);
  const [jobIntent, setJobIntent] = useState<SetupIntent>('check');
  /** The last finished job: its list of workspaces and what it restarted. */
  const [found, setFound] = useState<SetupRun | null>(null);
  /**
   * Whether `found` came from a job that asked to restart or start nothing (a look, or updating
   * the Crew in the account): it says what Update Crew would do, never why a restart was left.
   */
  const [foundLookOnly, setFoundLookOnly] = useState(true);
  const jobLookOnly = useRef(true);
  /** The row (`setupWorkspaceKey`) whose Update Crew is asking first. */
  const [confirmUpdate, setConfirmUpdate] = useState<string | null>(null);
  /** The row being opened. */
  const [opening, setOpening] = useState<string | null>(null);
  const [pendingConnect, setPendingConnect] = useState<string | null>(null);
  const [daemonOutdated, setDaemonOutdated] = useState(false);
  const artifact = useSetupArtifact(open).artifact;
  const showError = useCrewErrorSlot(SOURCE);
  const submitRef = useRef<HTMLButtonElement>(null);
  const foundRef = useRef<HTMLDivElement>(null);
  /**
   * Where focus goes once the running job ends: the row it acted on (its key), the list (`''`, a
   * look), or nowhere (`undefined`). Every control is disabled while a job runs, so the button
   * that started it loses focus and the dialog's frame takes it.
   */
  const focusAfterJob = useRef<string | undefined>(undefined);
  useInitialFocus(loginRef, open && !checkAtOnce);

  const server =
    connectionServer({ id: '', ssh_target: (looked?.route.ssh_target ?? login).trim() }) ||
    login.trim();
  const job = useSetupJob({
    server,
    onFinished: (run) => {
      setFound(run);
      setFoundLookOnly(jobLookOnly.current);
    },
  });
  const busy = preparing || job.busy || opening !== null;

  const routeNow = (): OpenWorkspaceRoute => ({
    ssh_target: login.trim(),
    ...(port.trim() ? { port: Number(port) } : {}),
    ...(identityFile.trim() ? { identity_file: identityFile.trim() } : {}),
    ...(proxyJump.trim() ? { proxy_jump: proxyJump.trim() } : {}),
  });

  /** Run a job on the server the last look used, with the device it prepared. */
  const runOnServer = async (
    intent: SetupIntent,
    extra: {
      restart_workspaces?: boolean;
      restart_workspace_ids?: string[];
      start_workspaces?: string[];
    } = {},
    on: { route: OpenWorkspaceRoute; preparationId: string } | null = looked,
    row = ''
  ) => {
    if (!on) return;
    setJobIntent(intent);
    setConfirmUpdate(null);
    focusAfterJob.current = row;
    jobLookOnly.current = !extra.restart_workspace_ids?.length && !extra.start_workspaces?.length;
    const result = await job.start({
      target: { kind: 'server', ...on.route, preparation_id: on.preparationId },
      intent,
      ...extra,
    });
    if (result === 'outdated' && mounted.current) setDaemonOutdated(true);
  };

  const look = async () => {
    if (busy) return;
    if (!advancedOpen && advancedInvalid(port, '')) {
      setAdvancedOpen(true);
      return;
    }
    if (!validate()) return;
    setPreparing(true);
    const device = await crew.act(SOURCE, 'device.prepare', () => crew.prepareHostingDevice());
    if (!mounted.current) return;
    setPreparing(false);
    if (!device) return;
    const next = { route: routeNow(), preparationId: device.preparation_id };
    setLooked(next);
    setFound(null);
    setDaemonOutdated(false);
    await runOnServer('check', {}, next);
  };

  // Opened by a surface that knows the login (Update Crew, a name in use): look at once.
  const lookedAtOnce = useRef(false);
  useEffect(() => {
    if (!open || !checkAtOnce || lookedAtOnce.current || !login.trim()) return;
    lookedAtOnce.current = true;
    void look();
    // `look` is recreated every render; this runs once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, checkAtOnce]);

  // Connect only once the controller has selected the saved connection and lists it: its
  // `connect` is bound to the selection of the render it came from (as Join does).
  useEffect(() => {
    if (!pendingConnect || crew.connectionId !== pendingConnect) return;
    if (!crew.connections.some((item) => item.id === pendingConnect)) return;
    setPendingConnect(null);
    if (crew.connection?.status === 'connected') {
      void crew.refresh().catch(() => undefined);
      onClose();
      return;
    }
    void crew.connect({ userInitiated: true }).then(() => {
      if (mounted.current) onClose();
    });
  }, [pendingConnect, crew, mounted, onClose]);

  // Once a job ends and the rows are enabled again, put focus that the job's disabled controls
  // dropped back on something to act with (live QA: it fell to the dialog's frame).
  useEffect(() => {
    const row = focusAfterJob.current;
    if (row === undefined || !found || busy) return;
    focusAfterJob.current = undefined;
    const active = typeof document === 'undefined' ? null : document.activeElement;
    const dropped =
      focusIsLost() ||
      active === submitRef.current ||
      (active instanceof HTMLButtonElement && active.disabled);
    if (!dropped) return;
    const list = foundRef.current;
    const rowButton = row
      ? Array.from(list?.querySelectorAll<HTMLElement>('[data-crew-found-row]') ?? [])
          .find((item) => item.dataset.crewFoundRow === row)
          ?.querySelector<HTMLElement>('button')
      : null;
    const target = [rowButton, list?.querySelector<HTMLElement>('button'), submitRef.current].find(
      canFocus
    );
    target?.focus();
  }, [found, busy]);

  const openWorkspace = async (entry: SetupWorkspace) => {
    if (busy || !looked) return;
    setOpening(setupWorkspaceKey(entry));
    const username = sshUsername(looked.route.ssh_target);
    const saved = await crew.act(SOURCE, 'connection.save', async () => {
      let id = entry.savedConnectionId;
      if (!id) {
        if (!entry.invitation) throw new Error(openCopy.cantOpen);
        const connection = await saveFromInvitation(entry.invitation, {
          ...(username ? { username } : {}),
          ...(entry.mode ? { mode: entry.mode } : {}),
          ...(entry.institutionId ? { institution_id: entry.institutionId } : {}),
          advanced: {
            ssh_target: looked.route.ssh_target,
            ...(looked.route.port ? { port: looked.route.port } : {}),
            ...(looked.route.identity_file ? { identity_file: looked.route.identity_file } : {}),
            ...(looked.route.proxy_jump ? { proxy_jump: looked.route.proxy_jump } : {}),
            preparation_id: looked.preparationId,
          },
        });
        id = connection.id;
      }
      // Reload the list before selecting, so the controller knows the connection it connects.
      await crew.refresh();
      crew.selectConnection(id);
      return id;
    });
    if (!mounted.current) return;
    if (!saved) {
      setOpening(null);
      return;
    }
    // The person asked for this workspace here: the join screen joins by account at once (AO-9).
    updateJoinContext(saved, {
      workspaceName: entry.name,
      hostUsername: entry.role === 'host' ? username : entry.hostUsername,
      username,
      joining: true,
    });
    setPendingConnect(saved);
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void look();
  };

  const workspaces = found?.workspaces ?? [];
  const outcome = found?.outcome ?? null;
  const available = artifact?.version ?? null;
  const shared = sharedNames(workspaces);

  // The job lists the workspaces before its restart stage, so one it just restarted or started
  // still reads as it was: what the outcome says wins. The outcome names workspaces the account
  // hosts by ID only, so it is read for the hosted row alone, never for another host's row that
  // lists the same ID.
  const hosted = (entry: SetupWorkspace) => entry.role === 'host';
  const listedOutdated = (entry: SetupWorkspace) =>
    hosted(entry)
      ? (outcome?.outdated?.find((item) => item.workspaceId === entry.workspaceId) ?? null)
      : null;
  const restartedNow = (entry: SetupWorkspace) =>
    hosted(entry) &&
    outcome?.restarted?.some((item) => item.workspaceId === entry.workspaceId) === true;
  const outdated = (entry: SetupWorkspace) =>
    !restartedNow(entry) && workspaceOutdated(entry, available, listedOutdated(entry) !== null);
  /** Update Crew can still help: not left running for a reason no restart changes. */
  const updatable = (entry: SetupWorkspace) => {
    const reason = listedOutdated(entry)?.reason;
    return outdated(entry) && !(reason && LEFT_FOR_GOOD.includes(reason));
  };
  /**
   * How many other people are connected: `null` when the server did not say, as a Crew from
   * before account ownership never does. Unknown is not nobody.
   */
  const othersOf = (entry: SetupWorkspace): number | null =>
    entry.othersConnected ?? listedOutdated(entry)?.othersConnected ?? null;
  /** Update this workspace and no other: its ID, and no consent for the rest. */
  const update = (entry: SetupWorkspace) =>
    void runOnServer(
      'install',
      { restart_workspaces: false, restart_workspace_ids: [entry.workspaceId] },
      looked,
      setupWorkspaceKey(entry)
    );

  /**
   * Why the last job listed no workspaces, and whether installing this computer's Crew in the
   * account answers it: a Crew there too old to list them, or none (live QA: an older Crew in the
   * account was a dead end). The install restarts no workspace (`restart_workspaces: false`), and
   * the same job then looks for the workspaces again with the Crew it installed.
   */
  const unlisted = found?.workspacesUnlisted ?? null;
  const installs = available !== null && (unlisted === 'too_old' || unlisted === 'not_installed');
  const unlistedText = !found
    ? null
    : installs && unlisted === 'too_old'
      ? openCopy.tooOld(found.installedVersion ?? null, server)
      : installs
        ? openCopy.notInstalled(server)
        : workspacesNoteText(found.workspacesNote ?? null);
  const installInAccount = () =>
    void runOnServer('install', { restart_workspaces: false }, looked, '');

  const actions = (entry: SetupWorkspace) => {
    const name = foundWorkspaceTitle(entry);
    const key = setupWorkspaceKey(entry);
    const buttons = [];
    if (updatable(entry)) {
      buttons.push(
        <Button
          key="update"
          type="button"
          variant="secondary"
          size="sm"
          disabled={busy}
          aria-label={openCopy.updateLabel(sanitizeDisplayText(entry.name) || name)}
          onClick={() => {
            if (othersOf(entry) === 0) update(entry);
            else setConfirmUpdate(key);
          }}
        >
          {openCopy.update}
        </Button>
      );
    }
    if (canStartWorkspace(entry) && !restartedNow(entry)) {
      buttons.push(
        <Button
          key="start"
          type="button"
          variant="secondary"
          size="sm"
          disabled={busy}
          aria-label={openCopy.startLabel(sanitizeDisplayText(entry.name) || name)}
          onClick={() =>
            // Start this workspace, and restart no other on the way.
            void runOnServer(
              'install',
              { restart_workspaces: false, start_workspaces: [entry.workspaceId] },
              looked,
              key
            )
          }
        >
          {openCopy.start}
        </Button>
      );
    }
    if (canOpenWorkspace(entry)) {
      buttons.push(
        <Button
          key="open"
          type="button"
          size="sm"
          disabled={busy}
          aria-label={openCopy.openLabel(name)}
          onClick={() => void openWorkspace(entry)}
        >
          {opening === key ? openCopy.opening : openCopy.open}
        </Button>
      );
    }
    return buttons.length ? <>{buttons}</> : null;
  };

  const note = (entry: SetupWorkspace) => {
    const name = sanitizeDisplayText(entry.name) || openCopy.unnamed;
    const restarted = hosted(entry)
      ? outcome?.restarted?.find((item) => item.workspaceId === entry.workspaceId)
      : undefined;
    const failed = hosted(entry)
      ? outcome?.restartFailed?.find((item) => item.workspaceId === entry.workspaceId)
      : undefined;
    const left = outdated(entry) ? listedOutdated(entry) : null;
    const lines = [];
    if (confirmUpdate === setupWorkspaceKey(entry)) {
      lines.push(
        <Note
          key="confirm"
          tone="warning"
          role="alert"
          testId="crew-open-update-confirm"
          action={
            <div className="crew-onboard-row">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => setConfirmUpdate(null)}
              >
                {openCopy.keepRunning}
              </Button>
              <Button type="button" size="sm" onClick={() => update(entry)}>
                {openCopy.updateAnyway}
              </Button>
            </div>
          }
        >
          {openCopy.othersConnected(othersOf(entry))}
        </Note>
      );
    } else if (outdated(entry)) {
      // A workspace left for good is not "older" in any way Update Crew can change: its reason
      // alone is said.
      if (updatable(entry))
        lines.push(
          <p key="outdated" className="text-supporting text-text-muted">
            {openCopy.outdated(entry.version, available)}
          </p>
        );
      // Why the last update left it, when a reason says more than "older" does. After a look,
      // which asked to restart nothing, what Update Crew would do instead (it was never tried).
      const reason =
        left?.reason === 'others_connected'
          ? foundLookOnly
            ? openCopy.restartsOnUpdate(othersOf(entry))
            : openCopy.leftRunning.others_connected(othersOf(entry))
          : left?.reason === 'running_newer'
            ? openCopy.leftRunning.running_newer(left.runningVersion ?? entry.version)
            : left?.reason === 'custom_state_dir'
              ? openCopy.leftRunning.custom_state_dir(server)
              : null;
      if (reason)
        lines.push(
          <p key="left" className="text-supporting text-text-muted" role="status">
            {reason}
          </p>
        );
    }
    if (restarted) {
      lines.push(
        <p key="restarted" role="status" className="text-supporting text-text-success">
          {entry.state === 'stopped'
            ? openCopy.started(name, restarted.toVersion)
            : openCopy.restarted(name, restarted.toVersion)}
        </p>
      );
    }
    if (failed) {
      lines.push(
        <div key="failed" className="crew-onboard-stack">
          <p className="text-supporting text-text-danger">
            {openCopy.restartFailed(name, sanitizeDisplayText(failed.message))}
          </p>
          {failed.command ? (
            <CopyField value={failed.command} label={openCopy.restartCommand} />
          ) : null}
        </div>
      );
    }
    if (
      entry.role !== 'removed' &&
      entry.state === 'running' &&
      !entry.invitation &&
      !entry.savedConnectionId
    ) {
      lines.push(
        <p key="cant" className="text-supporting text-text-muted">
          {openCopy.cantOpen}
        </p>
      );
    }
    return lines.length ? <>{lines}</> : null;
  };

  const lookLabel =
    preparing || job.busy ? openCopy.looking : found ? openCopy.lookAgain : openCopy.look;

  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => {
        if (!next && !busy) onClose();
      }}
      size="lg"
      purpose="form"
      title={openCopy.title}
      scrollBody
      className="crew-dialog"
      anchor="top"
      footer={
        <>
          <Button type="button" variant="secondary" disabled={opening !== null} onClick={onClose}>
            {found ? openCopy.close : openCopy.cancel}
          </Button>
          <Button ref={submitRef} type="submit" form={formId} disabled={busy}>
            {lookLabel}
          </Button>
        </>
      }
    >
      <FieldErrorsProvider value={errors}>
        <form
          id={formId}
          {...formProps}
          className="crew-onboard-form crew-onboard-dialog-form"
          onSubmit={submit}
          aria-busy={busy}
          data-testid="crew-open-workspace"
        >
          <p className="text-body text-text-muted">{openCopy.intro}</p>
          <Field
            label={openCopy.login}
            helper={openCopy.loginHelper}
            invalidMessage={openCopy.loginInvalid}
          >
            {(props) => (
              <Input
                {...props}
                ref={loginRef}
                required
                pattern={SSH_LOGIN_PATTERN}
                disabled={busy}
                placeholder={openCopy.loginPlaceholder}
                value={login}
                spellCheck={false}
                autoComplete="off"
                onChange={(event) => setLogin(event.target.value)}
              />
            )}
          </Field>
          <Disclosure open={advancedOpen} onOpenChange={setAdvancedOpen} label={openCopy.advanced}>
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
              <Field label={joinCopy.jumpHost}>
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
            </div>
          </Disclosure>
          {daemonOutdated ? (
            <Note tone="warning" role="alert">
              {openCopy.outdatedDaemon}
            </Note>
          ) : null}
          <SetupProgress
            job={job}
            intent={jobIntent}
            server={server}
            showWorkspaces={false}
            testId="crew-open-progress"
          />
          {/* Said here, not by the steps, so the way on can sit beside it. */}
          {!job.busy && unlistedText && installs ? (
            <Note
              tone="warning"
              role="status"
              testId="crew-open-unlisted"
              action={
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  onClick={installInAccount}
                >
                  {unlisted === 'too_old' ? openCopy.update : openCopy.install}
                </Button>
              }
            >
              {unlistedText}
            </Note>
          ) : !job.busy && unlistedText ? (
            <p className="text-supporting text-text-muted" data-testid="crew-setup-workspaces-note">
              {unlistedText}
            </p>
          ) : null}
          {found && workspaces.length === 0 && !workspacesNoteText(found.workspacesNote ?? null) ? (
            <p className="text-body text-text-default" role="status">
              {openCopy.none(server)}
            </p>
          ) : null}
          {shared.map((name) => (
            <Note key={name} tone="warning">
              {openCopy.sameName(name)}
            </Note>
          ))}
          <div ref={foundRef} className="contents">
            <SetupWorkspaceList
              workspaces={workspaces}
              label={openCopy.foundHeading(server)}
              actions={actions}
              note={note}
              testId="crew-open-found"
            />
          </div>
          {showError && crew.error ? (
            <Note tone="danger" role="alert">
              {crew.error.message}
            </Note>
          ) : null}
        </form>
      </FieldErrorsProvider>
    </ModalShell>
  );
}
