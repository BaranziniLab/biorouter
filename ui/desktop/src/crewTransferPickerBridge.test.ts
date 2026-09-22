// @vitest-environment node
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const source = (name: string) =>
  fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), name), 'utf8');

describe('Crew transfer picker opaque-capability bridge', () => {
  it('keeps selected filesystem paths in the main-process request and returns only the daemon capability', () => {
    const main = source('main.ts');
    const start = main.indexOf("ipcMain.handle('crew:select-transfer-file'");
    const end = main.indexOf("ipcMain.handle('crew:authenticate'", start);
    expect(start).toBeGreaterThanOrEqual(0);
    expect(end).toBeGreaterThan(start);
    const handler = main.slice(start, end);
    const returnStart = handler.lastIndexOf('return {');
    expect(handler).toMatch(/path: selected/);
    expect(handler).toMatch(/const purpose = options\.purpose \?\? 'transfer'/);
    expect(handler).toMatch(/purpose === 'cleanup'/);
    expect(handler).toMatch(/properties: \['openDirectory'\]/);
    expect(handler).toMatch(/typeof result\.capability_id !== 'string'/);
    expect(handler.slice(returnStart)).toMatch(
      /return \{\s*capability_id: result\.capability_id,\s*name: result\.name/
    );
    expect(handler.slice(returnStart)).not.toMatch(/selected|path|bytes|contents/);
    expect(handler).toMatch(/if \(!selected\) return null/);
  });

  it('passes only the opaque capability identifier from preload to Crew API calls', () => {
    const preload = source('preload.ts');
    const transfers = source('components/crew/crewTransfers.ts');
    const preloadStart = preload.indexOf('crewSelectTransferFile:');
    const preloadEnd = preload.indexOf('\n  ', preloadStart + 10);
    expect(preload.slice(preloadStart, preloadEnd)).not.toMatch(/path|bytes|contents/);
    expect(transfers).toMatch(/purpose: request\.purpose/);
    expect(transfers).toMatch(/file_capability: file\.capability_id/);
    expect(transfers).not.toMatch(/file\.(path|bytes|contents)/);
  });

  it('rejects an invalid privacy enum before opening a native picker and keeps daemon errors safe', () => {
    const main = source('main.ts');
    const start = main.indexOf("ipcMain.handle('crew:select-transfer-file'");
    const end = main.indexOf("ipcMain.handle('crew:authenticate'", start);
    const handler = main.slice(start, end);
    const validation = handler.indexOf('Invalid expected transfer privacy.');
    const firstDialog = handler.indexOf('dialog.showOpenDialog');
    expect(validation).toBeGreaterThanOrEqual(0);
    expect(firstDialog).toBeGreaterThan(validation);
    expect(handler).toMatch(/options\.expectedMode !== undefined/);
    expect(handler).toMatch(/options\.expectedMode !== 'private'/);
    expect(handler).toMatch(/options\.expectedMode !== 'public'/);
    expect(handler).toContain(
      'Connection privacy changed. Refresh Crew and choose the file again.'
    );
    expect(handler).toContain(
      'The daemon refused this file selection. Choose an accessible file or a new destination filename.'
    );
    expect(handler).not.toMatch(/throw new Error\(failure\?\.error/);
  });
});
