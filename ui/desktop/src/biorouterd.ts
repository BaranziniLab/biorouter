import Electron from 'electron';
import fs from 'node:fs';
import { spawn, ChildProcess } from 'child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'net';
import os from 'node:os';
import path from 'node:path';
import log from './utils/logger';
import { App } from 'electron';
import { Buffer } from 'node:buffer';
import { StringDecoder } from 'node:string_decoder';

import { status } from './api';
import { Client } from './api/client';
import { ExternalBiorouterdConfig } from './utils/settings';
import { isSharedDaemonEnabled } from './biorouterdSingleton';
import {
  createDaemonProxy,
  discoverDaemonRuntime,
  verifyDaemonRuntime,
  type DaemonRuntime,
} from './daemonRuntime';

export const findAvailablePort = (): Promise<number> => {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once('error', reject);

    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as { port: number };
      server.close(() => {
        log.info(`Found available port: ${port}`);
        resolve(port);
      });
    });
  });
};

// Check if biorouterd server is ready by polling the status endpoint
export const checkServerStatus = async (client: Client, errorLog: string[]): Promise<boolean> => {
  const interval = 100; // ms
  const maxAttempts = 100; // 10s

  const fatal = (line: string) => {
    const trimmed = line.trim().toLowerCase();
    return trimmed.startsWith("thread 'main' panicked at") || trimmed.startsWith('error:');
  };

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    if (errorLog.some(fatal)) {
      log.error('Detected fatal error in server logs');
      return false;
    }
    try {
      await status({ client, throwOnError: true });
      return true;
    } catch {
      if (attempt === maxAttempts) {
        log.error(`Server failed to respond after ${(interval * maxAttempts) / 1000} seconds`);
      }
    }
    await new Promise((resolve) => setTimeout(resolve, interval));
  }
  return false;
};

export type DaemonLogLevel = 'error' | 'warn' | 'info' | 'debug';

// eslint-disable-next-line no-control-regex
const ANSI_ESCAPE_PATTERN = /\u001b\[[0-9;]*m/g;

// The daemon writes tracing's pretty format to stderr:
//   `  2026-07-26T18:40:14.289898Z  WARN some::target: message`
// possibly with continuation lines (`    at src/foo.rs:12`). Match the level
// only at the head of the line (after an optional timestamp) so a level word
// appearing inside a message body cannot re-classify the line.
const DAEMON_LEVEL_PATTERN =
  /^\s*(?:\[?\d{4}-\d{2}-\d{2}[T ][0-9:.]+(?:Z|[+-]\d{2}:?\d{2})?\]?\s+)?\[?(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL)\]?\b/;

// Not everything on the daemon's stderr comes from tracing. Rust's default
// panic hook writes `thread '<name>' panicked at <loc>:` directly, and
// `biorouterd`'s `async fn main() -> anyhow::Result<()>` makes the standard
// `Termination` impl print `Error: <chain>` when startup fails. Neither line
// carries a level word, so the tracing parser cannot see them — and these are
// precisely the lines that must not be filed under `info`. Anchored at the
// head of the (trimmed) line so the same words inside a message body cannot
// re-classify it.
const RUST_PANIC_PATTERN = /^thread\s+'[^']*'\s+panicked\s+at\b/;
const FATAL_PREFIX_PATTERN = /^(?:error|fatal)\s*:/i;

/**
 * Map a line of `biorouterd` stderr onto the electron-log level that matches
 * the daemon's own severity. A line whose level cannot be parsed defaults to
 * `info`: it is a line that could not be parsed, not an error. Logging all
 * daemon stderr at `error` destroys severity at the process boundary and makes
 * main.log unfilterable (see issue #49).
 *
 * The daemon's console layer is configured with `.pretty().with_ansi(false)`
 * (crates/biorouter-server/src/logging.rs), so there is no JSON tracing format
 * to parse here — only the pretty format, plus the two non-tracing shapes
 * above.
 */
