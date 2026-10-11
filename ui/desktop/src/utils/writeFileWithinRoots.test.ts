// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { writeFileWithinRoots } from './writeFileWithinRoots';

let root: string;
let allowed: string;

beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-write-file-'));
  allowed = path.join(root, 'allowed');
  await fs.mkdir(allowed);
});

afterEach(async () => {
  await fs.rm(root, { recursive: true, force: true });
});

describe('generic renderer file writes', () => {
  it('creates ordinary files and atomically replaces them without leaving temporary files', async () => {
    const target = path.join(allowed, 'nested', 'workflow.yaml');
    await writeFileWithinRoots(target, 'first', [allowed]);
    expect(await fs.readFile(target, 'utf8')).toBe('first');
    await writeFileWithinRoots(target, 'second', [allowed]);
    expect(await fs.readFile(target, 'utf8')).toBe('second');
    expect(await fs.readdir(path.dirname(target))).toEqual(['workflow.yaml']);
  });

  it('refuses sibling prefix matches before creating a directory', async () => {
    const target = path.join(`${allowed}-outside`, 'nested', 'write.txt');
    await expect(writeFileWithinRoots(target, 'blocked', [allowed])).rejects.toThrow(
      'outside allowed directories'
    );
    await expect(fs.stat(path.dirname(target))).rejects.toMatchObject({ code: 'ENOENT' });
  });

  it('resolves parent links before allowing a new file', async () => {
    const outside = path.join(root, 'outside');
    await fs.mkdir(outside);
    await fs.symlink(outside, path.join(allowed, 'escape'), 'dir');
    await expect(
      writeFileWithinRoots(path.join(allowed, 'escape', 'nested', 'write.txt'), 'blocked', [
        allowed,
      ])
    ).rejects.toThrow('outside allowed directories');
    expect(await fs.readdir(outside)).toEqual([]);
  });

  it('refuses a final file link outside the roots and preserves its target', async () => {
    const outside = path.join(root, 'private.txt');
    await fs.writeFile(outside, 'original');
    const linked = path.join(allowed, 'linked.txt');
    await fs.symlink(outside, linked);
    await expect(writeFileWithinRoots(linked, 'blocked', [allowed])).rejects.toThrow(
      'outside allowed directories'
    );
    expect(await fs.readFile(outside, 'utf8')).toBe('original');
    expect((await fs.lstat(linked)).isSymbolicLink()).toBe(true);
  });

  it('allows roots named through a link and rejects relative or invalid payloads', async () => {
    const rootLink = path.join(root, 'root-link');
    await fs.symlink(allowed, rootLink, 'dir');
    const target = path.join(allowed, 'write.txt');
    await writeFileWithinRoots(target, 'works', [rootLink]);
    expect(await fs.readFile(target, 'utf8')).toBe('works');
    for (const [candidate, content] of [
      ['relative.txt', 'no'],
      ['', 'no'],
      [target, { value: 'no' }],
    ] as const) {
      await expect(writeFileWithinRoots(candidate, content, [allowed])).rejects.toThrow(
        'Invalid file write request'
      );
    }
    expect(await fs.readFile(target, 'utf8')).toBe('works');
  });

  it('receives only main-process roots from the window owning the IPC request', async () => {
    const main = await fs.readFile(path.join(__dirname, '..', 'main.ts'), 'utf8');
    const start = main.indexOf("ipcMain.handle('write-file'");
    const handler = main.slice(start, main.indexOf('\n});', start));
    expect(start).toBeGreaterThan(-1);
    expect(handler).toContain('allowedFileRoots(workingDirForSender(event))');
    expect(handler).toContain('writeFileWithinRoots(expandedPath, content,');
  });
});
