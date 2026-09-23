/**
 * dependencyChecker.ts
 *
 * Checks for required system dependencies (git, python, uv, npm) at startup.
 * Runs in the Electron main process. Sends check results and install progress
 * to the renderer via IPC push events on channel 'dependency-event'.
 *
 * Why custom PATH augmentation: Electron inherits a minimal PATH from launchd/
 * services, not the user's interactive shell. Tools installed via Homebrew,
 * cargo, or pyenv won't be on that PATH, so we probe known install locations.
 */

import { app, ipcMain, BrowserWindow } from 'electron';
import { execFile, spawn } from 'child_process';
import { promisify } from 'util';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';
import log from './logger';
import { getBiorouterCliBinaryPath } from '../biorouterd';

// ─── Types ────────────────────────────────────────────────────────────────────

export interface DependencyInfo {
  name: string;
  displayName: string;
  version: string | null;
  installed: boolean;
  /**
   * The check could not be completed in time, so this tool's presence is
   * UNKNOWN rather than disproved. `installed` is false either way; anything
   * that would offer an install must consult this first.
   */
  timedOut?: boolean;
  installCmd: string;
  requiresSudo: boolean;
  downloadUrl: string;
  required?: boolean;
}

export interface DependencyEvent {
  type:
    | 'check-results'
    | 'install-start'
    | 'install-output'
    | 'install-done'
    | 'install-error'
    | 'recheck-results';
  dep?: string;
  deps?: DependencyInfo[];
  output?: string;
  error?: string;
  installed?: boolean;
  version?: string | null;
}

// ─── Timing budgets ───────────────────────────────────────────────────────────

// Every probe below runs off the main thread (see `runProbe`), so these bound how
// long a *stale* answer takes to arrive, not how long the UI is frozen.
// Both budgets must clear the cold first-execution cost of a freshly installed
// binary, which is an operating-system scan rather than compute: a cold
// `llama-server --version` measured 8.33s real at 0.04s CPU, against 0.05s warm.
// 8_000 sat BELOW that, so the probe that motivated the bound was the one it cut
// off. The CLI bounds one prerequisite at 12s and runs them concurrently
// (crates/biorouter/src/system.rs PROBE_TIMEOUT), so the doctor budget must
// exceed 12s or the CLI is killed before it can answer and the desktop silently
// falls back to its own duplicated probes.
export const PROBE_TIMEOUT_MS = 12_000;
export const DOCTOR_TIMEOUT_MS = 20_000;
// A dependency set does not change while the app is open often enough to justify
// re-spawning `biorouter doctor` on every caller. Startup, the modal mount and the
// post-install re-check used to each pay the full probe cost.
export const DEPENDENCY_CACHE_TTL_MS = 60_000;

// ─── PATH augmentation ────────────────────────────────────────────────────────

