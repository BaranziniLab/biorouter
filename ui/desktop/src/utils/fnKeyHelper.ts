/**
 * Electron main's side of the `biorouter-fnkey` helper (`crates/biorouter-fnkey`): where the
 * program is, what its lines mean, and the service that keeps it running while the fn shortcut
 * is on and turns what it reports into gesture intents (`fnGesture.ts`).
 *
 * The helper reports only fn down, fn up and "another key while fn was down". It is never started
 * while the shortcut is off, so a Mac with the shortcut off runs no keyboard listener at all.
 */
import path from 'node:path';
import fs from 'node:fs';
import { FnGestureDriver, type FnIntent } from './fnGesture';

export const FN_HELPER_NAME = 'biorouter-fnkey';

/** What macOS does when fn is pressed: nothing, input source, emoji picker, system dictation. */
export type FnUsage = 0 | 1 | 2 | 3;
export type FnPermission = 'granted' | 'denied';

export type FnHelperMessage =
  | {
      type: 'ready';
      permission: FnPermission;
      listening: boolean;
      fnUsage: FnUsage | null;
      /** The helper's clock now, so its event times can be placed before any key arrives. */
      t: number | null;
    }
  /** `t`: when the key event happened, in ms on the helper's clock (Quartz's event timestamp). */
  | { type: 'fn'; down: boolean; t: number | null }
  | { type: 'other'; t: number | null }
  | { type: 'permission'; permission: FnPermission }
  | { type: 'listener-lost' };

const MAX_LINE = 512;

const isPermission = (value: unknown): value is FnPermission =>
  value === 'granted' || value === 'denied';

/** An event time: a finite, non-negative number, or null when the line carries none. */
function eventTime(value: unknown): number | null | undefined {
  if (value === undefined) return null;
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : undefined;
}

/** One line from the helper, or null for anything that is not exactly a known message. */
export function parseFnHelperLine(line: string): FnHelperMessage | null {
  if (line.length > MAX_LINE) return null;
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const row = value as Record<string, unknown>;
  switch (row.type) {
    case 'ready': {
      if (row.v !== 1 || !isPermission(row.permission) || typeof row.listening !== 'boolean')
        return null;
      const usage = row.fnUsage;
      const fnUsage =
        usage === 0 || usage === 1 || usage === 2 || usage === 3 ? (usage as FnUsage) : null;
      if (usage !== null && fnUsage === null) return null;
      const t = eventTime(row.t);
      if (t === undefined) return null;
      return { type: 'ready', permission: row.permission, listening: row.listening, fnUsage, t };
    }
    case 'fn': {
      const t = eventTime(row.t);
      return typeof row.down === 'boolean' && t !== undefined
        ? { type: 'fn', down: row.down, t }
        : null;
    }
    case 'other': {
      const t = eventTime(row.t);
      return t === undefined ? null : { type: 'other', t };
    }
    case 'permission':
      return isPermission(row.permission)
        ? { type: 'permission', permission: row.permission }
        : null;
    case 'listener-lost':
      return { type: 'listener-lost' };
    default:
      return null;
  }
}

/** Splits a byte stream into lines, dropping any line longer than the protocol allows. */
export class LineSplitter {
  private buffer = '';

  constructor(private readonly onLine: (line: string) => void) {}

  push(chunk: string): void {
    this.buffer += chunk;
    let newline = this.buffer.indexOf('\n');
    while (newline >= 0) {
      const line = this.buffer.slice(0, newline).trim();
      this.buffer = this.buffer.slice(newline + 1);
      if (line) this.onLine(line);
      newline = this.buffer.indexOf('\n');
    }
    if (this.buffer.length > MAX_LINE) this.buffer = '';
  }
}

function defaultExists(file: string): boolean {
  try {
    return fs.statSync(file).isFile();
  } catch {
    return false;
  }
}

/**
 * The helper beside `biorouterd` in a packaged app (`Contents/Resources/bin`), or a workspace
 * build in development. `BIOROUTER_FNKEY_HELPER` names another program only in an unpackaged app,
 * so a shipped app never starts a keyboard listener from a path in its environment.
 */