export const daemonStderrLogLevel = (line: string): DaemonLogLevel => {
  const plain = line.replace(ANSI_ESCAPE_PATTERN, '');
  const match = DAEMON_LEVEL_PATTERN.exec(plain);
  switch (match?.[1]) {
    case 'ERROR':
    case 'FATAL':
      return 'error';
    case 'WARN':
    case 'WARNING':
      return 'warn';
    case 'DEBUG':
    case 'TRACE':
      return 'debug';
    case undefined: {
      const head = plain.trimStart();
      return RUST_PANIC_PATTERN.test(head) || FATAL_PREFIX_PATTERN.test(head) ? 'error' : 'info';
    }
    default:
      return 'info';
  }
};

export interface StderrLineReader {
  /** Feed one raw chunk from the pipe. Emits every complete line it now has. */
  push: (chunk: Buffer | string) => void;
  /** Emit a trailing unterminated line, if any. Safe to call more than once. */
  flush: () => void;
}

/**
 * Longest logical stderr line the reader keeps, in characters.
 *
 * Holding a line back until its newline arrives is what lets the classifier and
 * the startup fatal probe see whole lines — but it also means an unterminated
 * record is retained in full, and nothing bounds how long the daemon (or a
 * dependency it links, or a subprocess sharing the pipe) may go without writing
 * a `\n`. Without a cap that buffer grows with the record, and because nothing
 * is emitted until the newline, the 500-line ring stays *empty* the whole time:
 * the symptom is main-process memory climbing with no diagnostic at all.
 *
 * 8 KiB is far more than any real tracing line and also bounds the ring itself
 * (500 lines x 8 KiB ~ 4 MB worst case) and the per-attempt cost of
 * `checkServerStatus`'s `trim().toLowerCase()` scan over it.
 */
export const STDERR_MAX_LINE_CHARS = 8192;

/** Appended to a line cut short at `STDERR_MAX_LINE_CHARS`, so a reader of
 * main.log can tell a truncated record from a genuinely short one. */
export const STDERR_TRUNCATION_SUFFIX = ' …[truncated]';

/**
 * Reassemble `biorouterd`'s stderr pipe into whole lines.
 *
 * Node stream chunks are byte-buffer sized, not line framed: one logical line
 * routinely arrives split across two `data` events, and a multi-byte UTF-8
 * character can straddle the boundary. Splitting each chunk independently
 * therefore hands the consumer fragments — `ER` then `ROR …` — which defeats
 * both the level classifier (both fragments look unparseable, so both land at
 * `info`) and, more seriously, the startup fatal probe in `checkServerStatus`,
 * which scans the ring for a *whole* `thread 'main' panicked at` / `error:`
 * line. A split panic would be invisible to it and startup would hang out the
 * full status-poll timeout instead of failing fast.
 *
 * `StringDecoder` holds back an incomplete multi-byte sequence; `pending` holds
 * back an incomplete line, capped at `STDERR_MAX_LINE_CHARS`. The cap keeps the
 * *prefix* because that is the part with meaning: both `daemonStderrLogLevel`
 * and `checkServerStatus`'s fatal predicate are anchored at the head of the
 * line. Everything past the cap is discarded until the next newline, at which
 * point the reader resumes normally. Blank lines are dropped, as before.
 */
export const createStderrLineReader = (onLine: (line: string) => void): StderrLineReader => {
  const decoder = new StringDecoder('utf8');
  // Head of the logical line being assembled; never longer than the cap (plus
  // the suffix, once `truncated` is set and further appends are refused).
  let pending = '';
  // Set when the current logical line has overflowed: the rest of it is dropped
  // rather than buffered. Cleared when its newline finally arrives.
  let truncated = false;

  const emit = (line: string) => {
    // Windows CRLF: keep the line itself clean rather than trailing a \r.
    const cleaned = line.endsWith('\r') ? line.slice(0, -1) : line;
    if (cleaned.trim()) onLine(cleaned);
  };

  // Append `text[start, end)` to the pending line, keeping only what fits.
  // Index-based rather than slicing first, so an oversized chunk is never
  // copied into a same-sized temporary just to be thrown away.
  const append = (text: string, start: number, end: number) => {
    if (truncated || end <= start) return;
    const room = STDERR_MAX_LINE_CHARS - pending.length;
    if (end - start <= room) {
      pending += text.slice(start, end);
      return;
    }
    pending += text.slice(start, start + room) + STDERR_TRUNCATION_SUFFIX;
    truncated = true;
  };

  const takePending = (): string => {
    const line = pending;
    pending = '';
    truncated = false;
    return line;
  };

  return {
    push(chunk: Buffer | string) {
      const text = typeof chunk === 'string' ? chunk : decoder.write(chunk);
      let start = 0;
      let newline = text.indexOf('\n');
      while (newline !== -1) {
        append(text, start, newline);
        emit(takePending());
        start = newline + 1;
        newline = text.indexOf('\n', start);
      }
      append(text, start, text.length);
    },
    flush() {
      const tail = decoder.end();
      append(tail, 0, tail.length);
      const line = takePending();
      if (line) emit(line);
    },
  };
};

