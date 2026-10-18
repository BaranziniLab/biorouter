import { existsSync, readFileSync } from 'node:fs';
import * as path from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * A page with no icon link makes the browser ask for `/favicon.ico` at the
 * host's root. Behind a reverse proxy that serves Biorouter under a path
 * (JupyterHub's `/user/<name>/proxy/<port>/`), that request leaves the prefix
 * and reaches whatever the proxy serves at its root: measured in the live test
 * of PR #404 on 2026-10-17, it was the one request of the session outside the
 * prefix. A relative link resolves against the `<base href>` the daemon writes
 * (`routes::web_ui::with_base`), so the icon is asked for under the prefix.
 */
describe('the shell names its icon relative to itself', () => {
  const desktop = path.join(__dirname, '..');
  const shell = readFileSync(path.join(desktop, 'index.html'), 'utf-8');
  const links = [...shell.matchAll(/<link\b[^>]*\brel="icon"[^>]*>/g)].map((m) => m[0]);

  it('has one icon link, relative, naming a file the build can emit', () => {
    expect(links).toHaveLength(1);
    const href = links[0].match(/\bhref="([^"]+)"/)?.[1] ?? '';
    // Not `/...`, which leaves a proxy's prefix, and not an absolute URL.
    expect(href).toMatch(/^\.\//);
    expect(existsSync(path.join(desktop, href))).toBe(true);
  });
});