export function fnKeyHelperPath(
  app: { isPackaged: boolean },
  env: Record<string, string | undefined> = process.env,
  exists: (file: string) => boolean = defaultExists,
  resourcesPath: string = process.resourcesPath,
  cwd: string = process.cwd()
): string | null {
  if (!app.isPackaged) {
    const override = env.BIOROUTER_FNKEY_HELPER?.trim();
    if (override) return exists(override) ? override : null;
  }
  const candidates = app.isPackaged
    ? [path.join(resourcesPath, 'bin', FN_HELPER_NAME)]
    : [
        path.join(cwd, '..', '..', 'target', 'debug', FN_HELPER_NAME),
        path.join(cwd, '..', '..', 'target', 'release', FN_HELPER_NAME),
        path.join(cwd, 'src', 'bin', FN_HELPER_NAME),
      ];
  return candidates.find((file) => exists(file)) ?? null;
}

/** The parts of a `ChildProcess` the service uses, so a test can hand it a fake. */
export interface HelperProcess {
  stdout: { on(event: 'data', listener: (chunk: Buffer | string) => void): unknown } | null;
  stdin: { write(text: string): unknown; end(): unknown } | null;
  on(event: 'exit', listener: (code: number | null) => void): unknown;
  on(event: 'error', listener: (error: Error) => void): unknown;
  kill(signal?: 'SIGTERM'): unknown;
}

export type SpawnHelper = (file: string) => HelperProcess;

export interface FnKeyStatus {
  /** macOS, with the helper program present. */
  supported: boolean;
  /** The shortcut is on, so the helper runs. */
  active: boolean;
  /** Input Monitoring, as the helper last read it; `unknown` before it has answered. */
  permission: FnPermission | 'unknown';
  /** The listener runs, so fn gestures work. */
  listening: boolean;
  fnUsage: FnUsage | null;
  /** The helper keeps failing to start; Biorouter stopped trying until the next refresh. */
  failed: boolean;
}

/** Restarts allowed in {@link RESTART_WINDOW_MS} before the service gives up. */
const MAX_RESTARTS = 5;
const RESTART_WINDOW_MS = 60_000;
const RESTART_BASE_MS = 1_000;
/** The fastest a refresh may restart a helper that is not listening. */
const REFRESH_MIN_MS = 2_000;

export interface FnKeyServiceOptions {
  platform: string;
  helperPath: string | null;
  spawn: SpawnHelper;
  onIntent: (intent: FnIntent) => void;
  onStatus: (status: FnKeyStatus) => void;
  now?: () => number;
}

/**
 * Keeps the helper running while the shortcut is on, restarts it when it dies (with a back off,
 * and not forever), and feeds what it reports into a {@link FnGestureDriver}.
 */
export class FnKeyService {
  private child: HelperProcess | null = null;
  private wanted = false;
  private state: Omit<FnKeyStatus, 'supported' | 'active'> = {
    permission: 'unknown',
    listening: false,
    fnUsage: null,
    failed: false,
  };
  private restarts: number[] = [];
  private restartTimer: ReturnType<typeof setTimeout> | null = null;
  private lastRefresh = -Infinity;
  /** The helper's clock against ours: the smallest delay seen, so a late line keeps its time. */
  private clockOffset: number | null = null;
  private lastEventAt = -Infinity;
  private readonly driver: FnGestureDriver;
  private readonly now: () => number;

  constructor(private readonly options: FnKeyServiceOptions) {
    this.now = options.now ?? (() => performance.now());
    this.driver = new FnGestureDriver(options.onIntent, this.now);
  }

  get supported(): boolean {
    return this.options.platform === 'darwin' && this.options.helperPath !== null;
  }

  status(): FnKeyStatus {
    return { supported: this.supported, active: this.wanted, ...this.state };
  }

  /** Turn the listener on or off (the shortcut setting). */
  setActive(active: boolean): void {
    if (active === this.wanted) return;
    this.wanted = active;
    if (active) {
      this.state.failed = false;
      this.restarts = [];
      this.spawn();
    } else {
      this.clearRestart();
      this.kill();
      this.state = { ...this.state, listening: false };
    }
    this.publish();
  }

  /**
   * Read Input Monitoring again, by starting a fresh helper when the current one does not listen:
   * a grant made in System Settings reaches a new process. Called when a Biorouter window gains
   * focus and when Settings asks; at most once every two seconds.
   */
  refresh(): void {
    if (!this.wanted || !this.supported || this.state.listening) return;
    const now = this.now();
    if (now - this.lastRefresh < REFRESH_MIN_MS) return;
    this.lastRefresh = now;
    this.state.failed = false;
    this.restarts = [];
    this.clearRestart();
    this.kill();
    this.spawn();
  }