export interface BiorouterdResult {
  baseUrl: string;
  managed: boolean;
  workingDir: string;
  process: ChildProcess;
  errorLog: string[];
}

/**
 * The URL the `BIOROUTER_EXTERNAL_BACKEND` developer escape hatch points at.
 *
 * ⚠ It used to be the hard-coded string `http://127.0.0.1:3000`, which quietly
 * ignored `BIOROUTER_EXTERNAL_PORT` — the variable this repo's own
 * documentation calls "Backend port (default 3000)" and which `just
 * debug-server` sets. So a developer who moved their daemon off 3000 had the app
 * connect to 3000 anyway and report the backend as down, with nothing in the
 * logs naming the port it actually tried.
 *
 * Only a well-formed positive port is honoured; anything else falls back to
 * 3000 rather than composing a URL that cannot resolve.
 */
export const externalBackendUrlFromEnv = (env: Record<string, string | undefined>): string => {
  const raw = (env.BIOROUTER_EXTERNAL_PORT ?? '').trim();
  // ⚠ A whole-string digit match, not `parseInt`. `parseInt('12.5')` is 12 and
  // `parseInt('80abc')` is 80, so a typo would be silently accepted as a
  // *different* port — which is worse than the fallback, because the app would
  // then report a backend as down at an address nobody typed.
  const port = /^\d+$/.test(raw) ? Number(raw) : Number.NaN;
  const valid = Number.isInteger(port) && port > 0 && port < 65536;
  return `http://127.0.0.1:${valid ? port : 3000}`;
};

const connectToExternalBackend = (workingDir: string, url: string): BiorouterdResult => {
  log.info(`Using external biorouterd backend at ${url}`);

  const mockProcess = {
    pid: undefined,
    kill: () => {
      log.info(`Not killing external process that is managed externally`);
    },
  } as ChildProcess;

  return { baseUrl: url, managed: false, workingDir, process: mockProcess, errorLog: [] };
};

// ⚠ Issue #56 DR-16: this interface gains NOTHING for the user-action key, and
// that omission is the point. AR-11 measured a daemon's environment to be
// recoverable in-process by any tool that reads a caller-named path
// (`/proc/self/environ`) or, on macOS, by `sysctl(KERN_PROCARGS2)` — which is
// not a path at all and which no sandbox profile can gate. A user-proof
// delivered as an env var is a proof the model already holds. It goes on stdin.
interface BiorouterProcessEnv {
  [key: string]: string | undefined;

  HOME: string;
  USERPROFILE: string;
  APPDATA: string;
  LOCALAPPDATA: string;
  PATH: string;
  BIOROUTER_PORT: string;
  BIOROUTER_SERVER__SECRET_KEY?: string;
  /** SD-12: this launcher sends a user-action digest on stdin. See the spawn. */
  BIOROUTER_USER_ACTION_EXPECTED?: string;
  BIOROUTER_DISABLE_KEYRING?: string;
}

/**
 * SHA-256, hex. The daemon is handed this and never the key itself, so a tool
 * that reads the daemon's heap recovers a value it cannot present.
 */
const sha256Hex = (value: string): string => createHash('sha256').update(value).digest('hex');

export function validateDaemonApprovalSecret(secret: string | undefined): asserts secret is string {
  if (!secret || !/^[!-~]{32,4096}$/.test(secret))
    throw new Error(
      'Approval secret must contain 32–4096 printable ASCII characters without spaces or other whitespace.'
    );
}