function buildAugmentedPath(): string {
  const home = os.homedir();
  const extra: string[] = [];

  if (process.platform === 'darwin') {
    extra.push(
      // rustup (~/.cargo/bin) must precede the Homebrew prefixes: source builds
      // (e.g. cryptography ≥49, which no longer ships Intel-Mac wheels) invoke
      // whichever `rustc` is first on PATH, and a self-contained rustup toolchain
      // is reliable whereas Homebrew's `rust` dynamically links `libLLVM.dylib`
      // and breaks whenever `llvm` is upgraded out from under it.
      path.join(home, '.cargo', 'bin'),
      '/usr/local/bin',
      '/opt/homebrew/bin',
      '/opt/homebrew/sbin',
      '/usr/bin',
      '/bin',
      path.join(home, '.local', 'bin'),
      path.join(home, 'Library', 'Python', '3.12', 'bin'),
      path.join(home, 'Library', 'Python', '3.11', 'bin'),
      path.join(home, 'Library', 'Python', '3.10', 'bin')
    );
  } else if (process.platform === 'win32') {
    const localAppData = process.env.LOCALAPPDATA || '';
    const programFiles = process.env.ProgramFiles || 'C:\\Program Files';
    const programFilesX86 = process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)';
    extra.push(
      // Biorouter's own bundled shims (uv.exe, uvx.exe, npx.cmd, git shim if added)
      // — must be first so bundled tools take priority over stale system installs
      path.join(localAppData, 'Biorouter', 'bin'),
      // rustup ahead of system tooling so source builds prefer it (see darwin note)
      path.join(home, '.cargo', 'bin'),
      path.join(programFiles, 'Git', 'bin'),
      path.join(programFilesX86, 'Git', 'bin'),
      path.join(localAppData, 'Programs', 'Python', 'Python312'),
      path.join(localAppData, 'Programs', 'Python', 'Python311'),
      path.join(localAppData, 'Programs', 'Python', 'Python310'),
      path.join(programFiles, 'nodejs'),
      path.join(localAppData, 'Programs', 'nodejs'),
      path.join(localAppData, 'uv', 'bin'),
      // Bundled git fallback — appended last so system git always takes priority
      path.join(localAppData, 'Biorouter', 'git', 'cmd')
    );
  } else {
    // Linux
    extra.push(
      // rustup ahead of the system dirs, so source builds use the rustup
      // toolchain rather than a distro `rustc` that may be too old to compile
      // modern Rust-backed wheels.
      path.join(home, '.cargo', 'bin'),
      '/usr/bin',
      '/usr/local/bin',
      '/bin',
      path.join(home, '.local', 'bin')
    );
  }

  const current = process.env.PATH || '';
  const sep = process.platform === 'win32' ? ';' : ':';
  const unique = [...new Set([...extra, ...current.split(sep).filter(Boolean)])];
  return unique.join(sep);
}

const AUGMENTED_PATH = buildAugmentedPath();
export const SPAWN_ENV = { ...process.env, PATH: AUGMENTED_PATH };

// ─── Linux distro detection ───────────────────────────────────────────────────

type LinuxDistro = 'deb' | 'rpm' | 'unknown';

function detectLinuxDistro(): LinuxDistro {
  try {
    const content = fs.readFileSync('/etc/os-release', 'utf8').toLowerCase();
    if (content.includes('ubuntu') || content.includes('debian')) return 'deb';
    if (
      content.includes('fedora') ||
      content.includes('rhel') ||
      content.includes('centos') ||
      content.includes('rocky') ||
      content.includes('alma')
    )
      return 'rpm';
  } catch {
    /* /etc/os-release unreadable — fall through to the file-probe below */
  }
  try {
    if (fs.existsSync('/etc/debian_version')) return 'deb';
    if (fs.existsSync('/etc/redhat-release')) return 'rpm';
  } catch {
    /* probe failed — treat the distro family as unknown */
  }
  return 'unknown';
}

// ─── Version probing ──────────────────────────────────────────────────────────

const execFileAsync = promisify(execFile);

/**
 * Run a probe WITHOUT blocking the Electron main thread.
 *
 * This used to be `spawnSync`. It must never go back: the main process runs the
 * window compositor, IPC and input handling on this one thread, so a synchronous
 * child process freezes the whole app for its full duration — which is exactly
 * the startup freeze in #88 (`biorouter doctor` alone measured 3.45 s warm, and
 * the timeout budget was 15 s). `execFile` returns a promise and keeps the event
 * loop turning.
 */
export interface ProbeResult {
  stdout: string;
  stderr: string;
  ok: boolean;
  code: number | null;
  timedOut: boolean;
  error: string | null;
}

/**
 * Whether to route this invocation through a shell.
 *
 * On Windows a BARE command name has to go through `cmd.exe`, or `npm`/`npx`
 * (which are `.cmd` wrappers, not `.exe`s) cannot be found at all. An ABSOLUTE
 * PATH must not: under a shell the path is re-parsed as a command line, so
 * `C:\Program Files\Biorouter\resources\bin\biorouter.exe` splits at the
 * space and the probe fails on exactly the machines the bundled CLI ships to.
 *
 * Deciding from the command itself keeps every call site correct by default.
 */
function needsShell(cmd: string): boolean {
  if (process.platform !== 'win32') return false;
  return !/[\\/]/.test(cmd);
}