  /** Ask macOS for Input Monitoring. Only from a person's click in Settings. */
  requestPermission(): boolean {
    if (!this.child?.stdin) return false;
    this.child.stdin.write('request\n');
    return true;
  }

  dispose(): void {
    this.wanted = false;
    this.clearRestart();
    this.kill();
  }

  private publish(): void {
    this.options.onStatus(this.status());
  }

  private spawn(): void {
    if (!this.supported || !this.wanted || this.child) return;
    let child: HelperProcess;
    try {
      child = this.options.spawn(this.options.helperPath!);
    } catch {
      this.scheduleRestart();
      return;
    }
    this.child = child;
    this.clockOffset = null;
    const lines = new LineSplitter((line) => {
      if (this.child === child) this.handle(parseFnHelperLine(line));
    });
    child.stdout?.on('data', (chunk) => lines.push(String(chunk)));
    child.on('error', () => {
      if (this.child === child) this.exited();
    });
    child.on('exit', () => {
      if (this.child === child) this.exited();
    });
  }

  private kill(): void {
    const child = this.child;
    this.child = null;
    this.driver.feed({ type: 'reset' });
    if (!child) return;
    try {
      child.stdin?.end();
    } catch {
      // Already closed: the helper exits on its own when its input ends.
    }
    try {
      child.kill('SIGTERM');
    } catch {
      // Already gone.
    }
  }

  private exited(): void {
    this.child = null;
    this.driver.feed({ type: 'reset' });
    this.state = { ...this.state, listening: false };
    this.publish();
    this.scheduleRestart();
  }

  private clearRestart(): void {
    if (this.restartTimer) clearTimeout(this.restartTimer);
    this.restartTimer = null;
  }

  private scheduleRestart(): void {
    if (!this.wanted || this.restartTimer) return;
    const now = this.now();
    this.restarts = this.restarts.filter((at) => now - at < RESTART_WINDOW_MS);
    if (this.restarts.length >= MAX_RESTARTS) {
      this.state = { ...this.state, failed: true, listening: false };
      this.publish();
      return;
    }
    this.restarts.push(now);
    const delay = RESTART_BASE_MS * 2 ** (this.restarts.length - 1);
    this.restartTimer = setTimeout(() => {
      this.restartTimer = null;
      this.spawn();
    }, delay);
  }

  /**
   * When a key event happened, on this process's clock. A line that reaches a busy main thread
   * late keeps its place in time: the offset between the two clocks is the smallest delay seen,
   * and times never run backwards.
   */
  private eventAt(helperTime: number | null): number {
    const now = this.now();
    let at = now;
    if (helperTime !== null) {
      const offset = now - helperTime;
      this.clockOffset = this.clockOffset === null ? offset : Math.min(this.clockOffset, offset);
      at = helperTime + this.clockOffset;
    }
    at = Math.min(now, Math.max(at, this.lastEventAt));
    this.lastEventAt = at;
    return at;
  }

  private handle(message: FnHelperMessage | null): void {
    if (!message) return;
    switch (message.type) {
      case 'ready':
        if (message.t !== null) this.eventAt(message.t);
        this.state = {
          ...this.state,
          permission: message.permission,
          listening: message.listening,
          fnUsage: message.fnUsage,
          // Granted, yet macOS refused the listener: nothing will change until a person retries.
          failed: message.permission === 'granted' && !message.listening,
        };
        this.publish();
        return;
      case 'permission': {
        const changed = message.permission !== this.state.permission;
        this.state = { ...this.state, permission: message.permission };
        if (message.permission === 'granted' && !this.state.listening) {
          // A grant reaches a new process: start one that can listen.
          this.lastRefresh = -Infinity;
          this.refresh();
        }
        if (changed) this.publish();
        return;
      }
      case 'listener-lost':
        this.state = { ...this.state, listening: false };
        this.publish();
        this.kill();
        this.scheduleRestart();
        return;
      case 'fn':
        this.driver.feed({ type: message.down ? 'fn-down' : 'fn-up', at: this.eventAt(message.t) });
        return;
      case 'other':
        this.driver.feed({ type: 'other-key', at: this.eventAt(message.t) });
        return;
    }
  }
}