export interface StartBiorouterdOptions {
  app: App;
  serverSecret: string;
  /** Per-launch renderer proof retained by main. Shared daemons receive the
   * digest of a separately supplied approval secret; the local proxy maps a
   * valid renderer proof to that secret only for the authenticated instance.
   * Legacy private daemons still receive this key's digest through stdin.
   */
  userActionKey?: string;
  dir: string;
  env?: Partial<BiorouterProcessEnv>;
  externalBiorouterd?: ExternalBiorouterdConfig;
  requestNewUserActionKey?: () => Promise<string | undefined>;
  requestUserActionKey?: (runtime: {
    profileId: string;
    instanceId: string;
    userActionInstalled: boolean;
  }) => Promise<string | undefined>;
}

async function attachSharedDaemon(
  options: StartBiorouterdOptions,
  runtime: DaemonRuntime,
  workingDir: string,
  ownedProcess?: ChildProcess,
  errorLog: string[] = [],
  ownedProof?: string
): Promise<BiorouterdResult> {
  await verifyDaemonRuntime(runtime);
  const owned = ownedProcess?.pid === runtime.pid;
  if (!owned && !runtime.user_action_installed)
    throw new Error(
      'This existing daemon has no installed human approval proof. Stop it explicitly and restart it through a trusted desktop launcher before attaching.'
    );
  const daemonProof = owned
    ? ownedProof
    : await options.requestUserActionKey?.({
        profileId: runtime.profile_id,
        instanceId: runtime.instance_id,
        userActionInstalled: runtime.user_action_installed,
      });
  if (!owned && (!runtime.user_action_installed || !daemonProof))
    throw new Error(
      'This profile daemon requires its independently held approval secret. Enter it through the desktop attachment prompt; it is never loaded from daemon metadata.'
    );
  validateDaemonApprovalSecret(daemonProof);
  const proxy = await createDaemonProxy(
    runtime,
    options.serverSecret,
    options.userActionKey,
    daemonProof,
    options.env?.BIOROUTER_RENDERER_ORIGIN
  );
  try {
    const response = await fetch(`${proxy.baseUrl}/crew/connections`, {
      headers: {
        'X-Secret-Key': options.serverSecret,
        'X-User-Action': options.userActionKey || '',
      },
      signal: AbortSignal.timeout(10000),
    });
    await response.body?.cancel();
    if (!response.ok)
      throw new Error(
        'The daemon did not accept human-authorized access. Reopen the app to retry with its existing approval secret, or cancel attachment.'
      );
  } catch (error) {
    proxy.close();
    throw error;
  }
  const handle = new ChildProcess();
  let detached = false;
  const detach = () => {
    if (detached) return;
    detached = true;
    options.app.removeListener('will-quit', detach);
    proxy.close();
    ownedProcess?.unref();
    handle.emit('exit', 0, null);
  };
  handle.kill = () => {
    detach();
    return true;
  };
  options.app.on('will-quit', detach);
  return { baseUrl: proxy.baseUrl, managed: true, workingDir, process: handle, errorLog };
}

async function stopFailedSharedStartup(child: ChildProcess): Promise<boolean> {
  if (child.pid === undefined || child.exitCode !== null || child.signalCode !== null) return true;
  return new Promise<boolean>((resolve) => {
    let hardStop: ReturnType<typeof setTimeout> | undefined;
    let deadline: ReturnType<typeof setTimeout> | undefined;
    const finish = (exited: boolean) => {
      clearTimeout(hardStop);
      clearTimeout(deadline);
      child.removeListener('exit', onExit);
      resolve(exited);
    };
    const onExit = () => finish(true);
    child.once('exit', onExit);
    hardStop = setTimeout(() => {
      deadline = setTimeout(() => finish(false), 1000);
      child.kill('SIGKILL');
    }, 2000);
    child.kill('SIGINT');
  });
}