export async function runProbe(
  cmd: string,
  args: string[],
  timeoutMs = PROBE_TIMEOUT_MS,
  opts?: { cwd?: string }
): Promise<ProbeResult> {
  try {
    const { stdout, stderr } = await execFileAsync(cmd, args, {
      encoding: 'utf8',
      timeout: timeoutMs,
      env: SPAWN_ENV,
      shell: needsShell(cmd),
      // Every probe is a console program, and `needsShell` routes a bare name
      // through `cmd.exe`, which is one too. Without this the Electron main
      // process — which owns no console — makes Windows allocate a visible one
      // per probe, and the user watches black boxes flash (#368).
      windowsHide: true,
      maxBuffer: 8 * 1024 * 1024,
      ...(opts?.cwd ? { cwd: opts.cwd } : {}),
    });
    return {
      stdout: stdout ?? '',
      stderr: stderr ?? '',
      ok: true,
      code: 0,
      timedOut: false,
      error: null,
    };
  } catch (err) {
    const e = err as {
      stdout?: string;
      stderr?: string;
      code?: number | string;
      killed?: boolean;
      signal?: string;
      message?: string;
    };
    // `execFile` reports a timeout by killing the child, so the signal is the
    // only reliable marker — the exit code is null in that case.
    const timedOut = !!e?.killed && (e?.signal === 'SIGTERM' || e?.code === undefined);
    let exitCode = typeof e?.code === 'number' ? e.code : null;
    if (exitCode === 1 && needsShell(cmd)) {
      try {
        await execFileAsync('where.exe', [cmd], {
          encoding: 'utf8',
          env: SPAWN_ENV,
          windowsHide: true,
          maxBuffer: 1024 * 1024,
        });
      } catch {
        exitCode = null;
      }
    }
    return {
      stdout: e?.stdout ?? '',
      stderr: e?.stderr ?? '',
      ok: false,
      code: exitCode,
      timedOut,
      error: e?.message ?? null,
    };
  }
}

async function probeVersion(cmd: string, args: string[]): Promise<string | null> {
  const { stdout, ok } = await runProbe(cmd, args);
  if (ok && stdout.trim()) {
    return stdout.trim().split('\n')[0].trim();
  }
  return null;
}

// ─── Install command builders ─────────────────────────────────────────────────

type NativeDep = 'git' | 'python' | 'uv' | 'npm' | 'aws' | 'llama-server' | 'rust';

