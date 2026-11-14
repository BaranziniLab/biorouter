/**
 * Updating Biorouter from a `biorouter serve` page (SD-19 in
 * `docs/deployment/serve-decisions.md`).
 *
 * The desktop app's updater lives in the Electron main process, which a browser
 * does not have. On a `serve` page the daemon does the work instead
 * (`routes/serve_update.rs`): it checks the release feed, downloads and verifies
 * the update in the background, and on the person's click installs it and exits
 * so `serve` restarts on the new version. This module is the page's half: one
 * store that every surface reads (the sidebar row, the update dialog, Settings
 * › About), polling `GET /headless/update` and following a restart through to
 * the reload.
 *
 * ⚠ **How the page knows the new daemon is up.** `serve` mints a new daemon
 * secret on every start, so while it restarts, the status request this page
 * sends with the OLD secret first fails (nothing listening), then answers `401`
 * (a new daemon, which does not know that secret). The `401` is the signal to
 * reload: the reload asks for the page again with the browser's access cookie,
 * which a restart for an update keeps, and the new page carries the new secret.
 */

import { isBrowserSurface } from './surface';

export type ServePhase =
  | 'idle'
  | 'checking'
  | 'up_to_date'
  | 'downloading'
  | 'ready'
  | 'needs_admin'
  | 'installed'
  | 'manual'
  | 'installing'
  | 'restarting'
  | 'error';

export interface ServeLastResult {
  outcome: 'updated' | 'rolled_back' | 'failed';
  from: string;
  to: string;
  reason?: string | null;
  /** Seconds since the epoch. */
  at: number;
}

/** `GET /headless/update`, as the daemon sends it. */
export interface ServeUpdateStatus {
  available: boolean;
  /** The version running, said even when `available` is false. */
  version?: string;
  current?: string;
  latest?: string | null;
  phase?: ServePhase;
  percent?: number;
  error?: string | null;
  installKind?: 'package' | 'app_bundle' | 'source' | 'manual';
  manualReason?: 'source' | 'windows' | 'unrecognised' | 'not_writable' | 'no_package' | null;
  adminCommand?: string | null;
  lastResult?: ServeLastResult | null;
  feedOverridden?: boolean;
  runningTurns: number;
  /** Background shell jobs and subagents a restart would stop. */
  runningWork?: number;
}

export interface ServeUpdateView {
  status: ServeUpdateStatus | null;
  /** The daemon went away during a restart and this page is waiting for it. */
  reconnecting: boolean;
  /** Seconds this page has waited for the daemon to come back. */
  waitedSeconds: number;
  /** A refusal or failure from the last action, in one line. */
  actionError: string | null;
  /** The person asked for a check here, so its verdict is worth saying. */
  checkRequested: boolean;
  /** The update dialog is open. */
  dialogOpen: boolean;
  /** A recent update that did not finish, said once when the page comes back. */
  result: ServeLastResult | null;
}

/** Phases in which an update is offered, arriving, or being installed. */
const OFFERING: ReadonlySet<ServePhase> = new Set([
  'downloading',
  'ready',
  'needs_admin',
  'installed',
  'manual',
  'installing',
  'restarting',
]);

/** Phases the page polls quickly through. */
const MOVING: ReadonlySet<ServePhase> = new Set([
  'checking',
  'downloading',
  'installing',
  'restarting',
]);

const SLOW_POLL_MS = 30_000;
const FAST_POLL_MS = 1_500;
/** After two minutes of waiting for a restart, look less often. */
const PATIENT_POLL_MS = 5_000;

/** Session record of the last reload a new daemon caused, so it never loops. */
export const RELOADED_AT_KEY = 'biorouter:serve-update-reloaded-at';

/** Session flag: this tab asked for the update, so it says "Updated" after the reload. */
export const UPDATE_REQUESTED_KEY = 'biorouter:serve-update-requested';

/** Is `latest` strictly newer than `current`? Dotted numbers; a suffix is ignored. */
export function isNewerVersion(
  latest: string | null | undefined,
  current: string | null | undefined
) {
  const parts = (v: string | null | undefined) =>
    (v ?? '')
      .trim()
      .replace(/^v/i, '')
      .split(/[-+]/)[0]
      .split('.')
      .map((p) => Number.parseInt(p, 10))
      .map((n) => (Number.isFinite(n) ? n : 0));
  const a = parts(latest);
  const b = parts(current);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}

