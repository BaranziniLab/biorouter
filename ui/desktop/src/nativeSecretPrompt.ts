import { spawn } from 'node:child_process';
import fs from 'node:fs';

let active = false;

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
        'The Linux desktop needs Zenity for its secure password dialog. DEB and RPM packages declare this dependency. AppImage users must make their distribution’s zenity package available before starting or attaching to a shared daemon. BioRouter does not install it automatically.'
      );
    program = helper;
    args = ['--password', `--title=${title}`, `--text=${message}`];
  }
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
        stdio: ['ignore', 'pipe', 'ignore'],
        windowsHide: true,
      });
      const chunks: Buffer[] = [];
      let size = 0;
      let exceeded = false;
      let timedOut = false;
      const timer = setTimeout(() => {
        timedOut = true;
        child.kill();
      }, 180000);
      child.stdout.on('data', (chunk: Buffer) => {
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
            'The native secure prompt is unavailable. Install the platform password-dialog helper or use the CLI secure prompt.'
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
        else if (code !== 0 || !answer) resolve(undefined);
        else resolve(answer);
      });
    });
  } finally {
    active = false;
  }
}