function buildInstallInfo(
  dep: NativeDep,
  distro: LinuxDistro
): { cmd: string; requiresSudo: boolean; downloadUrl: string } {
  const platform = process.platform;
  // Non-interactive winget flags — without them winget blocks on its first-run
  // source/package agreement prompt, which fails in this non-interactive shell.
  const WG = '--accept-package-agreements --accept-source-agreements --disable-interactivity';

  if (platform === 'darwin') {
    switch (dep) {
      case 'git':
        return {
          cmd: 'xcode-select --install',
          requiresSudo: false,
          downloadUrl: 'https://git-scm.com/download/mac',
        };
      case 'python':
        return {
          cmd: 'brew install python3',
          requiresSudo: false,
          downloadUrl: 'https://www.python.org/downloads/',
        };
      case 'uv':
        return {
          cmd: 'curl -LsSf https://astral.sh/uv/install.sh | sh',
          requiresSudo: false,
          downloadUrl: 'https://docs.astral.sh/uv/getting-started/installation/',
        };
      case 'npm':
        return {
          cmd: 'brew install node',
          requiresSudo: false,
          downloadUrl: 'https://nodejs.org/en/download',
        };
      case 'aws':
        return {
          cmd: 'brew install awscli',
          requiresSudo: false,
          downloadUrl: 'http://biorouter.ucsf.edu/docs',
        };
      case 'llama-server':
        return {
          cmd: 'brew install llama.cpp',
          requiresSudo: false,
          downloadUrl: 'https://github.com/ggml-org/llama.cpp/releases',
        };
      case 'rust':
        // `sh -s -- -y`: the piped installer has no TTY and aborts without -y.
        return {
          cmd: "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
          requiresSudo: false,
          downloadUrl: 'https://rustup.rs',
        };
    }
  }

  if (platform === 'win32') {
    switch (dep) {
      case 'git':
        return {
          cmd: `winget install --id Git.Git -e --source winget ${WG}`,
          requiresSudo: false,
          downloadUrl: 'https://git-scm.com/download/win',
        };
      case 'python':
        return {
          cmd: `winget install --id Python.Python.3 -e --source winget ${WG}`,
          requiresSudo: false,
          downloadUrl: 'https://www.python.org/downloads/',
        };
      case 'uv':
        return {
          cmd: 'powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"',
          requiresSudo: false,
          downloadUrl: 'https://docs.astral.sh/uv/getting-started/installation/',
        };
      case 'npm':
        return {
          cmd: `winget install --id OpenJS.NodeJS -e --source winget ${WG}`,
          requiresSudo: false,
          downloadUrl: 'https://nodejs.org/en/download',
        };
      case 'aws':
        return {
          cmd: `winget install Amazon.AWSCLI ${WG}`,
          requiresSudo: false,
          downloadUrl: 'http://biorouter.ucsf.edu/docs',
        };
      case 'llama-server':
        return {
          cmd: `winget install --id ggml.llamacpp -e --source winget ${WG}`,
          requiresSudo: false,
          downloadUrl: 'https://github.com/ggml-org/llama.cpp/releases',
        };
      case 'rust':
        return {
          cmd: `winget install --id Rustlang.Rustup -e --source winget ${WG}`,
          requiresSudo: false,
          downloadUrl: 'https://rustup.rs',
        };
    }
  }

  // Linux
  if (dep === 'llama-server') {
    // No standard distro package; prebuilt binaries on GitHub releases.
    return {
      cmd: '',
      requiresSudo: false,
      downloadUrl: 'https://github.com/ggml-org/llama.cpp/releases',
    };
  }

  if (dep === 'rust') {
    return {
      cmd: "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
      requiresSudo: false,
      downloadUrl: 'https://rustup.rs',
    };
  }

  if (dep === 'uv') {
    return {
      cmd: 'curl -LsSf https://astral.sh/uv/install.sh | sh',
      requiresSudo: false,
      downloadUrl: 'https://docs.astral.sh/uv/getting-started/installation/',
    };
  }

  if (dep === 'aws') {
    if (distro === 'deb') {
      return {
        cmd: 'sudo apt-get install -y awscli',
        requiresSudo: true,
        downloadUrl: 'http://biorouter.ucsf.edu/docs',
      };
    }
    if (distro === 'rpm') {
      return {
        cmd: 'sudo dnf install -y awscli',
        requiresSudo: true,
        downloadUrl: 'http://biorouter.ucsf.edu/docs',
      };
    }
    return {
      cmd: 'pip install awscli',
      requiresSudo: false,
      downloadUrl: 'http://biorouter.ucsf.edu/docs',
    };
  }

  if (distro === 'deb') {
    const pkgs: Record<string, string> = { git: 'git', python: 'python3', npm: 'nodejs npm' };
    return {
      cmd: `sudo apt-get install -y ${pkgs[dep] ?? dep}`,
      requiresSudo: true,
      downloadUrl: '',
    };
  }

  if (distro === 'rpm') {
    const pkgs: Record<string, string> = { git: 'git', python: 'python3', npm: 'nodejs npm' };
    return {
      cmd: `sudo dnf install -y ${pkgs[dep] ?? dep}`,
      requiresSudo: true,
      downloadUrl: '',
    };
  }

  return {
    cmd: `# Install ${dep} via your system package manager`,
    requiresSudo: false,
    downloadUrl: '',
  };
}

// ─── Dependency check ─────────────────────────────────────────────────────────

let _distro: LinuxDistro | null = null;
function getLinuxDistro(): LinuxDistro {
  if (_distro === null) _distro = process.platform === 'linux' ? detectLinuxDistro() : 'unknown';
  return _distro;
}

/**
 * Single source of truth: ask the bundled `biorouter` CLI (which reads the Rust
 * `biorouter::system` spec) for the dependency status. Falls back to the native
 * probes below if the CLI isn't available (e.g. dev build) or errors, so the
 * desktop never loses its dependency check.
 */