/** Should the sidebar offer an update? */
export function offersUpdate(view: ServeUpdateView): boolean {
  if (view.reconnecting) return true;
  const status = view.status;
  if (!status?.available || !status.phase) return false;
  // No file for this server's processor: nothing anyone here can install.
  if (status.phase === 'manual' && status.manualReason === 'no_package') return false;
  if (OFFERING.has(status.phase)) return true;
  // A failure with a newer release in hand stays offered, so "Try again" is a click away.
  return status.phase === 'error' && isNewerVersion(status.latest, status.current);
}

/** The headless endpoint under this page's mount (a reverse proxy prefix included). */
export function updateUrl(tail = ''): string {
  return new URL(`./headless/update${tail}`, document.baseURI).href;
}

type Listener = () => void;

/**
 * The page's update store. One per page; {@link serveUpdateStore} is it.
 * Exported as a class so a test can make a fresh one with its own fetch.
 */
export class ServeUpdateStore {
  private view: ServeUpdateView = {
    status: null,
    reconnecting: false,
    waitedSeconds: 0,
    actionError: null,
    checkRequested: false,
    dialogOpen: false,
    result: null,
  };
  private listeners = new Set<Listener>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private started = false;
  /** The daemon answered this page at least once with this page's secret. */
  private answered = false;
  /** An install was asked for, here or (seen through the status) elsewhere. */
  private expectingRestart = false;
  private awaySince: number | null = null;

  constructor(
    private readonly deps: {
      fetch: typeof fetch;
      secret: () => Promise<string>;
      reload: () => void;
      now: () => number;
      session?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
    }
  ) {}

