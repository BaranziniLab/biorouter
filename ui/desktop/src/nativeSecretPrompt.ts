import { spawn, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';

let active = false;
/** The dialog process of the open prompt, so quitting the app can close it (no orphaned dialog). */
let activeChild: ChildProcess | undefined;

/** Most stderr kept from the dialog helper. Only the answer goes to stdout; stderr is diagnostics. */
const STDERR_LIMIT = 4096;

export type NativePromptOutcome =
  | { kind: 'answer'; answer: string }
  | { kind: 'cancelled' }
  | { kind: 'failed'; message: string };

/**
 * How a finished dialog helper maps onto an answer, a cancellation, or a dialog that never
 * opened. Only Linux can tell the last two apart: zenity exits 1 when the person clicks Cancel
 * or closes the window, but also when GTK cannot open the display, and exits 255 when it rejects
 * its own arguments (1.92.0 hit that with non-ASCII prompt text in a C locale). Reporting those
 * as "cancelled" left the person with no idea what to fix. macOS and Windows keep the old
 * mapping: any non-zero exit is a cancellation.
 */
export function nativePromptOutcome(input: {
  platform: typeof process.platform;
  code: number | null;
  answer: string;
  stderr: string;
}): NativePromptOutcome {
  const { platform, code, answer, stderr } = input;
  if (code === 0 && answer) return { kind: 'answer', answer };
  if (platform === 'darwin' || platform === 'win32') return { kind: 'cancelled' };
  // Exit 0 with nothing typed, and a helper stopped by a signal (the app quitting), are no answer.
  if (code === 0 || code === null) return { kind: 'cancelled' };
  if (code === 1 && !/cannot open display/i.test(stderr)) return { kind: 'cancelled' };
  const detail = stderr
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.length > 0)
    ?.slice(0, 200);
  return {
    kind: 'failed',
    message: `Biorouter could not open its secure password dialog (zenity exited with code ${code}${detail ? `: ${detail}` : ''}). Check that zenity is installed and can open a window on this desktop, then reopen the app.`,
  };
}

/** Close the open prompt's dialog, if any. Called when the app quits so no dialog outlives it. */
export function closeNativeSecretPrompt(): void {
  const child = activeChild;
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  try {
    child.kill();
  } catch {
    // The dialog already exited; nothing to close.
  }
}

/** OS-owned password field: the application renderer never receives the answer.
 * Buffers are cleared; JavaScript strings remain subject to garbage collection
 * and cannot promise deterministic memory erasure. Never persist or log them.
 */