let _cache: { at: number; deps: DependencyInfo[] } | null = null;
let _inFlight: Promise<DependencyInfo[]> | null = null;

/**
 * Bumped whenever a caller invalidates. A probe carries the generation it started
 * in and only writes the cache if that is still current — otherwise a probe that
 * a `force` explicitly superseded would land afterwards and reinstate the very
 * snapshot the force was asking to discard, stamped with a fresh timestamp, for
 * another full TTL.
 */
let _generation = 0;

/** Drop the memoised result so the next check re-probes (after an install). */
export function invalidateDependencyCache(): void {
  _cache = null;
  // Also disown any probe already running: it was started against the old state.
  _generation += 1;
  _inFlight = null;
}

/**
 * Async by construction — there is deliberately no synchronous variant.
 *
 * Concurrent callers share one in-flight probe rather than each spawning their
 * own `biorouter doctor`; a fresh result is reused for `DEPENDENCY_CACHE_TTL_MS`.
 */
export function checkAllDependencies(opts?: { force?: boolean }): Promise<DependencyInfo[]> {
  if (opts?.force) {
    invalidateDependencyCache();
  } else {
    if (_cache && Date.now() - _cache.at < DEPENDENCY_CACHE_TTL_MS) {
      return Promise.resolve(_cache.deps);
    }
    if (_inFlight) return _inFlight;
  }

  const generation = _generation;
  const probe = (async () => {
    const deps = (await checkViaBundledCli()) ?? (await checkNativeDependencies());
    if (generation === _generation) _cache = { at: Date.now(), deps };
    return deps;
  })().finally(() => {
    // Only clear the slot if it still holds THIS probe. Clearing unconditionally
    // let a superseded probe's completion deregister the newer one, so the next
    // caller started a third `biorouter doctor` instead of joining the running
    // one — defeating the sharing this block exists to provide.
    if (_inFlight === probe) _inFlight = null;
  });

  _inFlight = probe;
  return probe;
}

async function checkViaBundledCli(): Promise<DependencyInfo[] | null> {
  try {
    const cli = getBiorouterCliBinaryPath(app);
    const res = await runProbe(
      cli,
      ['doctor', '--format', 'json', '--no-update'],
      DOCTOR_TIMEOUT_MS
    );
    if (!res.ok || !res.stdout) return null;
    const parsed = JSON.parse(res.stdout) as {
      dependencies?: Array<{
        name: string;
        display_name?: string;
        version?: string | null;
        installed?: boolean;
        timed_out?: boolean;
        install_command?: string | null;
        requires_sudo?: boolean;
        download_url?: string | null;
        required?: boolean;
      }>;
    };
    const deps = parsed?.dependencies;
    if (!Array.isArray(deps) || deps.length === 0) return null;
    return deps.map((d) => ({
      name: String(d.name),
      displayName: String(d.display_name ?? d.name),
      version: d.version ?? null,
      installed: !!d.installed,
      // A probe that timed out did not disprove the tool. Dropping this made the
      // desktop offer to install software the machine may already have.
      timedOut: !!d.timed_out,
      installCmd: d.install_command ?? '',
      requiresSudo: !!d.requires_sudo,
      downloadUrl: d.download_url ?? '',
      required: d.required ?? true,
    }));
  } catch (err) {
    log.warn('[DependencyChecker] bundled CLI check unavailable, using native probes:', err);
    return null;
  }
}

// Resolve the bundled llama-server that ships next to the CLI binary, mirroring
// the Rust sidecar's `find_binary` (`<bin dir>/llamacpp/llama-server`). Used so
// the native fallback agrees with `biorouter doctor` that a bundled server
// counts as installed even when nothing is on PATH.
function bundledLlamaServerPath(): string | null {
  try {
    const exeName = process.platform === 'win32' ? 'llama-server.exe' : 'llama-server';
    const dir = path.dirname(getBiorouterCliBinaryPath(app));
    const candidate = path.join(dir, 'llamacpp', exeName);
    return fs.existsSync(candidate) ? candidate : null;
  } catch {
    return null;
  }
}

