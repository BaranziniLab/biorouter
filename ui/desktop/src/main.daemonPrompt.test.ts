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

  it('asks and reports through the window-attached prompt, never a bare app-modal alert', () => {
    for (const hook of ['ask:', 'reportFailure:']) {
      const body = controller.slice(controller.indexOf(hook));
      const next = body.indexOf('showWindowPrompt(');
      expect(next, `${hook} does not use showWindowPrompt`).toBeGreaterThan(-1);
      expect(body.slice(0, next)).not.toContain('dialog.showMessageBox(');
    }
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