  getSnapshot = (): ServeUpdateView => this.view;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    this.start();
    return () => this.listeners.delete(listener);
  };

  private set(patch: Partial<ServeUpdateView>) {
    this.view = { ...this.view, ...patch };
    for (const listener of this.listeners) listener();
  }

  openDialog = () => {
    this.set({ dialogOpen: true, actionError: null });
    // What the dialog warns about (chats still working) must be now, not as of
    // the last background poll.
    void this.poll();
  };
  closeDialog = () => this.set({ dialogOpen: false, result: null });

  start() {
    if (this.started) return;
    this.started = true;
    void this.poll();
  }

  stop() {
    this.started = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private schedule() {
    if (!this.started) return;
    if (this.timer) clearTimeout(this.timer);
    const phase = this.view.status?.phase;
    const fast = this.view.reconnecting || this.expectingRestart || (phase && MOVING.has(phase));
    const delay = !fast
      ? SLOW_POLL_MS
      : this.view.waitedSeconds > 120
        ? PATIENT_POLL_MS
        : FAST_POLL_MS;
    this.timer = setTimeout(() => void this.poll(), delay);
  }

  private async request(path: string, init?: RequestInit): Promise<Response> {
    const secret = await this.deps.secret();
    return this.deps.fetch(updateUrl(path), {
      ...init,
      // The access cookie rides along: it is what says a person on this page,
      // not only something holding the secret, is asking (SD-19).
      credentials: 'same-origin',
      headers: { ...(init?.headers ?? {}), 'X-Secret-Key': secret },
    });
  }

  /** Read the status once; follows a restart through to the reload. */
  async poll(): Promise<void> {
    let response: Response | null = null;
    try {
      response = await this.request('');
    } catch {
      response = null;
    }
    try {
      if (response === null || response.status >= 500) {
        this.daemonAway();
      } else if (response.status === 401 || response.status === 429) {
        // A daemon that does not know this page's secret: a new one (429 is its
        // throttle on failed secrets). Whether or not this page saw the update
        // start, it cannot work until it reloads with the new one; a page that
        // has never been answered was never working, so it is left alone.
        if (this.answered || this.expectingRestart || this.view.reconnecting) {
          this.reloadOnce();
          return;
        }
      } else if (response.ok) {
        const status = (await response.json()) as ServeUpdateStatus;
        this.answered = true;
        if (status.phase === 'installing' || status.phase === 'restarting') {
          this.expectingRestart = true;
        }
        const autoOpen =
          this.expectingRestart &&
          !this.view.dialogOpen &&
          status.phase !== this.view.status?.phase;
        this.set({
          status,
          reconnecting: false,
          waitedSeconds: 0,
          ...(autoOpen ? { dialogOpen: true } : {}),
        });
        if (status.phase === 'error') this.expectingRestart = false;
        this.awaySince = null;
        this.announceResult(status);
      }
    } finally {
      this.schedule();
    }
  }

  private reloadOnce() {
    const session = this.deps.session;
    const now = this.deps.now();
    const last = Number(session?.getItem(RELOADED_AT_KEY) ?? 0);
    if (now - last < 30_000) {
      // Reloaded moments ago and still refused: the address itself changed
      // (serve restarted with a new token), which a reload cannot fix.
      this.set({
        actionError: 'Biorouter restarted with a new address. Open the address it printed.',
      });
      this.stop();
      return;
    }
    session?.setItem(RELOADED_AT_KEY, String(now));
    this.deps.reload();
  }

  private daemonAway() {
    if (!this.answered) return;
    if (!this.expectingRestart && !this.view.reconnecting) {
      // Not a restart this page knows about: a network blip, or the operator
      // stopped serve. Say nothing; the next poll tries again.
      return;
    }
    const now = this.deps.now();
    if (this.awaySince === null) this.awaySince = now;
    this.set({
      reconnecting: true,
      dialogOpen: true,
      waitedSeconds: Math.round((now - this.awaySince) / 1000),
    });
  }

  private announceResult(status: ServeUpdateStatus) {
    const result = status.lastResult;
    if (!result) return;
    const session = this.deps.session;
    const seenKey = `biorouter:serve-update-result:${result.at}`;
    if (session?.getItem(seenKey)) return;
    const recent = this.deps.now() / 1000 - result.at < 10 * 60;
    if (!recent) return;
    session?.setItem(seenKey, '1');
    if (result.outcome !== 'updated') {
      this.set({ dialogOpen: true, result });
    }
  }

  /** Look for an update now. */
  async check(): Promise<void> {
    this.set({ checkRequested: true, actionError: null });
    try {
      const response = await this.request('/check', { method: 'POST' });
      if (!response.ok) this.set({ actionError: await refusalMessage(response) });
    } catch {
      this.set({ actionError: 'Biorouter could not be reached.' });
    }
    await this.poll();
  }

  /**
   * Install and restart. `stopRunning` says the person saw that chats are
   * still working and chose to go on. Answers the refusal's reason, if any.
   */
  async install(stopRunning: boolean): Promise<string | null> {
    this.set({ actionError: null });
    let response: Response;
    try {
      response = await this.request('/install', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ stopRunning }),
      });
    } catch {
      this.set({ actionError: 'Biorouter could not be reached.' });
      return 'unreachable';
    }
    if (response.status === 202) {
      this.expectingRestart = true;
      this.deps.session?.setItem(UPDATE_REQUESTED_KEY, '1');
      if (this.view.status) {
        this.set({ status: { ...this.view.status, phase: 'installing' } });
      }
      await this.poll();
      return null;
    }
    const body = (await response.json().catch(() => null)) as {
      reason?: string;
      message?: string;
      runningTurns?: number;
      runningWork?: number;
    } | null;
    this.set({ actionError: body?.message ?? `Biorouter answered ${response.status}.` });
    if (body?.reason === 'turns_running' && this.view.status) {
      this.set({
        status: {
          ...this.view.status,
          runningTurns: body.runningTurns ?? 1,
          runningWork: body.runningWork ?? 0,
        },
      });
    }
    return body?.reason ?? 'refused';
  }
}

async function refusalMessage(response: Response): Promise<string> {
  const body = (await response.json().catch(() => null)) as { message?: string } | null;
  return body?.message ?? `Biorouter answered ${response.status}.`;
}

let shared: ServeUpdateStore | null = null;

/** The page's one store, made on first use on a browser surface. */
export function serveUpdateStore(): ServeUpdateStore | null {
  if (!isBrowserSurface()) return null;
  if (!shared) {
    shared = new ServeUpdateStore({
      fetch: (...args) => window.fetch(...args),
      secret: async () => (await window.electron?.getSecretKey?.()) ?? '',
      reload: () => window.location.reload(),
      now: () => Date.now(),
      session: safeSession(),
    });
  }
  return shared;
}

function safeSession(): Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | undefined {
  try {
    return window.sessionStorage;
  } catch {
    return undefined;
  }
}