export async function promptNativeSecret(
  title: string,
  message: string
): Promise<string | undefined> {
  if (active) throw new Error('Finish the open secure prompt before starting another.');
  const appleQuote = (value: string) =>
    `"${value
      .replace(/\\/g, '\\\\')
      .replace(/"/g, '\\"')
      .replace(/[\r\n]/g, ' ')}"`;
  const psQuote = (value: string) => `'${value.replace(/'/g, "''").replace(/[\r\n]/g, ' ')}'`;
  let program: string;
  let args: string[];
  if (process.platform === 'darwin') {
    program = '/usr/bin/osascript';
    args = [
      '-e',
      'tell current application to activate',
      '-e',
      `text returned of (display dialog ${appleQuote(message)} with title ${appleQuote(title)} default answer "" with hidden answer buttons {"Cancel", "Continue"} default button "Continue" cancel button "Cancel")`,
    ];
  } else if (process.platform === 'win32') {
    program = 'powershell.exe';
    const script = `[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding($false); Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $f=New-Object System.Windows.Forms.Form; $f.Text=${psQuote(title)}; $f.ClientSize=New-Object System.Drawing.Size(520,180); $f.StartPosition='CenterScreen'; $f.FormBorderStyle='FixedDialog'; $f.MinimizeBox=$false; $f.MaximizeBox=$false; $l=New-Object System.Windows.Forms.Label; $l.Text=${psQuote(message)}; $l.SetBounds(16,16,488,70); $t=New-Object System.Windows.Forms.TextBox; $t.UseSystemPasswordChar=$true; $t.SetBounds(16,88,488,24); $ok=New-Object System.Windows.Forms.Button; $ok.Text='Continue'; $ok.SetBounds(320,130,88,28); $ok.DialogResult='OK'; $cancel=New-Object System.Windows.Forms.Button; $cancel.Text='Cancel'; $cancel.SetBounds(416,130,88,28); $cancel.DialogResult='Cancel'; $f.Controls.AddRange(@($l,$t,$ok,$cancel)); $f.AcceptButton=$ok; $f.CancelButton=$cancel; $f.Add_Shown({$t.Focus()}); if($f.ShowDialog() -eq 'OK'){[Console]::Write($t.Text)}; $t.Clear(); $f.Dispose()`;
    args = [
      '-NoProfile',
      '-NonInteractive',
      '-STA',
      '-EncodedCommand',
      Buffer.from(script, 'utf16le').toString('base64'),
    ];
  } else {
    const helper = ['/usr/bin/zenity', '/bin/zenity'].find((candidate) => {
      try {
        fs.accessSync(candidate, fs.constants.X_OK);
        return fs.statSync(candidate).isFile();
      } catch {
        return false;
      }
    });
    if (!helper)
      throw new Error(
        "The Linux desktop needs Zenity for its secure password dialog. DEB and RPM packages declare this dependency. AppImage users must make their distribution's zenity package available before starting or attaching to a shared daemon. Biorouter does not install it automatically."
      );
    program = helper;
    args = ['--password', `--title=${title}`, `--text=${message}`];
  }
  const linux = process.platform !== 'darwin' && process.platform !== 'win32';
  active = true;
  try {
    return await new Promise<string | undefined>((resolve, reject) => {
      const env = Object.fromEntries(
        Object.entries(process.env).filter(
          ([key]) =>
            !/^(BIOROUTER|GOOSE)_.+(SECRET|TOKEN|PASSWORD|PASSWD|PASSPHRASE|PASSCODE|CREDENTIAL|KEY|DIGEST)/i.test(
              key
            ) && !/^(BIOROUTER|GOOSE)_SERVER__/i.test(key)
        )
      );
      const child = spawn(program, args, {
        env,
        stdio: ['ignore', 'pipe', linux ? 'pipe' : 'ignore'],
        windowsHide: true,
      });
      activeChild = child;
      const chunks: Buffer[] = [];
      let stderr = '';
      child.stderr?.on('data', (chunk: Buffer) => {
        if (stderr.length < STDERR_LIMIT)
          stderr += chunk.toString('utf8').slice(0, STDERR_LIMIT - stderr.length);
      });
      let size = 0;
      let exceeded = false;
      let timedOut = false;
      const timer = setTimeout(() => {
        timedOut = true;
        child.kill();
      }, 180000);
      child.stdout?.on('data', (chunk: Buffer) => {
        size += chunk.length;
        if (size > 4098) {
          exceeded = true;
          chunk.fill(0);
          child.kill();
          return;
        }
        chunks.push(chunk);
      });
      child.once('error', () => {
        clearTimeout(timer);
        chunks.forEach((chunk) => chunk.fill(0));
        reject(
          new Error(
            linux
              ? "Biorouter could not start zenity for its secure password dialog. Install your distribution's zenity package, then reopen the app."
              : 'The native secure prompt is unavailable. Install the platform password-dialog helper or use the CLI secure prompt.'
          )
        );
      });
      child.once('close', (code) => {
        clearTimeout(timer);
        const bytes = Buffer.concat(chunks);
        chunks.forEach((chunk) => chunk.fill(0));
        let answer = bytes.toString('utf8');
        bytes.fill(0);
        if (process.platform !== 'win32') answer = answer.replace(/\r?\n$/, '');
        if (exceeded || Buffer.byteLength(answer, 'utf8') > 4096)
          reject(new Error('The secure response exceeds the allowed length.'));
        else if (timedOut)
          reject(
            new Error(
              'The native secure prompt timed out after three minutes. Reopen the app and complete the password dialog to continue.'
            )
          );
        else {
          const outcome = nativePromptOutcome({
            platform: process.platform,
            code,
            answer,
            stderr,
          });
          if (outcome.kind === 'answer') resolve(outcome.answer);
          else if (outcome.kind === 'cancelled') resolve(undefined);
          else reject(new Error(outcome.message));
        }
      });
    });
  } finally {
    active = false;
    activeChild = undefined;
  }
}
