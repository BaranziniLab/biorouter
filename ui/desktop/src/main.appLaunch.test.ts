import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import ts from 'typescript';

/**
 * How the main process opens a built app in the browser and in the artifact panel (W2-HRD-1).
 *
 * Asserted against `main.ts`'s SOURCE, as `main.openPath.test.ts` does: nothing can import
 * `main.ts` under vitest. The decisions the handlers delegate to are covered behaviourally by
 * `utils/appLaunchLink.test.ts`, `utils/appBrowserLaunch.test.ts`,
 * `utils/managedAppPreviewPolicy.test.ts` and `utils/managedAppPreviewBackend.test.ts`; what is
 * pinned here is that the handlers really do delegate to them.
 */
const main = readFileSync(join(__dirname, 'main.ts'), 'utf8');
const mainSyntax = ts.createSourceFile('main.ts', main, ts.ScriptTarget.Latest, true);

function windowDaemonAttachment(): ts.CallExpression {
  const attachments: ts.CallExpression[] = [];
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      ts.isIdentifier(node.expression.expression) &&
      node.expression.expression.text === 'windowDaemons' &&
      node.expression.name.text === 'set'
    ) {
      attachments.push(node);
    }
    ts.forEachChild(node, visit);
  };
  visit(mainSyntax);
  expect(attachments).toHaveLength(1);
  return attachments[0];
}

/** The body of one `ipcMain.handle` registration, up to the next `ipcMain.` call. */
function ipcHandlerSource(channel: string): string {
  // `ipcMain.handle('x', …` or, wrapped by Prettier, `ipcMain.handle(\n  'x',`.
  const start = main.search(new RegExp(`ipcMain\\.handle\\(\\s*'${channel}'`));
  expect(start, `no handler registered for '${channel}'`).toBeGreaterThan(-1);
  const next = main.indexOf('ipcMain.', start + 1);
  return main.slice(start, next === -1 ? undefined : next);
}

/** The body of a top-level `const name = …` arrow function, up to its closing `};`. */
function constFunctionSource(name: string): string {
  const start = main.indexOf(`const ${name} = `);
  expect(start, `no function named ${name}`).toBeGreaterThan(-1);
  const end = main.indexOf('\n};\n', start);
  return main.slice(start, end === -1 ? undefined : end);
}

describe('opening an app in the browser', () => {
  const opener = constFunctionSource('openAppForWindow');

  it('routes a local app link to a fresh launch BEFORE the public-site refusal', () => {
    // The public-site path refuses every literal IP, and used to drop "Open in browser" on a
    // live app with nothing but a log line.
    const handler = ipcHandlerSource('open-external');
    const routed = handler.indexOf('localAppLinkId(url)');
    expect(routed).toBeGreaterThan(-1);
    expect(routed).toBeLessThan(handler.indexOf('openExternalBrowserNavigation(window, url)'));
    expect(handler.slice(routed)).toContain('await openAppForWindow(window, appId)');
    // The renderer is told why, rather than nothing.
    expect(handler).toContain('ok: false');
    expect(handler).toContain('message:');
  });

  it('opens Applications’ Launch through the same one function', () => {
    const handler = ipcHandlerSource('apps:open-in-browser');
    expect(handler).toContain('await openAppForWindow(owner, appId)');
    expect(handler).not.toContain('openAppInSystemBrowser(');
  });

  it('uses the secret the window was attached with, never the current settings', () => {
    // Settings ▸ External Backend saved under a running window named another daemon's secret.
    for (const source of [opener, ipcHandlerSource('apps:open-in-browser')]) {
      expect(source).not.toContain('getServerSecret(');
      expect(source).not.toContain('loadSettings(');
    }
    expect(opener).toContain('windowDaemons.get(window.id)');
    expect(opener).toContain('secretKey: daemon.secretKey');
    const attachment = windowDaemonAttachment();
    const owner = attachment.arguments[0];
    expect(ts.isPropertyAccessExpression(owner)).toBe(true);
    if (!ts.isPropertyAccessExpression(owner)) throw new Error('Missing attachment window id');
    expect(ts.isIdentifier(owner.expression) && owner.expression.text).toBe('mainWindow');
    expect(owner.name.text).toBe('id');

    const options = attachment.arguments[1];
    expect(ts.isObjectLiteralExpression(options)).toBe(true);
    if (!ts.isObjectLiteralExpression(options)) throw new Error('Missing captured daemon keys');
    const fields = new Map<string, ts.Expression>();
    for (const property of options.properties) {
      if (ts.isPropertyAssignment(property) && ts.isIdentifier(property.name)) {
        fields.set(property.name.text, property.initializer);
      } else if (ts.isShorthandPropertyAssignment(property)) {
        fields.set(property.name.text, property.name);
      } else {
        throw new Error('Attachment fields must be explicit, without spread overrides');
      }
    }
    for (const [field, captured] of [
      ['secretKey', 'serverSecret'],
      ['userActionKey', 'userActionKey'],
    ]) {
      const value = fields.get(field);
      expect(value && ts.isIdentifier(value) && value.text).toBe(captured);
    }
  });

  it('sends the browser to the daemon Biorouter runs, and opens the page in a web browser', () => {
    expect(opener).toContain('daemonListener: daemon.result.managed');
    // Not the `.html` handler, which on a developer's machine is often an editor.
    expect(opener).toContain('const browser = await findDefaultBrowser(opener);');
    expect(opener).toMatch(/openPage: \(page\) =>\s+openPageInBrowser\(page, browser, opener\)/);
    // A browser started on Windows is left running, never waited on and killed at a timeout.
    expect(opener).toContain('launch: launchFile');
  });

  it('offers the link to copy, after asking, for a browser that cannot read the page', () => {
    expect(opener).toMatch(
      /copyLink: browser\.kind === 'confined' \? appLinkToCopy\(window, appId\) : undefined/
    );
    const start = main.indexOf('const appLinkToCopy = ');
    expect(start).toBeGreaterThan(-1);
    const offer = main.slice(start, main.indexOf('\n});\n', start));
    expect(offer).toContain('dialog.showMessageBox(window,');
    expect(offer).toContain('return response === 1;');
    expect(offer).toContain('clipboard.writeText(link)');
  });
});

