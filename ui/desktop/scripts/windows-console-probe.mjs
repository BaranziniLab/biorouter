/**
 * Ask a Windows child process whether IT has a visible console window.
 *
 * The child answers about itself: `GetConsoleWindow()` returns the HWND of the
 * console it is attached to — owned by `conhost.exe` since Windows 7, which is
 * why the handle is meaningful across the process boundary — and
 * `IsWindowVisible()` says whether that window is on screen.
 *
 * Three verdicts, and the difference between two of them cost a round of
 * measurement to learn. `visible` is the bug. `none` is what a successful
 * CREATE_NO_WINDOW actually produces — Microsoft's own words are "the console
 * handle for the application is not set", so `GetConsoleWindow()` returns NULL
 * and there is no window to ask about. `hidden` — a console that exists and is
 * not shown — was what this file originally expected, and real Windows never
 * produced it for a flagged spawn. Both `none` and `hidden` mean "no black box";
 * `none` is the stronger of the two, so accepting it is not a loosened
 * assertion.
 *
 * What the child is asked about is deliberately its OWN console rather than the
 * window list, because everything about the window list moves with the machine:
 * a conhost is allocated in cases that draw nothing, and the class is
 * `PseudoConsoleWindow` where the default terminal is Windows Terminal and
 * `ConsoleWindowClass` where it is not. An EnumWindows sweep therefore answers a
 * different question on every box. This one does not.
 *
 * The verdict goes to a FILE, not to stdout, because the stdio shape is one of
 * the things under test: libuv declines to pass `CREATE_NO_WINDOW` when any
 * stdio entry is an inherited fd, and a probe that could only report through a
 * pipe could never be run in the shape that matters.
 */
import { spawn } from 'node:child_process';
import { readFileSync, rmSync } from 'node:fs';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** PowerShell 5.1 vocabulary: present on every Windows install. */
function probeScript(outputPath) {
  return `
$signature = @'
[DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr hWnd);
'@
$probe = Add-Type -MemberDefinition $signature -Name ConsoleProbe -Namespace Biorouter -PassThru
$handle = $probe::GetConsoleWindow()
if ($handle -eq [System.IntPtr]::Zero) { $verdict = 'none' }
elseif ($probe::IsWindowVisible($handle)) { $verdict = 'visible' }
else { $verdict = 'hidden' }
Set-Content -LiteralPath '${outputPath.replace(/'/g, "''")}' -Value $verdict -Encoding ascii
Write-Output $verdict
`;
}

/**
 * `-EncodedCommand` deliberately: base64 UTF-16LE is a single token with no
 * spaces or quotes, so it survives `cmd.exe` re-parsing it on the shell path,
 * where a script path containing a space would not.
 */
export function probeArgs(outputPath) {
  const encoded = Buffer.from(probeScript(outputPath), 'utf16le').toString('base64');
  return ['-NoProfile', '-NonInteractive', '-EncodedCommand', encoded];
}

export function scratchDir() {
  return mkdtempSync(join(tmpdir(), 'br-console-probe-'));
}

export function readVerdict(outputPath, context) {
  let text;
  try {
    text = readFileSync(outputPath, 'utf8').trim();
  } catch (error) {
    throw new Error(
      `the probe wrote no verdict, so NOTHING was measured (${context}): ${error.message}`
    );
  }
  if (!['visible', 'hidden', 'none'].includes(text)) {
    throw new Error(`the probe wrote an unreadable verdict (${context}): ${JSON.stringify(text)}`);
  }
  return text;
}

/** Spawn the probe from THIS process with the given options, and read its verdict. */
export function askFromHere(command, buildArgs, options, label) {
  const dir = scratchDir();
  const outputPath = join(dir, 'verdict.txt');
  return new Promise((resolve, reject) => {
    const child = spawn(command, buildArgs(outputPath), options);
    child.on('error', reject);
    child.on('close', () => {
      try {
        resolve(readVerdict(outputPath, label));
      } catch (error) {
        reject(error);
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    });
  });
}
