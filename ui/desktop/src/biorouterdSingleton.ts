// BR-54 Slice A — one `biorouterd` daemon per app (not per Electron window).
//
// `startBiorouterd` used to run once per `createChat`, so N windows spawned N
// whole daemons: N tokio runtimes, N AgentManagers, N SQLite pools, and N copies
// of every MCP child process. But the daemon is *already* a session-keyed
// singleton (the AgentManager is a process-global OnceCell, the server routes by
// session_id, and the secret key is a stable module constant), so every window
// can safely share ONE daemon and address its own sessions on it. The per-window
// working directory is not lost — it still flows to each session via
// `REQUEST_DIR` / `BIOROUTER_WORKING_DIR` in the window's `appConfig`, and the
// daemon's own spawn cwd is only a fallback the GUI never relies on.
//
// This module keeps the singleton isolated from `main.ts` (which pulls in all of
// Electron) so it can be unit-tested. `start` is injected rather than imported at
// runtime for the same reason — the test never loads `./biorouterd`.
//
// Kill switch: set `BIOROUTER_SHARED_DAEMON=0` (or `false`/`off`/`no`) to revert
// to the previous per-window daemon behavior.
import type { BiorouterdResult, StartBiorouterdOptions } from './biorouterd';

let sharedBackend: Promise<BiorouterdResult> | null = null;

/**
 * Whether windows should share one daemon (the default). Off only when the
 * `BIOROUTER_SHARED_DAEMON` env var is explicitly falsy, so the change is
 * trivially revertible without a rebuild.
 */
export function isSharedDaemonEnabled(
  env: Record<string, string | undefined> = process.env
): boolean {
  const flag = env.BIOROUTER_SHARED_DAEMON;
  if (flag === undefined) return true;
  const v = flag.trim().toLowerCase();
  return v !== '0' && v !== 'false' && v !== 'off' && v !== 'no';
}

/**
 * Return the process-wide `biorouterd` backend, starting it exactly once. All
 * concurrent callers await the same in-flight promise, so racing `createChat`
 * calls (e.g. restoring several windows on launch) share ONE spawn rather than
 * racing several daemons onto several ports.
 *
 * A rejected start is not cached, so a later window can retry (e.g. after the
 * user disables an unreachable external backend).
 */
export function getSharedBackend(
  start: (options: StartBiorouterdOptions) => Promise<BiorouterdResult>,
  options: StartBiorouterdOptions
): Promise<BiorouterdResult> {
  if (!sharedBackend) {
    const pending = start(options).catch((err) => {
      // Only clear if we're still the current attempt — never clobber a newer one.
      if (sharedBackend === pending) sharedBackend = null;
      throw err;
    });
    sharedBackend = pending;
  }
  return sharedBackend;
}

/**
 * Forget the cached backend so the next `getSharedBackend` starts a fresh one.
 * Used when settings that change the daemon (e.g. external-backend config)
 * change and a window retries.
 */
export function resetSharedBackend(): void {
  sharedBackend = null;
}

// ---------------------------------------------------------------------------------------------
// Reattaching after the shared daemon restarts (R-1)
// ---------------------------------------------------------------------------------------------

/**
 * Where the app stands with the shared daemon, as every window is told: `attached` (requests
 * reach the instance this app verified), `reconnecting` (that instance is gone or was replaced,
 * and the app is attaching to the profile's daemon as it is now) or `lost` (reconnecting failed,
 * and nothing will reach the daemon until a reconnect works or Biorouter reopens).
 */
export type DaemonConnectionState = 'attached' | 'lost' | 'reconnecting';

export interface DaemonReattachDeps {
  /** Reconnecting failed: say why, and offer to quit and reopen. */
  reportFailure(message: string): Promise<'restart' | 'close'>;
  /** Attach to the profile's daemon as it is now (`SharedDaemonLink.reconnect`). */
  reconnect(): Promise<void>;
  /** Quit and reopen Biorouter. */
  restart(): void;
  /** Tell every window the new state. */
  broadcast(state: DaemonConnectionState): void;
  /** Wait between automatic attempts. Defaults to a timer; injected by tests. */
  wait?(ms: number): Promise<void>;
}

/** The waits before the second and third automatic attempts. */
export const DAEMON_RECONNECT_BACKOFF_MS = [1000, 2000] as const;

export interface DaemonReattachController {
  state(): DaemonConnectionState;
  /**
   * The attached instance was found lost (or a new window found the app not attached).
   * Reconnects on its own, up to three attempts, and says why only after the last one failed.
   * Joins a reconnect already running. Resolves to whether the app is attached afterwards.
   */
  lost(): Promise<boolean>;
  /** The lost instance answered again by itself (it had only stopped answering for a moment). */
  answered(): void;
  /** The person asked to try again (the sidebar's Try again): one attempt. */
  reconnect(): Promise<boolean>;
}

/**
 * The main process's half of R-1: one reconnect at a time, with no question asked, and every
 * window told where things stand. Electron-free and injected, so it is tested without a window.
 *
 * A reconnect never reloads a window. The proxy keeps its local address, so every window's
 * `BIOROUTER_API_HOST` stays valid, and a reload would throw away what a person was writing,
 * including the message a failed "start chat" kept for them.
 */
export function createDaemonReattachController(deps: DaemonReattachDeps): DaemonReattachController {
  let state: DaemonConnectionState = 'attached';
  let reconnecting: Promise<boolean> | null = null;
  let reporting = false;
  const wait = deps.wait ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const set = (next: DaemonConnectionState) => {
    if (next === state) return;
    state = next;
    deps.broadcast(next);
  };

  const run = (attempts: number): Promise<boolean> => {
    if (reconnecting) return reconnecting;
    set('reconnecting');
    reconnecting = (async () => {
      let failure: unknown;
      for (let attempt = 0; attempt < attempts; attempt++) {
        if (attempt > 0) await wait(DAEMON_RECONNECT_BACKOFF_MS[attempt - 1] ?? 2000);
        try {
          await deps.reconnect();
          set('attached');
          return true;
        } catch (error) {
          failure = error;
        }
      }
      set('lost');
      const message =
        failure instanceof Error && failure.message
          ? failure.message
          : 'Biorouter could not reconnect to its background service.';
      // One report on screen at a time, however many windows or losses asked.
      if (!reporting) {
        reporting = true;
        void deps
          .reportFailure(message)
          .then((choice) => {
            if (choice === 'restart') deps.restart();
          })
          .finally(() => {
            reporting = false;
          });
      }
      return false;
    })().finally(() => {
      reconnecting = null;
    });
    return reconnecting;
  };

  return {
    state: () => state,
    lost: () => run(1 + DAEMON_RECONNECT_BACKOFF_MS.length),
    answered: () => {
      if (state === 'lost') set('attached');
    },
    reconnect: () => (state === 'attached' && !reconnecting ? Promise.resolve(true) : run(1)),
  };
}
