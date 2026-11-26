/**
 * Electron main's side of the `biorouter-textinsert` helper (`crates/biorouter-textinsert`): where
 * the program is, what its answers mean, and the service that keeps it running while fn in other
 * apps or the global dictation shortcut is on (DD-97).
 *
 * The helper types a finished dictation into the app that was in front when the dictation
 * started: it puts the text on the pasteboard for a moment, posts one Command-V to that app's
 * process and puts back what the pasteboard held. Main asks `front` when a dictation starts and
 * `insert` when its text is ready, and copies the text itself when an insert is refused.
 *
 * Posting events needs macOS Accessibility. Only {@link TextInsertService.requestAccess} may make
 * macOS ask for it, and main calls that only from a person's click in Settings
 * (`dictation:insert:open-settings`). Every other request reads the permission without asking.
 *
 * The text of an insert is written to the helper's input and nowhere else: this module never logs
 * it, and no answer carries it back.
 */
import path from 'node:path';
import fs from 'node:fs';

export const TEXT_INSERT_HELPER_NAME = 'biorouter-textinsert';

/** `PROTOCOL_VERSION` in `crates/biorouter-textinsert/src/protocol.rs`. */
export const TEXT_INSERT_PROTOCOL_VERSION = 1;

/** The longest request line the helper reads, in UTF-8 bytes without its newline. */
export const MAX_REQUEST_LINE_BYTES = 256 * 1024;

/** Answers are short (no text ever comes back); anything longer is not an answer. */
const MAX_ANSWER_LINE = 4096;

/** How long each request may take before it counts as failed. */
export const FRONT_TIMEOUT_MS = 300;
export const INSERT_TIMEOUT_MS = 3_000;
export const ACCESS_TIMEOUT_MS = 1_000;
/** Long enough for macOS to show its prompt; the answer does not wait for the person. */
export const REQUEST_ACCESS_TIMEOUT_MS = 10_000;

/** Restarts allowed in {@link RESTART_WINDOW_MS} before the service gives up. */
const MAX_RESTARTS = 5;
const RESTART_WINDOW_MS = 60_000;
const RESTART_BASE_MS = 1_000;
/** The fastest a fresh helper may be started to read a permission granted meanwhile. */
const REFRESH_MIN_MS = 2_000;
/**
 * How long a stopped helper may take to leave on its own. Its input closing is its signal to go,
 * once an insert under way has put the pasteboard back; only after this is it terminated.
 */
export const STOP_GRACE_MS = 6_000;

/** The unpackaged-only variables (DESIGN 3.12). */
export const TEXT_INSERT_HELPER_ENV = 'BIOROUTER_TEXTINSERT_HELPER';
export const TEXT_INSERT_TEST_PASTEBOARD_ENV = 'BIOROUTER_TEXTINSERT_TEST_PASTEBOARD';

export type InsertRefusal = 'no-permission' | 'secure-input' | 'target-changed' | 'failed';
export type InsertResult = { ok: true } | { ok: false; reason: InsertRefusal };

/** The app in front, as the helper read it. */
export interface FrontApp {
  pid: number;
  bundle: string | null;
  /** Secure input is on (a password field has focus somewhere). */
  secureInput: boolean;
  /** Accessibility allows the helper to post events. */
  postAllowed: boolean;
}

export type TextInsertAnswer =
  | { type: 'ready'; v: number; postAllowed: boolean }
  | {
      type: 'front';
      id: number;
      pid: number | null;
      bundle: string | null;
      secureInput: boolean;
      postAllowed: boolean;
    }
  | { type: 'inserted'; id: number }
  | { type: 'refused'; id: number; reason: InsertRefusal }
  | { type: 'access'; id: number; postAllowed: boolean };

const REFUSALS: readonly InsertRefusal[] = [
  'no-permission',
  'secure-input',
  'target-changed',
  'failed',
];

const isRefusal = (value: unknown): value is InsertRefusal =>
  typeof value === 'string' && (REFUSALS as readonly string[]).includes(value);

const isId = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value > 0;

const isPid = (value: unknown): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value > 0 && value <= 0x7fff_ffff;

