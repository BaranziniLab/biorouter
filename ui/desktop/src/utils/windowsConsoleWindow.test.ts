/**
 * Does a child process we start put a black console window on the user's screen?
 *
 * # What this measures, and why it has to run on Windows
 *
 * The Electron main process is a GUI-subsystem process, so it owns no console.
 * When a console-less process starts a CONSOLE-subsystem child — `cmd.exe`,
 * `where.exe`, `taskkill.exe`, a `.cmd` shim like `npx`, `uv`, `node`, `git` —
 * Windows allocates a **new console** for that child, and unless the creation
 * asked for `CREATE_NO_WINDOW` that console is **visible**. Probes are short, so
 * the user sees a black box flash open and shut. That is issue #368: "频繁跳黑框".
 *
 * Node's knob is the spawn option `windowsHide`, whose default is `false`.
 *
 * Nothing about that is observable from macOS or Linux, and a source-level check
 * can only say the option is *written* — not that it *works* on the Node that
 * Electron actually ships. So this file asks Windows itself, and pairs every
 * claim with its control:
 *
 *   * the control spawn (no `windowsHide`) must report a **visible** console —
 *     if it does not, the instrument is broken and every other assertion here is
 *     vacuous, so that is a failure, not a skip;
 *   * the same spawn with `windowsHide: true` must report a **hidden** console;
 *   * a grandchild behind `cmd.exe` must stay hidden too, because that is the
 *     shape `runProbe` takes on Windows for a bare command name;
 *   * and `runProbe` itself — the real function the app calls, not a copy of it
 *     — must report a hidden console.
 *
 * # How the measurement works
 *
 * The child asks about *itself*: `GetConsoleWindow()` returns the HWND of the
 * console it is attached to (owned by `conhost.exe` since Windows 7, which is
 * why the handle is valid across the process boundary), and `IsWindowVisible()`
 * answers whether that window is on screen. `CREATE_NO_WINDOW` still gives the
 * child a real console — it is the *window* that is not shown — so visibility,
 * not the presence of a handle, is the discriminator.
 *
 * The probe is passed as `-EncodedCommand` (base64 UTF-16LE) deliberately: it is
 * a single token with no spaces or quotes, so it survives being re-parsed by
 * `cmd.exe` on the shell path, where a script path with a space would not.
 */
import { describe, expect, it, vi } from 'vitest';
import { spawn, type SpawnOptions } from 'node:child_process';

vi.mock('electron', () => ({
  app: { getVersion: () => '0.0.0-test', getPath: () => process.cwd() },
  ipcMain: { handle: vi.fn() },
  BrowserWindow: { getAllWindows: () => [], fromWebContents: () => null },
}));
vi.mock('./logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() },
}));

const WINDOWS = process.platform === 'win32';

/**
 * Prints exactly one of `CONSOLE=visible`, `CONSOLE=hidden`, `CONSOLE=none`.
 *
 * `Add-Type -MemberDefinition` is Windows PowerShell 5.1 vocabulary and is
 * present on every Windows install; it is not one of the `Add-Type` switches
 * PowerShell 7 dropped.
 */
const PROBE_SCRIPT = `
$signature = @'
[DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr hWnd);
'@
$probe = Add-Type -MemberDefinition $signature -Name ConsoleProbe -Namespace Biorouter -PassThru
$handle = $probe::GetConsoleWindow()
if ($handle -eq [System.IntPtr]::Zero) { Write-Output 'CONSOLE=none' }
elseif ($probe::IsWindowVisible($handle)) { Write-Output 'CONSOLE=visible' }
else { Write-Output 'CONSOLE=hidden' }
`;

const ENCODED_PROBE = Buffer.from(PROBE_SCRIPT, 'utf16le').toString('base64');
const PROBE_ARGS = ['-NoProfile', '-NonInteractive', '-EncodedCommand', ENCODED_PROBE];

type ConsoleState = 'visible' | 'hidden' | 'none';

function readConsoleState(output: string): ConsoleState {
  const match = /CONSOLE=(visible|hidden|none)/.exec(output);
  if (!match) {
    throw new Error(
      `the probe printed no verdict, so nothing was measured. Output was:\n${output}`
    );
  }
  return match[1] as ConsoleState;
}

/** Spawn the probe directly, with whatever options the case is testing. */
function spawnProbe(command: string, args: string[], options: SpawnOptions): Promise<ConsoleState> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'], ...options });
    let stdout = '';
    let stderr = '';
    child.stdout?.on('data', (chunk: Buffer) => (stdout += chunk.toString()));
    child.stderr?.on('data', (chunk: Buffer) => (stderr += chunk.toString()));
    child.on('error', reject);
    child.on('close', () => {
      try {
        resolve(readConsoleState(stdout + stderr));
      } catch (error) {
        reject(error);
      }
    });
  });
}

describe.runIf(WINDOWS)('a child process and the console window it shows', () => {
  it('CONTROL: spawning without windowsHide shows a console window', async () => {
    // If this ever reports `hidden`, the rest of this file proves nothing —
    // every assertion below would pass on a machine where no spawn can show a
    // window at all. So the control is an assertion, never a skip.
    await expect(spawnProbe('powershell.exe', PROBE_ARGS, {})).resolves.toBe('visible');
  }, 60_000);

  it('windowsHide: true hides it', async () => {
    await expect(spawnProbe('powershell.exe', PROBE_ARGS, { windowsHide: true })).resolves.toBe(
      'hidden'
    );
  }, 60_000);

  it('a grandchild behind cmd.exe stays hidden', async () => {
    // `runProbe` routes a BARE command name through `cmd.exe` on Windows, so the
    // process that would own the window is the shell, and the probe is its
    // child. The child inherits the shell's console, so hiding the shell is
    // enough — this is the assertion that says so.
    await expect(
      spawnProbe('cmd.exe', ['/d', '/s', '/c', 'powershell.exe', ...PROBE_ARGS], {
        windowsHide: true,
      })
    ).resolves.toBe('hidden');
  }, 60_000);

  it('CONTROL: the same grandchild without windowsHide shows a window', async () => {
    await expect(
      spawnProbe('cmd.exe', ['/d', '/s', '/c', 'powershell.exe', ...PROBE_ARGS], {})
    ).resolves.toBe('visible');
  }, 60_000);

  it('runProbe — the function the app really calls — shows no window', async () => {
    const { runProbe } = await import('./dependencyChecker');
    // A bare command name, which is what makes `needsShell()` route this through
    // `cmd.exe`: the hardest case, and the one the app takes for `uv`/`npx`.
    const result = await runProbe('powershell', PROBE_ARGS, 60_000);
    expect(readConsoleState(result.stdout + result.stderr)).toBe('hidden');
  }, 60_000);

  it('runProbe with an absolute path shows no window either', async () => {
    const { runProbe } = await import('./dependencyChecker');
    const absolute = `${process.env.SystemRoot ?? 'C:\\Windows'}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe`;
    const result = await runProbe(absolute, PROBE_ARGS, 60_000);
    expect(readConsoleState(result.stdout + result.stderr)).toBe('hidden');
  }, 60_000);
});
