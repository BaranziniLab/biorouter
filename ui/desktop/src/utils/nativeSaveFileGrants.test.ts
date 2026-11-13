// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { NativeSaveFileGrants } from './nativeSaveFileGrants';
import { writeFileWithinRoots } from './writeFileWithinRoots';

let root: string;
let allowed: string;
let outside: string;
let grants: NativeSaveFileGrants<object>;
let sender: object;

beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), 'biorouter-native-save-'));
  allowed = path.join(root, 'working');
  outside = path.join(root, 'chosen');
  await fs.mkdir(allowed);
  await fs.mkdir(outside);
  grants = new NativeSaveFileGrants();
  sender = {};
});

afterEach(async () => {
  await fs.rm(root, { recursive: true, force: true });
});

async function choose(filePath: string, owner = sender): Promise<void> {
  expect(await grants.select(owner, grants.begin(owner), filePath)).toBe(true);
}

describe('native Save file choices', () => {
  it('writes the chosen outside file once without authorizing its directory', async () => {
    const target = path.join(outside, 'chat.json');
    await expect(writeFileWithinRoots(target, 'unselected', [allowed])).rejects.toThrow(
      'outside allowed directories'
    );
    await choose(target);
    expect(await grants.write(sender, target, 'exported')).toBe(true);
    expect(await fs.readFile(target, 'utf8')).toBe('exported');
    expect(await grants.write(sender, target, 'replayed')).toBe(false);
    expect(await grants.write(sender, path.join(outside, 'sibling.json'), 'no')).toBe(false);
    expect(await fs.readdir(outside)).toEqual(['chat.json']);
    expect(await fs.readFile(target, 'utf8')).toBe('exported');
  });

  it('does not let another sender consume or use the selected file', async () => {
    const target = path.join(outside, 'chat.json');
    await choose(target);
    expect(await grants.write({}, target, 'foreign')).toBe(false);
    await expect(fs.stat(target)).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await grants.write(sender, target, 'owner')).toBe(true);
  });

  it('refuses another path and consumes the original selection', async () => {
    const target = path.join(outside, 'chat.json');
    await choose(target);
    expect(await grants.write(sender, path.join(outside, 'other.json'), 'wrong')).toBe(false);
    expect(await grants.write(sender, target, 'reused')).toBe(false);
    expect(await fs.readdir(outside)).toEqual([]);
  });

  it('revokes an earlier choice when a new dialog starts or is canceled', async () => {
    const target = path.join(outside, 'chat.json');
    await choose(target);
    const canceled = grants.begin(sender);
    expect(await grants.write(sender, target, 'old')).toBe(false);
    expect(await grants.select(sender, canceled)).toBe(false);
    expect(await grants.write(sender, target, 'after cancel')).toBe(false);
    expect(await fs.readdir(outside)).toEqual([]);
  });

  it('ignores an older dialog result without revoking the newer choice', async () => {
    const old = grants.begin(sender);
    const current = grants.begin(sender);
    const target = path.join(outside, 'current.json');
    expect(await grants.select(sender, current, target)).toBe(true);
    expect(await grants.select(sender, old, path.join(outside, 'old.json'))).toBe(false);
    expect(await grants.write(sender, target, 'current')).toBe(true);
    expect(await fs.readdir(outside)).toEqual(['current.json']);
  });

  it('revokes pending and selected choices when their owner goes away', async () => {
    const token = grants.begin(sender);
    const target = path.join(outside, 'chat.json');
    grants.revoke(sender);
    expect(await grants.select(sender, token, target)).toBe(false);
    await choose(target);
    grants.revoke(sender);
    expect(await grants.write(sender, target, 'gone')).toBe(false);
    expect(await fs.readdir(outside)).toEqual([]);
  });

  it('permits only one of two concurrent writes', async () => {
    const target = path.join(outside, 'chat.json');
    await choose(target);
    const results = await Promise.all([
      grants.write(sender, target, 'first'),
      grants.write(sender, target, 'second'),
    ]);
    expect(results).toEqual([true, false]);
    expect(await fs.readFile(target, 'utf8')).toBe('first');
  });

  it('refuses a replaced selected parent even when its pathname stays the same', async () => {
    const target = path.join(outside, 'chat.json');
    await choose(target);
    const moved = `${outside}-original`;
    await fs.rename(outside, moved);
    await fs.mkdir(outside);
    await expect(grants.write(sender, target, 'changed')).rejects.toThrow(
      'selected save directory changed'
    );
    expect(await fs.readdir(outside)).toEqual([]);
    expect(await fs.readdir(moved)).toEqual([]);
    expect(await grants.write(sender, target, 'retry')).toBe(false);
  });

  it('pins the selected parent through a link and refuses a later link change', async () => {
    const link = path.join(root, 'selected-link');
    await fs.symlink(outside, link, 'dir');
    const target = path.join(link, 'chat.json');
    await choose(target);
    expect(await grants.write(sender, target, 'first')).toBe(true);
    await choose(target);
    await fs.unlink(link);
    await fs.symlink(allowed, link, 'dir');
    await expect(grants.write(sender, target, 'redirected')).rejects.toThrow(
      'selected save directory changed'
    );
    expect(await fs.readFile(path.join(outside, 'chat.json'), 'utf8')).toBe('first');
    expect(await fs.readdir(allowed)).toEqual([]);
  });

  it('retains the secure writer refusal for an escaping final link', async () => {
    const original = path.join(allowed, 'original.json');
    await fs.writeFile(original, 'original');
    const target = path.join(outside, 'linked.json');
    await fs.symlink(original, target);
    await choose(target);
    await expect(grants.write(sender, target, 'blocked')).rejects.toThrow(
      'outside allowed directories'
    );
    expect(await fs.readFile(original, 'utf8')).toBe('original');
    expect((await fs.lstat(target)).isSymbolicLink()).toBe(true);
  });

  it('leaves ordinary root writes available without a native choice', async () => {
    const target = path.join(allowed, 'normal.json');
    expect(await grants.write(sender, target, 'ordinary')).toBe(false);
    await writeFileWithinRoots(target, 'ordinary', [allowed]);
    expect(await fs.readFile(target, 'utf8')).toBe('ordinary');
    for (const invalid of ['', 'relative.json', path.join(root, 'missing', 'chat.json')]) {
      expect(await grants.select(sender, grants.begin(sender), invalid)).toBe(false);
    }
  });
});