/** One line from the helper, or null for anything that is not exactly a known answer. */
export function parseTextInsertLine(line: string): TextInsertAnswer | null {
  if (line.length > MAX_ANSWER_LINE) return null;
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const row = value as Record<string, unknown>;
  switch (row.type) {
    case 'ready':
      return typeof row.v === 'number' && typeof row.postAllowed === 'boolean'
        ? { type: 'ready', v: row.v, postAllowed: row.postAllowed }
        : null;
    case 'front': {
      if (!isId(row.id)) return null;
      if (row.pid !== null && !isPid(row.pid)) return null;
      if (row.bundle !== null && typeof row.bundle !== 'string') return null;
      if (typeof row.secureInput !== 'boolean' || typeof row.postAllowed !== 'boolean') return null;
      return {
        type: 'front',
        id: row.id,
        pid: row.pid as number | null,
        bundle: row.bundle as string | null,
        secureInput: row.secureInput,
        postAllowed: row.postAllowed,
      };
    }
    case 'inserted':
      return isId(row.id) ? { type: 'inserted', id: row.id } : null;
    case 'refused':
      return isId(row.id) && isRefusal(row.reason)
        ? { type: 'refused', id: row.id, reason: row.reason }
        : null;
    case 'access':
      return isId(row.id) && typeof row.postAllowed === 'boolean'
        ? { type: 'access', id: row.id, postAllowed: row.postAllowed }
        : null;
    default:
      return null;
  }
}

/** Splits the helper's output into lines, dropping any line longer than an answer can be. */
export class AnswerSplitter {
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
    if (this.buffer.length > MAX_ANSWER_LINE) this.buffer = '';
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
 * build in development. `BIOROUTER_TEXTINSERT_HELPER` names another program only in an unpackaged
 * app, so a shipped app never starts a program that types into other apps from a path in its
 * environment.
 */
export function textInsertHelperPath(
  app: { isPackaged: boolean },
  env: Record<string, string | undefined> = process.env,
  exists: (file: string) => boolean = defaultExists,
  resourcesPath: string = process.resourcesPath,
  cwd: string = process.cwd()
): string | null {
  if (!app.isPackaged) {
    const override = env[TEXT_INSERT_HELPER_ENV]?.trim();
    if (override) return exists(override) ? override : null;
  }
  const candidates = app.isPackaged
    ? [path.join(resourcesPath, 'bin', TEXT_INSERT_HELPER_NAME)]
    : [
        path.join(cwd, '..', '..', 'target', 'debug', TEXT_INSERT_HELPER_NAME),
        path.join(cwd, '..', '..', 'target', 'release', TEXT_INSERT_HELPER_NAME),
        path.join(cwd, 'src', 'bin', TEXT_INSERT_HELPER_NAME),
      ];
  return candidates.find((file) => exists(file)) ?? null;
}

/** A pasteboard name the helper's `--test-pasteboard` accepts (`valid_pasteboard_name`). */
const TEST_PASTEBOARD_NAME = /^[A-Za-z0-9._-]{1,128}$/;

/**
 * The helper's arguments. In an unpackaged app only, `BIOROUTER_TEXTINSERT_TEST_PASTEBOARD` makes
 * the helper use that named pasteboard instead of the general one, so a live test runs the whole
 * real path without touching the person's clipboard. `null` when the variable is set to a name the
 * helper would refuse: start no helper rather than one that would use the general pasteboard.
 */
export function textInsertHelperArgs(
  app: { isPackaged: boolean },
  env: Record<string, string | undefined> = process.env
): string[] | null {
  if (app.isPackaged) return [];
  const raw = env[TEXT_INSERT_TEST_PASTEBOARD_ENV];
  if (raw === undefined || raw.trim() === '') return [];
  const name = raw.trim();
  return TEST_PASTEBOARD_NAME.test(name) ? ['--test-pasteboard', name] : null;
}

/** The parts of a `ChildProcess` the service uses, so a test can hand it a fake. */
export interface TextInsertProcess {
  stdout: { on(event: 'data', listener: (chunk: Buffer | string) => void): unknown } | null;
  stdin: {
    write(text: string): unknown;
    end(): unknown;
    on?(event: 'error', listener: (error: Error) => void): unknown;
  } | null;
  on(event: 'exit', listener: (code: number | null) => void): unknown;
  on(event: 'error', listener: (error: Error) => void): unknown;
  kill(signal?: 'SIGTERM'): unknown;
}

export type SpawnTextInsertHelper = (file: string, args: readonly string[]) => TextInsertProcess;

export interface TextInsertStatus {
  /** macOS, with the helper program present. */
  supported: boolean;
  /** The service is started, so the helper runs (or is being restarted). */
  active: boolean;
  /** Accessibility as the helper last reported it; null before it has answered. */
  postAllowed: boolean | null;
  /** The helper keeps failing to start, or speaks another protocol; the service stopped trying. */
  failed: boolean;
}

export interface TextInsertServiceOptions {
  platform: string;
  helperPath: string | null;
  /** From {@link textInsertHelperArgs}. */
  args?: readonly string[];
  spawn: SpawnTextInsertHelper;
  onStatus?: (status: TextInsertStatus) => void;
  now?: () => number;
}

type RequestKind = 'front' | 'insert' | 'access';

interface Pending {
  kind: RequestKind;
  settle: (answer: TextInsertAnswer | null) => void;
  timer: ReturnType<typeof setTimeout>;
}

const FAILED: InsertResult = { ok: false, reason: 'failed' };

/**
 * Keeps the helper running while it is wanted, restarts it when it dies (with a back off, and not
 * forever), and turns each request into a promise that always settles: a dead, missing or late
 * helper gives `null` for `front`, `failed` for `insert` and `false` for the access requests.
 */
export class TextInsertService {
  private child: TextInsertProcess | null = null;
  private wanted = false;
  private postAllowed: boolean | null = null;
  private failed = false;
  private restarts: number[] = [];
  private restartTimer: ReturnType<typeof setTimeout> | null = null;
  private spawnedAt = -Infinity;
  private lastFresh = -Infinity;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly now: () => number;