async function checkNativeDependencies(): Promise<DependencyInfo[]> {
  const distro = getLinuxDistro();

  const checks: Array<{
    name: NativeDep;
    displayName: string;
    probes: Array<[string, string[]]>;
    required?: boolean;
  }> = [
    {
      name: 'git',
      displayName: 'Git',
      probes: [['git', ['--version']]],
    },
    {
      name: 'python',
      displayName: 'Python',
      // Probe python3 first, then python (Windows often uses python)
      probes: [
        ['python3', ['--version']],
        ['python', ['--version']],
      ],
    },
    {
      name: 'uv',
      displayName: 'uv (Python package manager)',
      probes: [['uv', ['--version']]],
    },
    {
      name: 'npm',
      displayName: 'npm (Node.js)',
      probes: [['npm', ['--version']]],
    },
    {
      name: 'aws' as const,
      displayName: 'AWS CLI (optional)',
      probes: [['aws', ['--version']]],
    },
    {
      name: 'llama-server',
      displayName: 'llama-server (local models)',
      probes: [['llama-server', ['--version']]],
      required: false,
    },
    {
      name: 'rust',
      displayName: 'Rust toolchain (rustc)',
      // Health probe (-vV), matching the Rust spec: exits non-zero on a broken
      // toolchain so it reads as unavailable rather than merely "present".
      probes: [['rustc', ['-vV']]],
      required: false,
    },
  ];

  // On Windows, uv manages its own Python runtime via uvx — system Python is not
  // required for extensions. If uv is present, mark Python as satisfied.
  const uvVersion = process.platform === 'win32' ? await probeVersion('uv', ['--version']) : null;

  // Probed concurrently: these are independent subprocesses, and running them in
  // sequence made the fallback path cost the SUM of every probe's timeout.
  return Promise.all(
    checks.map(async ({ name, displayName, probes, required }) => {
      let version: string | null = null;

      if (name === 'python' && uvVersion !== null) {
        // uv bundles Python internally; no separate system Python needed on Windows
        version = `managed by uv ${uvVersion}`;
      } else {
        for (const [cmd, args] of probes) {
          version = await probeVersion(cmd, args);
          if (version) break;
        }
      }

      // llama-server usually isn't on PATH — the desktop app bundles it next to
      // the CLI. Fall back to probing the bundled copy (matches `biorouter doctor`).
      if (version === null && name === 'llama-server') {
        const bundled = bundledLlamaServerPath();
        if (bundled) version = await probeVersion(bundled, ['--version']);
      }

      const { cmd, requiresSudo, downloadUrl } = buildInstallInfo(name, distro);
      return {
        name,
        displayName,
        version,
        installed: version !== null,
        installCmd: cmd,
        requiresSudo,
        downloadUrl,
        required: required ?? true,
      };
    })
  );
}

// ─── Install runner ───────────────────────────────────────────────────────────

type SendFn = (event: DependencyEvent) => void;

function runInstallCommand(dep: string, cmd: string, send: SendFn): void {
  if (!cmd || !cmd.trim()) {
    send({ type: 'install-error', dep, error: 'No automated installer. Use the Download link.' });
    return;
  }
  send({ type: 'install-start', dep });

  let child: ReturnType<typeof spawn>;

  if (process.platform === 'win32') {
    // On Windows, run via cmd.exe /c so pipes and .cmd wrappers work
    child = spawn('cmd.exe', ['/c', cmd], {
      env: SPAWN_ENV,
      shell: false,
      // The installer's output is streamed into the modal, so the console this
      // would otherwise pop up shows the user nothing they are not already
      // being shown — it only steals their focus mid-install.
      windowsHide: true,
    });
  } else {
    // macOS/Linux: run via sh -c so pipes (curl | sh) work
    child = spawn('sh', ['-c', cmd], {
      env: SPAWN_ENV,
      shell: false,
      windowsHide: true,
    });
  }

  child.stdout?.on('data', (chunk: Buffer) => {
    send({ type: 'install-output', dep, output: chunk.toString() });
  });
  child.stderr?.on('data', (chunk: Buffer) => {
    send({ type: 'install-output', dep, output: chunk.toString() });
  });

  child.on('close', (code) => {
    void (async () => {
      if (code === 0) {
        // Re-probe to get the version. `force` because the install just changed
        // the very state the cache is holding.
        const info = (await checkAllDependencies({ force: true })).find((d) => d.name === dep);
        if (info?.installed) {
          send({
            type: 'install-done',
            dep,
            installed: true,
            version: info.version ?? null,
          });
          return;
        }
        // Exit code 0 but the tool still isn't detectable. That is a failure the
        // user can act on, so report it as one and carry the context a debugging
        // session would need.
        send({
          type: 'install-error',
          dep,
          error:
            'The installer finished successfully but the tool is still not detectable on PATH.',
          installed: false,
        });
        return;
      }
      send({ type: 'install-error', dep, error: `Process exited with code ${code}` });
    })();
  });

  child.on('error', (err) => {
    send({ type: 'install-error', dep, error: err.message });
  });
}