describe('the in-chat app preview on a shared daemon', () => {
  it('binds each window’s preview to the daemon’s own listener, not to the proxy', () => {
    const bind = constFunctionSource('bindWindowAppPreview');
    expect(bind).toContain('fetchDaemonAppOrigin(daemon.baseUrl, daemon.secretKey)');
    // No listener named, no backend: an app link is then an external page, never a preview
    // sent to the proxy, which refuses it.
    expect(bind).toContain('if (!appOrigin || window.isDestroyed()) return undefined;');
    expect(main).toContain('void bindWindowAppPreview(mainWindow);');
  });

  it('answers whether a link is an app once the window’s backend is bound', () => {
    const handler = ipcHandlerSource('embedded-browser:is-managed-app-url');
    expect(handler).toContain('await windowAppPreviewBackend(window)');
    // Only a link to an app waits; any other page is not one, and is answered at once.
    expect(handler.indexOf('!localAppPageId(payload.url)')).toBeLessThan(
      handler.indexOf('await windowAppPreviewBackend(window)')
    );
    const backend = constFunctionSource('windowAppPreviewBackend');
    expect(backend).toContain('await managedAppPreviewBindings.get(window.id)');
    // A shared daemon that named no listener as the window opened is asked again, rather than
    // leaving the window without previews until the daemon next restarts; not while it is lost.
    expect(backend).toContain('sharedAppPreviewLifetime.signal.aborted');
    expect(backend).toContain('return bindWindowAppPreview(window)');
    expect(constFunctionSource('openAppForWindow')).toContain(
      'await windowAppPreviewBackend(window)'
    );
  });

  it('makes the view synchronously, so the bounds and visibility sent behind it find it', () => {
    const handler = ipcHandlerSource('embedded-browser:create');
    expect(handler).not.toMatch(/async \(event/);
    expect(handler).not.toContain('await ');
    expect(handler).toContain('managedAppPreviewBackends.get(window.id)');
  });

  it('revokes previews when the shared daemon is lost, and rebinds when one answers', () => {
    const watch = constFunctionSource('watchSharedDaemon');
    const lost = watch.slice(watch.indexOf("event.kind === 'lost'"), watch.indexOf('} else {'));
    expect(lost).toContain('sharedAppPreviewLifetime.abort()');
    expect(watch.slice(watch.indexOf('} else {'))).toContain(
      'rebindSharedAppPreviews(event.instanceId)'
    );
    const rebind = constFunctionSource('rebindSharedAppPreviews');
    expect(rebind).toContain('sharedAppPreviewLifetime = new AbortController()');
    expect(rebind).toContain('void bindWindowAppPreview(window)');
  });
});