  constructor(private readonly options: TextInsertServiceOptions) {
    this.now = options.now ?? (() => performance.now());
  }

  get supported(): boolean {
    return this.options.platform === 'darwin' && this.options.helperPath !== null;
  }

  status(): TextInsertStatus {
    return {
      supported: this.supported,
      active: this.wanted,
      postAllowed: this.postAllowed,
      failed: this.failed,
    };
  }

  /**
   * Start the helper (fn in other apps or the global shortcut turned on). Called again after the
   * service gave up, it tries afresh.
   */
  start(): void {
    if (!this.supported || (this.wanted && !this.failed)) return;
    this.wanted = true;
    this.failed = false;
    this.restarts = [];
    this.clearRestart();
    this.spawn();
    this.publish();
  }

  /** Stop the helper. Requests waiting on it settle as failed at once. */
  stop(): void {
    if (!this.wanted && !this.child) return;
    this.wanted = false;
    this.clearRestart();
    this.retire();
    this.publish();
  }

  /** The app in front, or null when there is none, or the helper is not there to say. */
  async front(): Promise<FrontApp | null> {
    const answer = await this.request('front', { op: 'front' }, FRONT_TIMEOUT_MS);
    if (answer?.type !== 'front' || answer.pid === null) return null;
    return {
      pid: answer.pid,
      bundle: answer.bundle,
      secureInput: answer.secureInput,
      postAllowed: answer.postAllowed,
    };
  }

  /** Type `text` into the app whose process is `pid`, which must still be in front. */
  async insert(pid: number, text: string): Promise<InsertResult> {
    if (!isPid(pid) || typeof text !== 'string' || text.length === 0) return FAILED;
    const answer = await this.request('insert', { op: 'insert', pid, text }, INSERT_TIMEOUT_MS);
    if (answer?.type === 'inserted') return { ok: true };
    if (answer?.type === 'refused') return { ok: false, reason: answer.reason };
    return FAILED;
  }

  /**
   * Whether Accessibility allows the helper to post events, read without asking. When the running
   * helper says no and is idle, a fresh one is asked as well (at most every two seconds), because
   * a grant made in System Settings may reach only a new process. While the service is stopped,
   * a helper is started for this one answer and let go after it.
   */
  async access(): Promise<boolean> {
    return this.withHelper(async () => {
      const first = await this.askAccess('access');
      // Only a helper that answered "no" is asked again; one that did not answer is broken, not
      // out of date.
      if (first !== false) return first === true;
      if (!this.mayStartFresh()) return false;
      this.lastFresh = this.now();
      this.retire();
      this.spawn();
      return (await this.askAccess('access')) === true;
    });
  }

  /**
   * Ask macOS for Accessibility (its prompt, the first time only), then read it. Only from a
   * person's click in Settings: this is the one request that can make macOS ask. Like
   * {@link access}, it works while the service is stopped.
   */
  async requestAccess(): Promise<boolean> {
    return this.withHelper(async () => (await this.askAccess('request-access')) === true);
  }

  /**
   * Run `work` against a helper: the running one, or, while the service is stopped, one started
   * for it and let go after (unless the service was started meanwhile and kept it).
   */
  private async withHelper(work: () => Promise<boolean>): Promise<boolean> {
    if (this.child || this.wanted || !this.supported) return work();
    const own = this.launch();
    if (!own) return false;
    try {
      return await work();
    } finally {
      if (!this.wanted && this.child === own) this.retire();
    }
  }

  private async askAccess(op: 'access' | 'request-access'): Promise<boolean | null> {
    const timeout = op === 'access' ? ACCESS_TIMEOUT_MS : REQUEST_ACCESS_TIMEOUT_MS;
    const answer = await this.request('access', { op }, timeout);
    return answer?.type === 'access' ? answer.postAllowed : null;
  }

