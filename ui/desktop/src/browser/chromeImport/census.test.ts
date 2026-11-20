// @vitest-environment node
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * BB-21: only the person can start a Chrome import, and only from the desktop app. There is no
 * daemon route, workspace frame, agent operation or tool for it. This census reads the tree and
 * fails if any of those names the import, so a later change cannot quietly hand it to an agent.
 */

const DESKTOP = path.resolve(__dirname, '../../..');
const REPO = path.resolve(DESKTOP, '../..');
const IMPORT_WORDS =
  /chrome[_-]?import|import[_-]?chrome|browser[_-]?import|biorouter-browser-import/i;

function walk(dir: string, accept: (file: string) => boolean, out: string[] = []): string[] {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    if (entry.name === 'node_modules' || entry.name === 'target' || entry.name.startsWith('.')) {
      continue;
    }
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, accept, out);
    else if (accept(full)) out.push(full);
  }
  return out;
}

const relative = (file: string) => path.relative(REPO, file);

describe('the Chrome import census (BB-21)', () => {
  it('no Rust code, route, tool or OpenAPI schema names the import', () => {
    const rust = walk(path.join(REPO, 'crates'), (file) => file.endsWith('.rs'));
    expect(rust.length).toBeGreaterThan(100);
    const naming = rust.filter((file) => IMPORT_WORDS.test(fs.readFileSync(file, 'utf8')));
    expect(naming.map(relative)).toEqual([]);
    const openapi = fs.readFileSync(path.join(DESKTOP, 'openapi.json'), 'utf8');
    expect(IMPORT_WORDS.test(openapi)).toBe(false);
  });

  it('the key helper is reached only from the daemon binary, never from a route or tool', () => {
    const rust = walk(path.join(REPO, 'crates'), (file) => file.endsWith('.rs'));
    const users = rust
      .filter((file) => /browser_key::|browser-key|BrowserKey/.test(fs.readFileSync(file, 'utf8')))
      .map(relative)
      .sort();
    expect(users).toEqual([
      // The secret guard names the helper only to refuse any command that runs it (S1).
      'crates/biorouter-mcp/src/secret_guard.rs',
      'crates/biorouter-mcp/src/secret_guard/resolve.rs',
      'crates/biorouter-server/src/main.rs',
      'crates/biorouter/src/browser_key.rs',
    ]);
  });

  it('no agent path in the desktop app reaches the import', () => {
    const agentPaths = [
      ...walk(path.join(DESKTOP, 'src/browser/agent'), (file) => /\.tsx?$/.test(file)),
      ...walk(path.join(DESKTOP, 'src/components/chatGroups'), (file) => /\.tsx?$/.test(file)),
      ...walk(path.join(DESKTOP, 'src/api'), (file) => /\.ts$/.test(file)),
    ].filter((file) => !/\.test\.tsx?$/.test(file));
    expect(agentPaths.length).toBeGreaterThan(5);
    const naming = agentPaths.filter((file) => {
      const text = fs.readFileSync(file, 'utf8');
      return IMPORT_WORDS.test(text) || text.includes('browserImport');
    });
    expect(naming.map(relative)).toEqual([]);
  });

  it('the import channels are handled in main and bridged in preload, and nowhere else', () => {
    const sources = walk(path.join(DESKTOP, 'src'), (file) => /\.tsx?$/.test(file)).filter(
      (file) => !/\.test\.tsx?$/.test(file)
    );
    const channelUsers = sources
      .filter((file) => fs.readFileSync(file, 'utf8').includes("'biorouter-browser-import:"))
      .map(relative)
      .sort();
    expect(channelUsers).toEqual(['ui/desktop/src/main.ts', 'ui/desktop/src/preload.ts']);
    const main = fs.readFileSync(path.join(DESKTOP, 'src/main.ts'), 'utf8');
    const handlers = [
      ...main.matchAll(/ipcMain\.handle\('(biorouter-browser-import:[a-z-]+)'/g),
    ].map((match) => match[1]);
    expect(handlers.sort()).toEqual(
      [
        'biorouter-browser-import:dismiss',
        'biorouter-browser-import:profiles',
        'biorouter-browser-import:remove',
        'biorouter-browser-import:run',
        'biorouter-browser-import:sites',
        'biorouter-browser-import:status',
      ].sort()
    );
    // Every handler checks its sender is one of the app's own windows before anything else.
    for (const channel of handlers) {
      const start = main.indexOf(`ipcMain.handle('${channel}'`);
      const next = main.indexOf('ipcMain.handle(', start + 1);
      const body = main.slice(start, next === -1 ? undefined : next);
      expect(body, channel).toMatch(/browserImportOwner\(event\)/);
    }
  });

  it('the import starts only after main’s own native confirmation (S3)', () => {
    const main = fs.readFileSync(path.join(DESKTOP, 'src/main.ts'), 'utf8');
    const start = main.indexOf("ipcMain.handle('biorouter-browser-import:run'");
    const body = main.slice(start, main.indexOf('ipcMain.handle(', start + 1));
    expect(body).toMatch(/\.run\(request, \(text\) => confirmChromeImport\(owner, text\)\)/);
    const confirm = main.slice(main.indexOf('async function confirmChromeImport'));
    expect(confirm.slice(0, 600)).toMatch(/dialog\.showMessageBox\(owner,/);
    expect(confirm.slice(0, 600)).toMatch(/return response === 1;/);
  });

  it('the native confirmation defaults to Cancel, so a stray Return does not import', () => {
    const main = fs.readFileSync(path.join(DESKTOP, 'src/main.ts'), 'utf8');
    const confirm = main.slice(main.indexOf('async function confirmChromeImport'));
    const options = confirm.slice(0, confirm.indexOf('});'));
    expect(options).toMatch(/buttons: \['Cancel', text\.confirmLabel\]/);
    expect(options).toMatch(/defaultId: 0,/);
    expect(options).toMatch(/cancelId: 0,/);
  });
});
