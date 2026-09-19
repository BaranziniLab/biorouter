/**
 * DependencySetupModal.tsx
 *
 * Shown automatically when the startup dependency check finds missing tools.
 * Lets the user install each missing dependency individually from inside the app,
 * with live output streaming and re-check after install.
 *
 * Dismissed state is stored in sessionStorage so re-opening the app re-checks
 * again (user may have installed things in the meantime).
 */

import { useState, useEffect, useRef } from 'react';
import { Button } from './ui/button';
import type { DependencyInfo, DependencyEvent } from '../utils/dependencyChecker';
import { ModalShell } from './ModalShell';
import { launchDependencyDebugSession } from '../utils/launchDependencyDebug';

type InstallState = 'idle' | 'running' | 'done' | 'error' | 'installed';

/**
 * Whether a dependency can be installed by the "Install all" button.
 *
 * Excludes rows whose "install command" is really a note to the user. On macOS
 * and unknown Linux distros the fallback is `# Install <dep> via your system
 * package manager` — a shell COMMENT, so running it exits 0 and the tool is then
 * reported as "the installer finished successfully but is still not detectable",
 * which reads as a broken installer rather than as no installer.
 */
export function isBatchInstallable(dep: {
  info: DependencyInfo;
  installState: InstallState;
}): boolean {
  if (dep.info.installed || dep.installState === 'done') return false;
  const cmd = dep.info.installCmd.trim();
  return cmd.length > 0 && !cmd.startsWith('#');
}

interface DepState {
  info: DependencyInfo;
  installState: InstallState;
  output: string;
  errorMsg: string;
}