  private mayStartFresh(): boolean {
    const now = this.now();
    return (
      this.wanted &&
      this.child !== null &&
      this.pending.size === 0 &&
      now - this.spawnedAt >= REFRESH_MIN_MS &&
      now - this.lastFresh >= REFRESH_MIN_MS
    );
  }

  private request(
    kind: RequestKind,
    body: Record<string, unknown>,
    timeoutMs: number
  ): Promise<TextInsertAnswer | null> {
    const child = this.child;
    if (!child?.stdin) return Promise.resolve(null);
    const id = this.nextId++;
    // `id` first: the helper can still answer a line it refuses to read whole.
    const line = JSON.stringify({ id, ...body });
    if (Buffer.byteLength(line, 'utf8') > MAX_REQUEST_LINE_BYTES) return Promise.resolve(null);
    return new Promise((resolve) => {
      const timer = setTimeout(() => this.settle(id, null), timeoutMs);
      this.pending.set(id, { kind, settle: resolve, timer });
      try {
        child.stdin!.write(`${line}\n`);
      } catch {
        this.settle(id, null);
      }
    });
  }

  private settle(id: number, answer: TextInsertAnswer | null): void {
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    clearTimeout(pending.timer);
    pending.settle(answer);
  }

  private failAll(): void {
    for (const id of [...this.pending.keys()]) this.settle(id, null);
  }

  private publish(): void {
    this.options.onStatus?.(this.status());
  }

  private spawn(): void {
    if (!this.supported || !this.wanted || this.child) return;
    if (!this.launch()) this.scheduleRestart();
  }

  /** Start a helper and make it the current one; null when it could not be started. */
  private launch(): TextInsertProcess | null {
    if (!this.supported) return null;
    let child: TextInsertProcess;
    try {
      child = this.options.spawn(this.options.helperPath!, this.options.args ?? []);
    } catch {
      return null;
    }
    this.child = child;
    this.spawnedAt = this.now();
    const lines = new AnswerSplitter((line) => {
      if (this.child === child) this.handle(parseTextInsertLine(line));
    });
    child.stdout?.on('data', (chunk) => lines.push(String(chunk)));
    // A write to a helper that just died fails here, not as a crash of main.
    child.stdin?.on?.('error', () => {});
    child.on('error', () => {
      if (this.child === child) this.exited();
    });
    child.on('exit', () => {
      if (this.child === child) this.exited();
    });
    return child;
  }

  /** Let the current helper go: close its input, which makes it leave once an insert under way
   * has put the pasteboard back, and terminate it only if it is still there after a grace. */
  private retire(): void {
    const child = this.child;
    this.child = null;
    this.failAll();
    if (!child) return;
    let gone = false;
    child.on('exit', () => {
      gone = true;
    });
    try {
      child.stdin?.end();
    } catch {
      // Already closed.
    }
    const timer = setTimeout(() => {
      if (gone) return;
      try {
        child.kill('SIGTERM');
      } catch {
        // Already gone.
      }
    }, STOP_GRACE_MS);
    // Never what keeps a quitting app alive.
    (timer as unknown as { unref?: () => void }).unref?.();
  }

  private exited(): void {
    this.child = null;
    this.failAll();
    this.publish();
    this.scheduleRestart();
  }

  private clearRestart(): void {
    if (this.restartTimer) clearTimeout(this.restartTimer);
    this.restartTimer = null;
  }

  private scheduleRestart(): void {
    if (!this.wanted || this.restartTimer || this.failed) return;
    const now = this.now();
    this.restarts = this.restarts.filter((at) => now - at < RESTART_WINDOW_MS);
    if (this.restarts.length >= MAX_RESTARTS) {
      this.failed = true;
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

  private handle(answer: TextInsertAnswer | null): void {
    if (!answer) return;
    if (answer.type === 'ready') {
      if (answer.v !== TEXT_INSERT_PROTOCOL_VERSION) {
        // Another build's helper: nothing it says can be trusted to mean what this code reads.
        this.failed = true;
        this.clearRestart();
        this.retire();
        this.publish();
        return;
      }
      this.notePermission(answer.postAllowed);
      return;
    }
    const pending = this.pending.get(answer.id);
    if (!pending) return;
    if (answer.type === 'front' || answer.type === 'access')
      this.notePermission(answer.postAllowed);
    if (answer.type === 'refused' && answer.reason === 'no-permission') this.notePermission(false);
    const fits =
      (pending.kind === 'front' && answer.type === 'front') ||
      (pending.kind === 'insert' && (answer.type === 'inserted' || answer.type === 'refused')) ||
      (pending.kind === 'access' && answer.type === 'access');
    this.settle(answer.id, fits ? answer : null);
  }

  private notePermission(postAllowed: boolean): void {
    if (postAllowed === this.postAllowed) return;
    this.postAllowed = postAllowed;
    this.publish();
  }
}