export const startBiorouterd = async (
  options: StartBiorouterdOptions
): Promise<BiorouterdResult> => {
  const { app, serverSecret, userActionKey, dir: inputDir, env = {}, externalBiorouterd } = options;
  const isWindows = process.platform === 'win32';
  const profileRoot = !app.isPackaged ? process.env.BIOROUTER_DEV_PROFILE_ROOT : undefined;
  const homeDir = profileRoot ? path.join(profileRoot, 'home') : os.homedir();
  if (profileRoot && (externalBiorouterd?.enabled || process.env.BIOROUTER_EXTERNAL_BACKEND))
    throw new Error('Isolated development profiles cannot reuse an external backend.');
  const dir = path.resolve(path.normalize(inputDir));

  if (externalBiorouterd?.enabled && externalBiorouterd.url) {
    return connectToExternalBackend(dir, externalBiorouterd.url);
  }

  if (process.env.BIOROUTER_EXTERNAL_BACKEND) {
    return connectToExternalBackend(dir, externalBackendUrlFromEnv(process.env));
  }

  const sharedRuntime = !isWindows && isSharedDaemonEnabled();
  if (isWindows && process.env.BIOROUTER_SHARED_DAEMON !== undefined && isSharedDaemonEnabled())
    throw new Error(
      'Shared profile daemon attachment on Windows requires an owner-protected named pipe and is not available yet.'
    );
  let staleInstance: string | undefined;
  if (sharedRuntime) {
    const existing = discoverDaemonRuntime();
    if (existing) {
      try {
        await verifyDaemonRuntime(existing);
      } catch (error) {
        const failure = error as Error & { code?: string; syscall?: string };
        if (
          failure.syscall !== 'connect' ||
          !['ENOENT', 'ECONNREFUSED'].includes(failure.code || '')
        )
          throw error;
        staleInstance = existing.instance_id;
      }
      if (!staleInstance) return attachSharedDaemon(options, existing, dir);
    }
  }

  const newDaemonProof = sharedRuntime ? await options.requestNewUserActionKey?.() : undefined;
  if (sharedRuntime) {
    if (newDaemonProof === undefined)
      throw new Error(
        'Shared daemon startup cancelled. Reopen the app to supply your independently held approval secret. No daemon was started.'
      );
    validateDaemonApprovalSecret(newDaemonProof);
  }

  let biorouterdPath = getBiorouterdBinaryPath(app);

  const resolvedBiorouterdPath = path.resolve(biorouterdPath);

  const port = await findAvailablePort();
  // Bounded ring of the most recent stderr lines. Without a cap this array
  // grows for the lifetime of the Electron main process — a long-running
  // chatty biorouterd can retain hundreds of MB of strings and trip a fatal
  // V8 CHECK on the main thread during optimizing compile / GC compaction.
  const STDERR_RING_MAX = 500;
  const stderrLines: string[] = [];
  const appendStderrLine = (line: string) => {
    stderrLines.push(line);
    if (stderrLines.length > STDERR_RING_MAX) {
      stderrLines.splice(0, stderrLines.length - STDERR_RING_MAX);
    }
  };

  log.info(`Starting biorouterd from: ${resolvedBiorouterdPath} on port ${port} in dir ${dir}`);

  const additionalEnv: BiorouterProcessEnv = {
    HOME: homeDir,
    USERPROFILE: homeDir,
    APPDATA: process.env.APPDATA || path.join(homeDir, 'AppData', 'Roaming'),
    LOCALAPPDATA: process.env.LOCALAPPDATA || path.join(homeDir, 'AppData', 'Local'),
    PATH: `${path.dirname(resolvedBiorouterdPath)}${path.delimiter}${process.env.PATH || ''}`,
    BIOROUTER_PORT: String(port),
    BIOROUTER_SERVER__SECRET_KEY: serverSecret,
    // Issue #56 DR-16 / SD-12. This launcher writes a user-action digest down
    // stdin below, and says so here.
    //
    // ⚠ **Unconditional, including when there is no key to send.** That case is
    // exactly the one worth naming: `UserActionProof::NoKeyInstalled` means only
    // "this process read no valid digest", which a `biorouter serve` daemon (no
    // key by design, `Stdio::null()`) and a desktop daemon whose key never
    // arrived both satisfy. SD-12 lets the first start a new chat on a private
    // model without a proof, because nobody there can ever give one; the second
    // is a repairable fault and must keep refusing. Declaring the intent is what
    // lets the daemon tell them apart — so a `if (userActionKey)` here would
    // delete the signal in the only situation that needs it.
    //
    // Never the key or its digest: AR-11 measured the environment to be
    // recoverable in-process. This is a boolean-shaped claim that authenticates
    // nothing, and a value a model could only use to make the daemon stricter.
    BIOROUTER_USER_ACTION_EXPECTED: '1',
    // Dev Electron rebuilds should not trigger macOS Keychain prompts; packaged
    // builds keep the normal OS credential-store behavior.
    BIOROUTER_DISABLE_KEYRING:
      process.env.BIOROUTER_DISABLE_KEYRING ?? (!app.isPackaged ? 'true' : undefined),
    // Default Auto Visualiser to CDN-referenced assets so each figure's persisted
    // HTML blob is a few KB instead of megabytes of inlined D3/Chart.js/Leaflet/
    // Mermaid — keeps figure-heavy sessions light in the renderer heap and SQLite.
    // Respects an explicit user override; set BIOROUTER_AUTOVIS_CDN=0 for fully
    // offline/self-contained figures (no network needed at render time).
    BIOROUTER_AUTOVIS_CDN: process.env.BIOROUTER_AUTOVIS_CDN ?? '1',
    ...env,
    ...(sharedRuntime ? { BIOROUTER_SHARED_DAEMON: '1' } : {}),
  } as BiorouterProcessEnv;

  const processEnv: BiorouterProcessEnv = {
    ...(profileRoot
      ? Object.fromEntries(
          Object.entries(process.env).filter(([key]) =>
            [
              'PATH',
              'LANG',
              'LC_ALL',
              'TERM',
              'SHELL',
              'SystemRoot',
              'WINDIR',
              'ComSpec',
              'PATHEXT',
              'BIOROUTER_PATH_ROOT',
              'BIOROUTER_DEV_PROFILE_ROOT',
              'BIOROUTER_DEV_PROFILE_NAME',
            ].includes(key)
          )
        )
      : process.env),
    ...additionalEnv,
    ...(profileRoot
      ? {
          HOME: homeDir,
          USERPROFILE: homeDir,
          APPDATA: path.join(profileRoot, 'appdata'),
          LOCALAPPDATA: path.join(profileRoot, 'localappdata'),
          TMPDIR: path.join(profileRoot, 'temp'),
          TMP: path.join(profileRoot, 'temp'),
          TEMP: path.join(profileRoot, 'temp'),
          XDG_CONFIG_HOME: path.join(homeDir, '.config'),
          XDG_DATA_HOME: path.join(homeDir, '.local/share'),
          XDG_STATE_HOME: path.join(homeDir, '.local/state'),
          BIOROUTER_DISABLE_KEYRING: 'true',
        }
      : {}),
  } as BiorouterProcessEnv;

  if (isWindows && !resolvedBiorouterdPath.toLowerCase().endsWith('.exe')) {
    biorouterdPath = resolvedBiorouterdPath + '.exe';
  } else {
    biorouterdPath = resolvedBiorouterdPath;
  }
  log.info(`Binary path resolved to: ${biorouterdPath}`);

  const spawnOptions = {
    cwd: dir,
    env: processEnv,
    // stdin is a pipe (it used to be 'ignore') for one reason: issue #56's
    // user-action digest is written down it and the pipe is closed immediately.
    // Shared daemons outlive Electron; their log sinks cannot depend on its pipes.
    stdio: ['pipe', sharedRuntime ? 'ignore' : 'pipe', sharedRuntime ? 'ignore' : 'pipe'] as [
      'pipe',
      'ignore' | 'pipe',
      'ignore' | 'pipe',
    ],
    windowsHide: true,
    detached: isWindows || sharedRuntime,
    shell: false,
  };

  // Unchanged, and asserted to stay that way: argv sits in the same
  // KERN_PROCARGS2 block as the environment and in /proc/<pid>/cmdline, so the
  // user-action key may not travel here either.
  const safeArgs = ['agent'];

  const biorouterdProcess: ChildProcess = spawn(biorouterdPath, safeArgs, spawnOptions);

  // Issue #56 DR-16. The DIGEST, never the key — the daemon compares a hash of
  // what a caller presents, so what it stores authenticates nothing. Written
  // and closed immediately: the daemon's read is bounded (2s) and a pipe whose
  // writer never closes would stall its startup. After `end()` fd 0 is at EOF,
  // so every process the daemon later spawns inherits a stdin that carries
  // nothing.
  const launchProof = sharedRuntime ? newDaemonProof : userActionKey;
  if (launchProof) {
    biorouterdProcess.stdin?.write(sha256Hex(launchProof) + '\n');
  }
  biorouterdProcess.stdin?.end();

  if (isWindows && biorouterdProcess.unref) {
    biorouterdProcess.unref();
  }

  biorouterdProcess.stdout?.on('data', (data: Buffer) => {
    log.info(`biorouterd stdout for port ${port} and dir ${dir}: ${data.toString()}`);
  });

  // Exactly one place turns a stderr line into output: it logs at the daemon's
  // own severity *and* records it in the ring the startup probe reads, so the
  // two can never disagree about what a line was. Logging the whole stream at
  // `error` made main.log a wall of apparent failures with no way to find the
  // real one.
  const stderrReader = createStderrLineReader((line) => {
    log[daemonStderrLogLevel(line)](`biorouterd stderr for port ${port} and dir ${dir}: ${line}`);
    appendStderrLine(line);
  });

  biorouterdProcess.stderr?.on('data', (data: Buffer) => stderrReader.push(data));
  // A daemon that dies mid-line leaves its most interesting line unterminated.
  // Flush on both signals — `end` may never fire if the pipe is destroyed, and
  // `close` may arrive first. `flush` clears its buffer, so it cannot double-emit.
  biorouterdProcess.stderr?.on('end', () => stderrReader.flush());

  biorouterdProcess.on('close', (code: number | null) => {
    stderrReader.flush();
    log.info(`biorouterd process exited with code ${code} for port ${port} and dir ${dir}`);
  });

  biorouterdProcess.on('error', (err: Error) => {
    // Do not `throw` here — this callback runs inside the EventEmitter, and
    // a synchronous throw becomes an uncaught exception in the Node event
    // loop, fatally aborting the Electron main process with no usable
    // diagnostic. Record the failure so checkServerStatus can surface it
    // through the normal startup error path instead.
    log.error(`Failed to start biorouterd on port ${port} and dir ${dir}`, err);
    // "error:" prefix matches checkServerStatus's fatal() predicate so the
    // startup probe short-circuits with a useful error rather than waiting
    // out the 10s status-poll timeout. Appended through the same bounded
    // helper as real stderr lines (it is already logged above, so it does not
    // go through the logging path a second time).
    appendStderrLine(`error: failed to spawn biorouterd: ${err.message}`);
  });

  if (sharedRuntime) {
    try {
      for (let attempt = 0; attempt < 100; attempt++) {
        const runtime = discoverDaemonRuntime();
        if (runtime && runtime.instance_id !== staleInstance)
          return await attachSharedDaemon(
            options,
            runtime,
            dir,
            biorouterdProcess,
            stderrLines,
            newDaemonProof
          );
        if (
          biorouterdProcess.exitCode !== null ||
          stderrLines.some((line) => /^error:/i.test(line.trim()))
        )
          throw new Error(
            'Shared profile daemon failed to start. Inspect its startup diagnostics.'
          );
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      throw new Error(
        'Shared profile daemon did not publish its private runtime descriptor. Inspect the daemon before retrying.'
      );
    } catch (error) {
      const stopped = await stopFailedSharedStartup(biorouterdProcess);
      if (!stopped)
        throw Object.assign(
          new Error(
            'Shared daemon startup failed, and its owned child did not exit within the cleanup deadline. Inspect that daemon before retrying.'
          ),
          { cause: error }
        );
      throw error;
    }
  }

  const try_kill_biorouter = () => {
    try {
      if (isWindows) {
        const pid = biorouterdProcess.pid?.toString() || '0';
        // `taskkill.exe` is a console program, so without this the last
        // thing the user sees on the way out is a black box (#368).
        spawn('taskkill', ['/pid', pid, '/T', '/F'], { shell: false, windowsHide: true });
      } else {
        biorouterdProcess.kill?.();
      }
    } catch (error) {
      log.error('Error while terminating biorouterd process:', error);
    }
  };

  app.on('will-quit', () => {
    log.info('App quitting, terminating biorouterd server');
    try_kill_biorouter();
  });

  log.info(`Biorouterd server successfully started on port ${port}`);
  // Issue #56. This daemon is deliberately unreachable from a terminal: the port
  // above was chosen by the OS at launch and `serverSecret` is minted per
  // launch, and neither is written anywhere a `biorouter` CLI could read it. So
  // `biorouter session send/watch/attach/cancel` cannot talk to the app's
  // daemon, by construction rather than by omission — and the CLI's own error
  // (`session_watch.rs`'s NO_SECRET_KEY_HELP) names the External Backend
  // setting as the supported way round it.
  //
  // Logged, at the one moment the values are both known, so that a support
  // question about "why does the CLI say it cannot authenticate" is answerable
  // from main.log without anyone having to rediscover this. The SECRET is never
  // logged — only the fact that one exists.
  log.info(
    `This managed biorouterd is not reachable from a terminal: its port (${port}) is ephemeral ` +
      'and its secret is per-launch. To drive sessions from the `biorouter` CLI, run your own ' +
      'daemon with a fixed BIOROUTER_PORT and BIOROUTER_SERVER__SECRET_KEY and point the app at ' +
      'it via Settings > Advanced > External Backend.'
  );
  return {
    baseUrl: `http://127.0.0.1:${port}`,
    managed: true,
    workingDir: dir,
    process: biorouterdProcess,
    errorLog: stderrLines,
  };
};

/**
 * Resolve the bundled `biorouter` CLI binary (sibling of biorouterd). Used to
 * offer "install the Biorouter CLI onto PATH" and to run `biorouter doctor`
 * from the desktop app, so the dependency/install logic lives in one place
 * (the Rust `biorouter::system` module) shared by the CLI and the GUI.
 */
export const getBiorouterCliBinaryPath = (app: Electron.App): string => {
  const executableName = process.platform === 'win32' ? 'biorouter.exe' : 'biorouter';
  const possiblePaths = app.isPackaged
    ? [path.join(process.resourcesPath, 'bin', executableName)]
    : [
        path.join(process.cwd(), '..', '..', 'target', 'debug', executableName),
        path.join(process.cwd(), '..', '..', 'target', 'release', executableName),
        path.join(process.cwd(), 'src', 'bin', executableName),
        path.join(process.cwd(), 'bin', executableName),
      ];
  for (const binPath of possiblePaths) {
    const resolved = path.resolve(binPath);
    if (fs.existsSync(resolved) && fs.statSync(resolved).isFile()) {
      return resolved;
    }
  }
  throw new Error(`Could not find ${executableName} in: ${possiblePaths.join(', ')}`);
};

const getBiorouterdBinaryPath = (app: Electron.App): string => {
  let executableName = process.platform === 'win32' ? 'biorouterd.exe' : 'biorouterd';

  let possiblePaths: string[];
  if (!app.isPackaged) {
    possiblePaths = [
      path.join(process.cwd(), '..', '..', 'target', 'debug', executableName),
      path.join(process.cwd(), '..', '..', 'target', 'release', executableName),
      path.join(process.cwd(), 'src', 'bin', executableName),
      path.join(process.cwd(), 'bin', executableName),
    ];
  } else {
    possiblePaths = [path.join(process.resourcesPath, 'bin', executableName)];
  }

  for (const binPath of possiblePaths) {
    try {
      const resolvedPath = path.resolve(binPath);

      if (fs.existsSync(resolvedPath)) {
        const stats = fs.statSync(resolvedPath);
        if (stats.isFile()) {
          return resolvedPath;
        } else {
          log.error(`Path exists but is not a regular file: ${resolvedPath}`);
        }
      }
    } catch (error) {
      log.error(`Error checking path ${binPath}:`, error);
    }
  }

  throw new Error(
    `Could not find ${executableName} binary in any of the expected locations: ${possiblePaths.join(
      ', '
    )}`
  );
};
