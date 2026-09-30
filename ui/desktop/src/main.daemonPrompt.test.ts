import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * T3-SH-10. The "background service restarted" prompt and its failure report
 * called `dialog.showMessageBox` with no window. On macOS that is an app-modal
 * alert run on the main thread: the watchdog measured it blocked for 74 s and
 * 42 s, every window stopped painting, and the app looked hung until someone
 * found the prompt. Given a window it is a sheet and the app keeps running.
 *
 * Asserted against `main.ts`'s source, as `main.openPath.test.ts` is: nothing
 * can import `main.ts` under vitest.
 */
const main = readFileSync(join(__dirname, 'main.ts'), 'utf8');

function between(start: string, end: string): string {
  const from = main.indexOf(start);
  expect(from, `no ${start}`).toBeGreaterThan(-1);
  const to = main.indexOf(end, from);
  expect(to, `no ${end} after ${start}`).toBeGreaterThan(from);
  return main.slice(from, to);
}

describe('the reattach prompts belong to a window', () => {
  const controller = between('const daemonReattach = createDaemonReattachController({', '\n});\n');

  it('reports a failed reconnect through the window-attached prompt, never a bare app-modal alert', () => {
    const body = controller.slice(controller.indexOf('reportFailure:'));
    const next = body.indexOf('showWindowPrompt(');
    expect(next, 'reportFailure does not use showWindowPrompt').toBeGreaterThan(-1);
    expect(body.slice(0, next)).not.toContain('dialog.showMessageBox(');
    expect(controller).not.toMatch(/dialog\.showMessageBox\(\{/);
  });

  it('passes the parent window whenever one is showing', () => {
    const helper = between('function showWindowPrompt(', '\n}\n');
    expect(helper).toContain('dialog.showMessageBox(parent, options)');
    const parent = between('function promptParentWindow(', '\n}\n');
    expect(parent).toContain('BrowserWindow.getFocusedWindow()');
    // A sheet on a hidden window is a prompt nobody can see.
    expect(parent).toContain('isVisible()');
  });
});

/**
 * Biorouter 1.92.0 asked every person to invent an approval secret for the shared daemon, and to
 * type it on every launch and every reconnect. The app now mints the key itself and saves it in a
 * private file, so neither a first launch nor a reconnect asks anything. The only native secret
 * prompts left in `main.ts` are the Crew vault passphrase's.
 */
describe('the shared daemon asks nobody for a secret', () => {
  it('has no reconnect question: the controller is given no way to ask', () => {
    const controller = between(
      'const daemonReattach = createDaemonReattachController({',
      '\n});\n'
    );
    expect(controller).not.toMatch(/\bask:/);
    expect(controller.match(/showWindowPrompt\(/g)).toHaveLength(1);
  });

  it('calls promptNativeSecret only inside the Crew vault handler', () => {
    const vault = between("ipcMain.handle('crew:credentials'", '\n  });\n');
    const all = [...main.matchAll(/promptNativeSecret\(/g)].map((match) => match.index ?? -1);
    // One import line mentions the name without calling it.
    expect(all.length).toBeGreaterThan(0);
    const start = main.indexOf(vault);
    for (const index of all) {
      expect(index, 'a promptNativeSecret call outside the Crew vault handler').toBeGreaterThan(
        start
      );
      expect(index).toBeLessThan(start + vault.length);
    }
  });

  it('never mentions an approval secret, and never passes a prompt to the daemon starter', () => {
    expect(main).not.toMatch(/approval secret/i);
    expect(main).not.toMatch(/requestNewUserActionKey|requestUserActionKey/);
    expect(main).not.toMatch(/ApprovalSecret|developmentApprovalInput/);
  });
});