// ─── IPC registration ─────────────────────────────────────────────────────────

let _registered = false;

export function registerDependencyIpcHandlers(): void {
  if (_registered) return;
  _registered = true;

  ipcMain.handle('dep:check', async (_event, opts?: { force?: boolean }) => {
    try {
      return await checkAllDependencies({ force: opts?.force });
    } catch (err) {
      log.error('[DependencyChecker] check error:', err);
      return [];
    }
  });

  ipcMain.handle('dep:install', async (_event, dep: string) => {
    const info = (await checkAllDependencies()).find((d) => d.name === dep);
    if (!info) return { error: `Unknown dependency: ${dep}` };

    const win = BrowserWindow.fromWebContents(_event.sender);
    if (!win) return { error: 'No window found' };

    const send: SendFn = (payload) => {
      if (!win.isDestroyed()) win.webContents.send('dependency-event', payload);
    };

    runInstallCommand(dep, info.installCmd, send);
    return { started: true };
  });

  // Environment snapshot for a debugging session: what the app can actually see,
  // which is routinely different from what the user's login shell sees.
  ipcMain.handle('dep:environment', async () => {
    const [shellPath, unameOut] = await Promise.all([
      runProbe(process.platform === 'win32' ? 'where' : 'which', ['uv']).catch(() => null),
      process.platform === 'win32'
        ? Promise.resolve(null)
        : runProbe('uname', ['-a']).catch(() => null),
    ]);
    return {
      platform: process.platform,
      arch: process.arch,
      appVersion: app.getVersion(),
      osRelease: (unameOut?.stdout ?? '').trim() || os.release(),
      augmentedPath: AUGMENTED_PATH,
      inheritedPath: process.env.PATH ?? '',
      homedir: os.homedir(),
      uvResolvesTo: (shellPath?.stdout ?? '').trim() || null,
    };
  });
}

// ─── Startup orchestrator ─────────────────────────────────────────────────────

function broadcastResults(deps: DependencyInfo[]): void {
  const payload: DependencyEvent = { type: 'check-results', deps };
  BrowserWindow.getAllWindows().forEach((win) => {
    if (!win.isDestroyed()) win.webContents.send('dependency-event', payload);
  });
}

/**
 * Startup dependency check.
 *
 * `delayMs` exists so the caller can stagger this against the other startup
 * timers; the probe itself no longer blocks the main thread, so the delay is
 * only about not competing with the renderer's first paint.
 */
export function setupDependencyChecker(delayMs = 4000): void {
  setTimeout(() => {
    void (async () => {
      try {
        const deps = await checkAllDependencies();
        // A timed-out probe did not disprove the tool, so it is not "missing".
        const missing = deps.filter((d) => !d.installed && !d.timedOut);
        if (missing.length === 0) {
          log.info('[DependencyChecker] All dependencies present.');
          return;
        }
        log.warn(
          '[DependencyChecker] Missing deps:',
          missing.map((d) => d.name)
        );
        broadcastResults(deps);
      } catch (err) {
        log.error('[DependencyChecker] startup check error:', err);
      }
    })();
  }, delayMs);
}

export function triggerDependencyCheck(): void {
  void (async () => {
    try {
      broadcastResults(await checkAllDependencies({ force: true }));
    } catch (err) {
      log.error('[DependencyChecker] manual check error:', err);
    }
  })();
}