export default function DependencySetupModal() {
  const [deps, setDeps] = useState<DepState[]>([]);
  const [visible, setVisible] = useState(false);
  const outputRefs = useRef<Record<string, HTMLDivElement | null>>({});
  // `dep:install` returns as soon as the child is SPAWNED — completion arrives
  // later as a push event. "Install all" has to wait on the event, or every
  // installer starts at once and two package managers fight over the same lock.
  const installWaiters = useRef<Record<string, () => void>>({});

  const settleInstall = (dep: string) => {
    const resolve = installWaiters.current[dep];
    if (resolve) {
      delete installWaiters.current[dep];
      resolve();
    }
  };

  // If this modal goes away mid-batch, the push events stop arriving and every
  // waiter still parked would never settle — leaving "Install all"'s loop
  // suspended forever on a component that no longer exists. Release them.
  useEffect(() => {
    const waiters = installWaiters.current;
    return () => {
      Object.keys(waiters).forEach((dep) => {
        const resolve = waiters[dep];
        delete waiters[dep];
        resolve();
      });
    };
  }, []);

  // Biorouter CLI install state (the bundled `biorouter` onto PATH).
  type CliStatus = {
    bundled: string | null;
    onPath: boolean;
    pathLocation: string | null;
    bundledVersion: string | null;
    pathVersion: string | null;
    needsUpdate: boolean;
    brokenOnPath: boolean;
  };
  const [cli, setCli] = useState<CliStatus | null>(null);
  const [cliState, setCliState] = useState<InstallState>('idle');
  const [cliOutput, setCliOutput] = useState('');
  const [cliError, setCliError] = useState('');
  const [cliCommand, setCliCommand] = useState<string | undefined>(undefined);

  // Whether a given status warrants showing the card: not installed at all, a
  // stale (older) install after an app upgrade, or a broken/dangling entry.
  const cliNeedsAttention = (s: CliStatus | null): boolean =>
    !!s && !!s.bundled && (!s.onPath || s.needsUpdate || s.brokenOnPath);

  // Dismissing a specific version-pair upgrade prompt persists across app
  // launches (localStorage) — a *new* app version prompts again, and the
  // toolbar CLI button can always re-open the card explicitly.
  const cliDismissKey = (s: CliStatus | null): string =>
    `cli-update-dismissed:${s?.pathVersion ?? 'none'}->${s?.bundledVersion ?? 'unknown'}`;

  // On startup, offer to install/upgrade the CLI if it isn't callable from a
  // terminal or is older than what this app ships (once per launch).
  useEffect(() => {
    let cancelled = false;
    window.electron
      .cliStatus()
      .then((s) => {
        if (cancelled || !s) return;
        if (
          cliNeedsAttention(s) &&
          !sessionStorage.getItem('cli-install-dismissed') &&
          !localStorage.getItem(cliDismissKey(s))
        ) {
          setCli(s);
          setVisible(true);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The toolbar CLI button dispatches this event when the CLI isn't installed
  // (or is stale) — re-open the modal with the install/update card even if it
  // was dismissed earlier this session.
  useEffect(() => {
    const handler = () => {
      window.electron
        .cliStatus()
        .then((s) => {
          if (!s) return;
          if (cliNeedsAttention(s)) {
            setCli(s);
            setCliState('idle');
            setVisible(true);
          }
        })
        .catch(() => {});
    };
    window.addEventListener('biorouter:open-cli-install', handler);
    return () => window.removeEventListener('biorouter:open-cli-install', handler);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleInstallCli = async () => {
    if (cliState === 'running') return;
    setCliState('running');
    setCliError('');
    let res: Awaited<ReturnType<typeof window.electron.installCli>>;
    try {
      res = await window.electron.installCli();
    } catch (error) {
      setCliError(error instanceof Error ? error.message : 'CLI installation failed');
      setCliState('error');
      return;
    }
    if (!res.success) {
      setCliError(res.error);
      setCliCommand(res.command);
      setCliState('error');
      return;
    }
    setCliOutput(res.output);
    // Verify with a fresh probe rather than assuming success: if a stale
    // `biorouter` sits earlier on PATH than where we just installed, it keeps
    // shadowing the update and the version won't actually change. Tell the user
    // exactly which file to remove instead of falsely reporting success.
    const fresh = (await window.electron.cliStatus().catch(() => null)) as CliStatus | null;
    if (fresh && cliNeedsAttention(fresh)) {
      setCli(fresh);
      setCliState('error');
      setCliError(
        fresh.pathLocation && fresh.needsUpdate
          ? `Updated the bundled copy, but an older \`biorouter\` (${
              fresh.pathVersion ?? 'unknown'
            }) still takes priority on your PATH at ${fresh.pathLocation}. Remove it (e.g. \`rm ${
              fresh.pathLocation
            }\`), then reopen this app.`
          : 'Installed, but the CLI is not resolving on PATH yet. Open a new terminal and try again.'
      );
      return;
    }
    // Success — reflect the now-matching on-PATH binary.
    setCli((c) =>
      c
        ? {
            ...c,
            onPath: true,
            needsUpdate: false,
            brokenOnPath: false,
            pathVersion: c.bundledVersion,
          }
        : c
    );
    setCliState('done');
  };

  // Listen for the push event from main process
  useEffect(() => {
    const handler = (_event: Electron.IpcRendererEvent, ...args: unknown[]) => {
      const payload = args[0] as DependencyEvent;
      if (payload.type === 'check-results' && payload.deps) {
        // Never prompt to install something whose check merely timed out: the
        // probe established nothing, and offering an install asserts absence.
        const missing = payload.deps.filter((d) => !d.installed && !d.timedOut);
        if (missing.length === 0) return;
        setDeps(
          missing.map((info) => ({
            info,
            installState: 'idle' as InstallState,
            output: '',
            errorMsg: '',
          }))
        );
        setVisible(true);
      }

      if (payload.type === 'install-start' && payload.dep) {
        setDeps((prev) =>
          prev.map((d) =>
            d.info.name === payload.dep
              ? { ...d, installState: 'running' as InstallState, output: '', errorMsg: '' }
              : d
          )
        );
      }

      if (payload.type === 'install-output' && payload.dep) {
        setDeps((prev) =>
          prev.map((d) =>
            d.info.name === payload.dep ? { ...d, output: d.output + (payload.output ?? '') } : d
          )
        );
        // Auto-scroll
        const el = outputRefs.current[payload.dep];
        if (el) el.scrollTop = el.scrollHeight;
      }

      if ((payload.type === 'install-done' || payload.type === 'recheck-results') && payload.dep) {
        settleInstall(payload.dep);
        setDeps((prev) =>
          prev.map((d) => {
            if (d.info.name !== payload.dep) return d;
            if (payload.installed) {
              return {
                ...d,
                installState: 'done' as InstallState,
                info: { ...d.info, installed: true, version: payload.version ?? null },
              };
            }
            return {
              ...d,
              installState: 'error' as InstallState,
              errorMsg:
                'Install completed but tool still not detected. Try opening a new terminal and re-running Biorouter.',
            };
          })
        );
      }

      if (payload.type === 'install-error' && payload.dep) {
        settleInstall(payload.dep);
        setDeps((prev) =>
          prev.map((d) =>
            d.info.name === payload.dep
              ? {
                  ...d,
                  installState: 'error' as InstallState,
                  errorMsg: payload.error ?? 'Unknown error',
                }
              : d
          )
        );
      }
    };

    return window.electron.on('dependency-event', handler);
  }, []);

  // Auto-dismiss when all are installed
  useEffect(() => {
    if (deps.length > 0 && deps.every((d) => d.installState === 'done' || d.info.installed)) {
      const t = setTimeout(() => setVisible(false), 1500);
      return () => clearTimeout(t);
    }
    return undefined;
  }, [deps]);

  /** Resolves when the install reaches a terminal state, not when it starts. */
  const handleInstall = async (depName: string): Promise<void> => {
    // Guard on the LIVE waiter map, not on `deps`.
    //
    // `handleInstallAll` calls this closure repeatedly across the whole batch, so
    // its `deps` is frozen at the render where the button was clicked and still
    // reads "idle" for a dependency the user has since started by hand from its
    // own row. That let two package managers run at once and fight over one lock,
    // and orphaned the first install's waiter. The ref is the only view of what is
    // actually in flight right now.
    if (installWaiters.current[depName]) return;
    if (deps.some((dep) => dep.info.name === depName && dep.installState === 'running')) return;

    const finished = new Promise<void>((resolve) => {
      installWaiters.current[depName] = resolve;
    });

    try {
      const res = await window.electron.installDependency(depName);
      if (res && 'error' in res) throw new Error(res.error);
    } catch (error) {
      settleInstall(depName);
      setDeps((prev) =>
        prev.map((dep) =>
          dep.info.name === depName
            ? {
                ...dep,
                installState: 'error',
                errorMsg: error instanceof Error ? error.message : 'Dependency installation failed',
              }
            : dep
        )
      );
      return;
    }

    await finished;
  };

  const handleOpenUrl = (url: string) => {
    if (url) window.electron.openExternal(url);
  };

  // Hand the failure to a fresh chat with shell access. New window, not this
  // one — the user hit this mid-task and should not lose the chat they were in.
  const handleDebugDependency = (dep: DepState) => {
    void launchDependencyDebugSession({
      kind: 'dependency',
      name: dep.info.name,
      displayName: dep.info.displayName,
      command: dep.info.installCmd,
      output: dep.output,
      error: dep.errorMsg,
      downloadUrl: dep.info.downloadUrl,
      requiresSudo: dep.info.requiresSudo,
    });
  };

  const handleDebugCli = () => {
    void launchDependencyDebugSession({
      kind: 'cli',
      name: 'biorouter',
      displayName: 'Biorouter CLI',
      command: cliCommand ?? 'biorouter setup-path',
      output: cliOutput,
      error: cliError,
    });
  };

  // One click for the whole list. Each install streams its own output and fails
  // independently, so a dependency that needs a hand does not stop the others.
  const handleInstallAll = async () => {
    // Same predicate as `installableCount`, so the button's number and what the
    // batch actually attempts can never disagree.
    const pending = deps.filter(isBatchInstallable);
    for (const dep of pending) {
      await handleInstall(dep.info.name);
    }
  };

  const showCli = cliNeedsAttention(cli) && cliState !== 'done';
  // Fixing an existing entry (stale version or broken/dangling) vs a first
  // install. `needsUpdate` implies it's on PATH; `brokenOnPath` means an entry
  // exists but won't run.
  const cliIsUpdate = !!cli && (cli.needsUpdate || cli.brokenOnPath);
  const cliButtonLabel = cli?.brokenOnPath ? 'Reinstall' : cliIsUpdate ? 'Update' : 'Install';
  const cliProgressLabel = cli?.brokenOnPath
    ? 'Reinstalling…'
    : cliIsUpdate
      ? 'Updating…'
      : 'Installing…';
  const isBusy = cliState === 'running' || deps.some((dep) => dep.installState === 'running');
  const installableCount = deps.filter(isBatchInstallable).length;
  if (!visible || (deps.length === 0 && !cli)) return null;

  const allDone =
    deps.length > 0 && deps.every((d) => d.installState === 'done' || d.info.installed);
  const handleDismiss = () => {
    if (isBusy) return;
    sessionStorage.setItem('cli-install-dismissed', '1');
    if (cliIsUpdate) localStorage.setItem(cliDismissKey(cli), '1');
    setVisible(false);
  };

  return (
    <ModalShell
      open={visible}
      onOpenChange={(open) => !open && handleDismiss()}
      // A report with a list: width L. An install in flight makes it
      // `required`, so a backdrop click cannot walk away from a running
      // subprocess whose output is only visible here.
      size="lg"
      purpose={isBusy ? 'required' : 'form'}
      scrollBody
      title={
        deps.length === 0 && cliIsUpdate
          ? cli?.brokenOnPath
            ? 'Repair the Biorouter CLI'
            : 'Update the Biorouter CLI'
          : 'Install missing dependencies'
      }
      subtitle={
        deps.length === 0 && cliIsUpdate
          ? cli?.brokenOnPath
            ? 'The `biorouter` on your PATH no longer runs. Reinstall it from this app.'
            : 'Your terminal `biorouter` is older than this app. Update it to match.'
          : 'The following tools are required for Biorouter features. Install them to continue.'
      }
      footer={
        <div className="flex w-full min-w-0 items-center justify-between gap-3">
          <p className="min-w-0 text-supporting text-text-muted">
            {allDone
              ? 'All dependencies installed.'
              : deps.length === 0 && cliIsUpdate
                ? 'Updating keeps the terminal CLI in sync with the desktop app.'
                : deps.length === 0
                  ? 'Install the CLI to use `biorouter` from any terminal.'
                  : 'Biorouter features may be limited until these are installed.'}
          </p>
          <div className="flex shrink-0 items-center gap-2">
            {installableCount > 1 && !allDone && (
              <Button variant="default" size="sm" onClick={handleInstallAll} disabled={isBusy}>
                Install all ({installableCount})
              </Button>
            )}
            <Button variant="outline" size="sm" onClick={handleDismiss} disabled={isBusy}>
              {allDone ? 'Done' : 'Dismiss'}
            </Button>
          </div>
        </div>
      }
    >
      {/* Dep list */}
      <div className="flex flex-col gap-3 py-3">
        {/* Biorouter CLI install card */}
        {(showCli || cliState === 'done') && (
          <div
            className={`rounded-xl border px-4 py-3 ${
              cliState === 'done'
                ? 'border-border-success/40 bg-background-success/10'
                : cliState === 'error'
                  ? 'border-border-danger/30 bg-background-danger/5'
                  : 'biorouter-modal-panel bg-background-medium/20'
            }`}
          >
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm font-semibold flex items-center gap-1.5">
                  {cliState === 'done' && <span className="text-text-success">✓</span>}
                  {cliState === 'error' && <span className="text-text-danger">✗</span>}
                  {cliState === 'running' && (
                    <span className="inline-block w-3 h-3 rounded-full border-2 border-text-muted border-t-transparent animate-spin" />
                  )}
                  Biorouter CLI
                </p>
                <p className="text-[11px] text-text-muted mt-0.5">
                  {cliState === 'done'
                    ? 'Installed. Open a new terminal and run `biorouter`.'
                    : cli?.brokenOnPath
                      ? 'The `biorouter` on your PATH won’t run. Reinstall it.'
                      : cliIsUpdate
                        ? `Update ${cli?.pathVersion ?? 'older'} → ${cli?.bundledVersion ?? 'latest'} to match this app.`
                        : 'Call `biorouter` from any terminal.'}
                </p>
              </div>
              {cliState !== 'running' && cliState !== 'done' && (
                <Button
                  variant="default"
                  size="sm"
                  className="h-7 text-xs"
                  disabled={isBusy}
                  onClick={handleInstallCli}
                >
                  {cliButtonLabel}
                </Button>
              )}
              {cliState === 'running' && (
                <span className="text-xs text-text-muted">{cliProgressLabel}</span>
              )}
            </div>
            {cliOutput && (
              <div className="mt-2 font-mono text-[11px] text-text-muted bg-background-medium/40 rounded p-2 max-h-28 overflow-y-auto whitespace-pre-wrap break-all">
                {cliOutput}
              </div>
            )}
            {cliState === 'error' && cliError && (
              <div className="mt-2 flex min-w-0 items-start justify-between gap-3">
                <p className="min-w-0 text-xs text-text-danger">{cliError}</p>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 shrink-0 text-xs"
                  onClick={handleDebugCli}
                  title="Open a new chat with this error and let Biorouter work it out"
                >
                  Debug with Biorouter
                </Button>
              </div>
            )}
          </div>
        )}

        {deps.map(({ info, installState, output, errorMsg }) => {
          const installed = installState === 'done' || info.installed;
          return (
            <div
              key={info.name}
              className={`rounded-lg border px-4 py-3 ${
                installed
                  ? 'border-border-success/40 bg-background-success/10'
                  : installState === 'error'
                    ? 'border-border-danger/30 bg-background-danger/5'
                    : 'border-border-subtle bg-background-medium/20'
              }`}
            >
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-sm font-semibold flex items-center gap-1.5">
                    {installed && <span className="text-text-success">✓</span>}
                    {installState === 'error' && <span className="text-text-danger">✗</span>}
                    {installState === 'running' && (
                      <span className="inline-block w-3 h-3 rounded-full border-2 border-text-muted border-t-transparent animate-spin" />
                    )}
                    {info.displayName}
                  </p>
                  {installed && info.version && (
                    <p className="text-[11px] text-text-muted mt-0.5">{info.version}</p>
                  )}
                  {!installed && installState === 'idle' && (
                    <p className="text-[11px] text-text-muted font-mono mt-0.5 break-all">
                      {info.installCmd}
                    </p>
                  )}
                </div>
                <div className="flex items-center gap-2 shrink-0 ml-3">
                  {!installed && info.downloadUrl && installState !== 'running' && (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 text-xs text-text-muted"
                      onClick={() => handleOpenUrl(info.downloadUrl)}
                      title="Open download page"
                    >
                      Download
                    </Button>
                  )}
                  {!installed && installState !== 'running' && (
                    <Button
                      variant="default"
                      size="sm"
                      className="h-7 text-xs"
                      // Disabled while ANY install is running. A live row button
                      // during a batch let the user start a second installer for a
                      // dependency the batch had not reached yet, so two package
                      // managers contended for one lock.
                      disabled={isBusy}
                      onClick={() => handleInstall(info.name)}
                    >
                      Install
                    </Button>
                  )}
                  {installState === 'running' && (
                    <span className="text-xs text-text-muted">Installing…</span>
                  )}
                </div>
              </div>

              {/* Live output */}
              {output && (
                <div
                  ref={(el) => {
                    outputRefs.current[info.name] = el;
                  }}
                  className="mt-2 font-mono text-[11px] text-text-muted bg-background-medium/40 rounded p-2 max-h-28 overflow-y-auto whitespace-pre-wrap break-all"
                >
                  {output}
                </div>
              )}

              {/* Error — always paired with the escape hatch. An error string on
                  its own is a dead end; Biorouter can already read the output,
                  probe the machine and fix it. */}
              {installState === 'error' && errorMsg && (
                <div className="mt-2 flex min-w-0 items-start justify-between gap-3">
                  <p className="min-w-0 text-xs text-text-danger">{errorMsg}</p>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 shrink-0 text-xs"
                    onClick={() => handleDebugDependency({ info, installState, output, errorMsg })}
                    title="Open a new chat with this error and let Biorouter work it out"
                  >
                    Debug with Biorouter
                  </Button>
                </div>
              )}

              {/* Linux sudo note */}
              {info.requiresSudo && !installed && installState !== 'running' && (
                <p className="mt-1 text-[11px] text-text-warning">
                  Requires sudo. You may be prompted for your password in a terminal.
                </p>
              )}
            </div>
          );
        })}
      </div>
    </ModalShell>
  );
}
