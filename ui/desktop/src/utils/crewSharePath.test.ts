// @vitest-environment node
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CREW_SHARE_BUTTON_SHARE,
  CREW_SHARE_LABEL_MAX_CHARS,
  CREW_SHARE_SIZE_LIMIT,
  CREW_FILE_IS_CREDENTIAL,
  CrewSharePending,
  DEV_AUTO_CONFIRM_SHARE_ENV,
  crewFileRefusal,
  crewShareCopy,
  crewShareDialogOptions,
  crewShareRegistrationBody,
  inspectDroppedFile,
  parseCrewShareRequest,
  resolveDevAutoConfirmShare,
  shareDroppedFile,
  visibleText,
  type CrewShareRequest,
  type ShareDroppedFileDeps,
  type ShareFs,
} from './crewSharePath';

const posix = process.platform !== 'win32';
const isRoot = typeof process.getuid === 'function' && process.getuid() === 0;

let root: string;
beforeEach(async () => {
  // Resolved, so the expectations below compare against real paths (macOS's tmpdir is under a
  // linked /var).
  root = await fsp.realpath(await fsp.mkdtemp(path.join(os.tmpdir(), 'crew-share-')));
});
afterEach(async () => {
  await fsp.chmod(root, 0o700).catch(() => undefined);
  await fsp.rm(root, { recursive: true, force: true });
});

const write = async (relative: string, content = 'a,b\n1,2\n') => {
  const target = path.join(root, relative);
  await fsp.mkdir(path.dirname(target), { recursive: true });
  await fsp.writeFile(target, content);
  return target;
};

const refusal = async (target: string, options?: Parameters<typeof inspectDroppedFile>[1]) => {
  const result = await inspectDroppedFile(target, options);
  if (result.ok) throw new Error(`expected a refusal for ${target}`);
  return result.message;
};

describe('inspectDroppedFile: what may be offered at all', () => {
  it('accepts a regular file and reports its real path, name and size', async () => {
    const file = await write('growth.csv');
    const result = await inspectDroppedFile(file);
    expect(result).toMatchObject({ ok: true, realPath: file, name: 'growth.csv', size: 8 });
  });

  it.each([
    ['an empty path (pasted image data, a synthetic File)', ''],
    ['a relative path', 'growth.csv'],
    ['a path with a NUL', '/tmp/a\0b'],
    ['a path longer than a drop produces', `/${'a'.repeat(5000)}`],
  ])('refuses %s as not a saved file', async (_label, target) => {
    expect(await refusal(target)).toBe(crewShareCopy.notAFile);
  });

  it('refuses a file that is gone', async () => {
    expect(await refusal(path.join(root, 'gone.csv'))).toBe(crewShareCopy.missing('gone.csv'));
  });

  it('refuses a folder', async () => {
    await fsp.mkdir(path.join(root, 'results'));
    expect(await refusal(path.join(root, 'results'))).toBe(crewShareCopy.folder('results'));
  });

  it.skipIf(!posix)('refuses a shortcut to a file in another folder', async () => {
    const secret = await write('elsewhere/id_rsa', 'PRIVATE');
    await fsp.mkdir(path.join(root, 'desktop'));
    const link = path.join(root, 'desktop', 'notes.csv');
    await fsp.symlink(secret, link);
    expect(await refusal(link)).toBe(crewShareCopy.shortcutOutside('notes.csv'));
  });

  it.skipIf(!posix)('refuses a relative shortcut that climbs out of its folder', async () => {
    await write('outside.csv');
    await fsp.mkdir(path.join(root, 'desktop'));
    const link = path.join(root, 'desktop', 'data.csv');
    await fsp.symlink('../outside.csv', link);
    expect(await refusal(link)).toBe(crewShareCopy.shortcutOutside('data.csv'));
  });

  it.skipIf(!posix)(
    'follows a shortcut whose target is in its own folder and shows the target',
    async () => {
      const target = await write('lab/growth-v2.csv');
      const link = path.join(root, 'lab', 'growth.csv');
      await fsp.symlink('growth-v2.csv', link);
      expect(await inspectDroppedFile(link)).toMatchObject({
        ok: true,
        realPath: target,
        name: 'growth-v2.csv',
      });
    }
  );

  it.skipIf(!posix)('follows a shortcut into a folder below its own', async () => {
    const target = await write('lab/raw/growth.csv');
    const link = path.join(root, 'lab', 'growth.csv');
    await fsp.symlink(path.join('raw', 'growth.csv'), link);
    expect(await inspectDroppedFile(link)).toMatchObject({ ok: true, realPath: target });
  });

  it.skipIf(!posix)('refuses a shortcut to its own folder as a folder', async () => {
    await fsp.mkdir(path.join(root, 'lab'));
    const link = path.join(root, 'lab', 'here');
    await fsp.symlink('.', link);
    expect(await refusal(link)).toBe(crewShareCopy.folder('lab'));
  });

  it.skipIf(!posix)('refuses a broken shortcut', async () => {
    const link = path.join(root, 'dangling.csv');
    await fsp.symlink(path.join(root, 'nothing-here.csv'), link);
    expect(await refusal(link)).toBe(crewShareCopy.shortcutBroken('dangling.csv'));
  });

  it.skipIf(!posix)(
    'resolves a linked folder above the file, so the dialog shows the real location',
    async () => {
      const target = await write('real-folder/growth.csv');
      const linkedFolder = path.join(root, 'linked-folder');
      await fsp.symlink(path.join(root, 'real-folder'), linkedFolder);
      const result = await inspectDroppedFile(path.join(linkedFolder, 'growth.csv'));
      expect(result).toMatchObject({ ok: true, realPath: target });
    }
  );

  it.skipIf(!posix)('refuses a named pipe', async () => {
    const fifo = path.join(root, 'pipe');
    execFileSync('mkfifo', [fifo]);
    expect(await refusal(fifo)).toBe(crewShareCopy.special('pipe'));
  });

  it.skipIf(!posix)('refuses a socket', async () => {
    const socketPath = path.join(root, 's.sock');
    const server = net.createServer();
    await new Promise<void>((resolve) => server.listen(socketPath, resolve));
    try {
      expect(await refusal(socketPath)).toBe(crewShareCopy.special('s.sock'));
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  });

  it.skipIf(!posix)('refuses a device', async () => {
    expect(await refusal('/dev/null')).toBe(crewShareCopy.special('null'));
  });

  it.skipIf(!posix)('refuses a shortcut to a device even from its own folder', async () => {
    const link = path.join(root, 'looks-like-data.csv');
    await fsp.symlink('/dev/zero', link);
    expect(await refusal(link)).toBe(crewShareCopy.shortcutOutside('looks-like-data.csv'));
  });

  it('refuses a file over the limit and accepts one exactly at it', async () => {
    const big = await write('eleven.bin', 'x'.repeat(11));
    expect(await refusal(big, { limit: 10 })).toBe(crewShareCopy.tooLarge('eleven.bin', 10));
    expect(crewShareCopy.tooLarge('eleven.bin', 10)).toBe(
      '"eleven.bin" is larger than 10 bytes, the most Crew can attach.'
    );
    const exact = await write('ten.bin', 'x'.repeat(10));
    expect(await inspectDroppedFile(exact, { limit: 10 })).toMatchObject({ ok: true, size: 10 });
  });

  it("defaults to the daemon's 1 GiB attachment limit", async () => {
    expect(CREW_SHARE_SIZE_LIMIT).toBe(1024 ** 3);
    const fakeFs = (size: number): ShareFs => ({
      lstat: async () => ({
        isFile: () => true,
        isDirectory: () => false,
        isSymbolicLink: () => false,
        size,
        dev: 1,
        ino: 2,
        mtimeMs: 3,
      }),
      realpath: async (target) => target,
      access: async () => undefined,
    });
    expect(await refusal('/data/huge.bam', { fs: fakeFs(1024 ** 3 + 1) })).toBe(
      '"huge.bam" is larger than 1 GB, the most Crew can attach.'
    );
    expect(await inspectDroppedFile('/data/huge.bam', { fs: fakeFs(1024 ** 3) })).toMatchObject({
      ok: true,
    });
  });

  it.skipIf(!posix || isRoot)('refuses a file this user cannot read', async () => {
    const file = await write('locked.csv');
    await fsp.chmod(file, 0o000);
    expect(await refusal(file)).toBe(crewShareCopy.unreadable('locked.csv'));
  });

  it.skipIf(!posix)('makes hidden characters in a refused name visible', async () => {
    const name = 'report\u202Evsc.exe';
    await fsp.mkdir(path.join(root, name));
    const message = await refusal(path.join(root, name));
    expect(message).toBe(crewShareCopy.folder('report�vsc.exe'));
    expect(message).not.toContain('\u202E');
  });

  it('notices a file replaced since the last look through its identity', async () => {
    const file = await write('growth.csv');
    const first = await inspectDroppedFile(file);
    await fsp.rename(await write('other.csv', 'z,z\n9,9\n'), file);
    const second = await inspectDroppedFile(file);
    expect(first.ok && second.ok).toBe(true);
    if (first.ok && second.ok) expect(second.identity).not.toBe(first.identity);
  });
});

describe('the native confirmation', () => {
  const file = { name: 'growth.csv', size: 2_516_582, realPath: '/Users/frank/Desktop/growth.csv' };

  it('names the file, its size and the destination, with the full path in the detail', () => {
    const options = crewShareDialogOptions(file, {
      channelName: 'methods',
      workspaceName: 'chen-lab',
    });
    expect(options.message).toBe('Share "growth.csv" (2.4 MB) to Crew?');
    expect(options.detail).toBe(
      [
        'Full path: /Users/frank/Desktop/growth.csv',
        'Destination: #methods in chen-lab',
        'It uploads now and appears in #methods when you send your message.',
      ].join('\n')
    );
  });

  it('says Share uploads now and posts on Send, below the true path (Q3-14)', () => {
    const options = crewShareDialogOptions(file, { channelName: 'data', workspaceName: 'lab' });
    const lines = options.detail?.split('\n') ?? [];
    expect(lines[0]).toBe('Full path: /Users/frank/Desktop/growth.csv');
    expect(lines[lines.length - 1]).toBe(
      'It uploads now and appears in #data when you send your message.'
    );
    expect(options.defaultId).toBe(1);
    expect(options.buttons?.[1]).toBe('Cancel');
  });

  it('offers Share and Cancel, with Cancel the default and the Escape answer', () => {
    const options = crewShareDialogOptions(file, { channelName: 'm', workspaceName: 'w' });
    expect(options.buttons).toEqual(['Share', 'Cancel']);
    expect(options.buttons?.[CREW_SHARE_BUTTON_SHARE]).toBe('Share');
    expect(options.defaultId).toBe(1);
    expect(options.cancelId).toBe(1);
    expect(options.noLink).toBe(true);
  });

  it('shows hidden characters in the name and path instead of rendering them', () => {
    const options = crewShareDialogOptions(
      { name: 'a\u202Egpj.exe', size: 1, realPath: '/x/a\u202Egpj.exe\n/etc/passwd' },
      { channelName: 'm', workspaceName: 'w' }
    );
    expect(options.message).toContain('"a�gpj.exe" (1 byte)');
    expect(options.detail?.split('\n')[0]).toBe('Full path: /x/a�gpj.exe�/etc/passwd');
  });

  it('keeps ordinary non-Latin names, and turns only hidden characters into U+FFFD', () => {
    expect(visibleText('数据 données.csv')).toBe('数据 données.csv');
    expect(visibleText('a\u200Bb\tc')).toBe('a�b�c');
  });

  it('turns the line and paragraph separators into U+FFFD, which a control-character rule misses', () => {
    // U+2028 and U+2029 are neither \p{Cc} nor \p{Cf}, and both are mandatory line breaks.
    expect(visibleText('a\u2028b\u2029c')).toBe('a\uFFFDb\uFFFDc');
    expect(visibleText('\u0085\u000B\u000C')).toBe('\uFFFD\uFFFD\uFFFD');
  });

  it('collapses a run of spaces of any width to one, and keeps a single space as it is', () => {
    expect(visibleText('Chen Lab' + ' '.repeat(60) + 'Full path')).toBe('Chen Lab Full path');
    expect(visibleText('a\u3000\u2003\u00A0 b')).toBe('a b');
    // The narrow no-break space macOS writes into screenshot names.
    const screenshot = 'Screenshot 2026-09-24 at 10.15.32\u202FAM.png';
    expect(visibleText(screenshot)).toBe(screenshot);
  });

  it('keeps a line separator in a file name or path from breaking the dialog', () => {
    const options = crewShareDialogOptions(
      {
        name: 'report\u2028Full path: x.csv',
        size: 1,
        realPath: '/x/report\u2028Full path: x.csv',
      },
      { channelName: 'm', workspaceName: 'w' }
    );
    for (const text of [options.message, options.detail])
      expect(text).not.toMatch(/[\u2028\u2029]/);
    expect(options.message).toBe('Share "report\uFFFDFull path: x.csv" (1 byte) to Crew?');
    expect(options.detail?.split('\n')).toEqual([
      'Full path: /x/report\uFFFDFull path: x.csv',
      'Destination: #m in w',
      'It uploads now and appears in #m when you send your message.',
    ]);
  });

  describe('a destination name that tries to forge a line (renderer-written text)', () => {
    const forged = `Chen Lab\u2028\u2028Full path: /Users/frank/Desktop/growth.csv\u2028`;
    const real = { name: 'config', size: 412, realPath: '/Users/frank/.ssh/config' };
    const lines = (options: ReturnType<typeof crewShareDialogOptions>) => ({
      message: options.message,
      detail: options.detail?.split('\n'),
    });

    it('cannot put a line above the true path, through the parsed request', () => {
      const request = parseCrewShareRequest({
        path: real.realPath,
        connectionId: 'conn-1',
        channelId: 'chan-1',
        channelName: `general\u2029Full path: /tmp/a.csv`,
        workspaceName: forged,
      });
      expect(request.workspaceName).toBe('Chen Lab Full path: /Users/frank/Desktop/growth.csv');
      expect(lines(crewShareDialogOptions(real, request))).toEqual({
        message: 'Share "config" (412 bytes) to Crew?',
        detail: [
          'Full path: /Users/frank/.ssh/config',
          'Destination: #general Full path: /tmp/a.csv in Chen Lab Full path: /Users/frank/Desktop/growth.csv',
          'It uploads now and appears in #general Full path: /tmp/a.csv when you send your message.',
        ],
      });
    });

    it('cannot either when a caller hands the dialog raw names', () => {
      const options = crewShareDialogOptions(real, {
        channelName: 'general\n\u2028x',
        workspaceName: forged,
      });
      expect(options.message).toBe('Share "config" (412 bytes) to Crew?');
      expect(options.detail?.split('\n')).toHaveLength(3);
      expect(options.detail?.split('\n')[0]).toBe('Full path: /Users/frank/.ssh/config');
      expect(options.detail).not.toMatch(/[\u2028\u2029]/);
    });

    it('keeps the renderer-written names out of the bold message entirely', () => {
      const options = crewShareDialogOptions(real, {
        channelName: 'Full path: /a',
        workspaceName: 'Full path: /b',
      });
      expect(options.message).not.toContain('Full path');
      expect(options.detail?.indexOf('Full path: /Users/frank/.ssh/config')).toBe(0);
    });
  });
});

describe('crewFileRefusal: the daemon credential floor, in words (Q3-01, Q3-15)', () => {
  const credential = { code: CREW_FILE_IS_CREDENTIAL, error: 'daemon text is never shown' };

  it('names the file for an upload, and the folder for a download', () => {
    expect(crewFileRefusal(credential, 'upload', 'secrets.yaml')).toBe(
      "\u201csecrets.yaml\u201d looks like a credential file (a password, key or token store), so Crew won't share it."
    );
    expect(crewFileRefusal(credential, 'download', 'id_ed25519')).toBe(
      "Crew won't save into a credential or settings location. Choose another folder."
    );
  });

  it('makes a hidden character in the name visible rather than rendering it', () => {
    expect(crewFileRefusal(credential, 'upload', 'id_rsa\u202Evsc.\n')).toBe(
      crewShareCopy.credential('id_rsa\uFFFDvsc.\uFFFD')
    );
    expect(crewFileRefusal(credential, 'upload', '')).toBe(crewShareCopy.credential('This file'));
  });

  it.each([
    ['another code', { code: 'crew_transfer_refused', error: 'Symlink file selections' }],
    [
      'the privacy change',
      {
        error:
          'Crew connection privacy changed; refresh the verified workspace before selecting a file',
      },
    ],
    ['no body', null],
    ['a string body', 'crew_file_is_credential'],
  ])('leaves %s to the caller', (_label, failure) => {
    expect(crewFileRefusal(failure, 'upload', 'a.csv')).toBeUndefined();
  });

  it("is the daemon's code and sentences word for word", () => {
    const rust = fs.readFileSync(
      path.join(
        path.dirname(fileURLToPath(import.meta.url)),
        '../../../../crates/biorouter-server/src/crew/local_files.rs'
      ),
      'utf8'
    );
    expect(rust).toContain(
      `pub const CREDENTIAL_REFUSAL_CODE: &str = "${CREW_FILE_IS_CREDENTIAL}";`
    );
    const [open, rest] = crewShareCopy.credential('{name}').split('{name}');
    expect(open).toBe('\u201c');
    expect(rust).toContain(`"\\u{201c}{name}\\u{201d}${rest.slice(1)}"`);
    expect(rust).toContain(`"${crewShareCopy.credentialLocation}"`);
  });
});

describe('parseCrewShareRequest', () => {
  const valid = {
    path: '/Users/frank/Desktop/growth.csv',
    connectionId: 'conn-1',
    channelId: 'chan_2',
    channelName: '#methods',
    workspaceName: 'chen-lab',
    expectedMode: 'private',
  };

  it('keeps the named fields only, and drops a leading # from the channel', () => {
    expect(parseCrewShareRequest({ ...valid, bytes: 'x', overwrite: true })).toEqual({
      path: '/Users/frank/Desktop/growth.csv',
      connectionId: 'conn-1',
      channelId: 'chan_2',
      channelName: 'methods',
      workspaceName: 'chen-lab',
      expectedMode: 'private',
    });
  });

  it('keeps an empty path for the flow to refuse in words', () => {
    expect(parseCrewShareRequest({ ...valid, path: '' }).path).toBe('');
  });

  it.each([
    ['a non-object', 'x'],
    ['an array', []],
    ['a missing path', { ...valid, path: undefined }],
    ['a numeric path', { ...valid, path: 7 }],
    ['a bad connection id', { ...valid, connectionId: '../x' }],
    ['a bad channel id', { ...valid, channelId: 'a b' }],
    ['an overlong id', { ...valid, channelId: 'a'.repeat(129) }],
    ['an unknown privacy', { ...valid, expectedMode: 'secret' }],
    ['a missing channel name', { ...valid, channelName: undefined }],
    ['a channel name of only hidden characters', { ...valid, channelName: '#\u202E\u200B' }],
    ['a missing workspace name', { ...valid, workspaceName: '' }],
  ])('throws on %s', (_label, raw) => {
    expect(() => parseCrewShareRequest(raw)).toThrow();
  });

  it('flattens a destination name to one line and removes blank-looking padding', () => {
    const parsed = parseCrewShareRequest({
      ...valid,
      channelName: '#gen\u2028eral',
      // Hangul fillers render as blank space but are not White_Space.
      workspaceName: 'chen\u3164\u3164\u3164\u2003\u2003-lab\u2029',
    });
    expect(parsed.channelName).toBe('gen eral');
    expect(parsed.workspaceName).toBe('chen -lab');
  });

  it('strips hidden characters from the destination names and shortens long ones', () => {
    const parsed = parseCrewShareRequest({
      ...valid,
      channelName: '#gen\u202Eeral\n',
      workspaceName: 'w'.repeat(200),
    });
    expect(parsed.channelName).toBe('general');
    expect(Array.from(parsed.workspaceName)).toHaveLength(CREW_SHARE_LABEL_MAX_CHARS);
    expect(parsed.workspaceName.endsWith('…')).toBe(true);
  });
});

describe('crewShareRegistrationBody', () => {
  it("is the Attach picker's upload registration, with only fields the daemon accepts", () => {
    const request: CrewShareRequest = {
      path: '/ignored/by/the/body',
      connectionId: 'conn-1',
      channelId: 'chan-1',
      channelName: 'general',
      workspaceName: 'lab',
      expectedMode: 'public',
    };
    const body = crewShareRegistrationBody(request, '/real/growth.csv');
    expect(body).toEqual({
      direction: 'upload',
      purpose: 'transfer',
      path: '/real/growth.csv',
      overwrite: false,
      approval_pending: false,
      connection_id: 'conn-1',
      channel_id: 'chan-1',
      expected_mode: 'public',
    });
    // `FileRequest` is `deny_unknown_fields`: a key it does not declare fails the request.
    const rust = fs.readFileSync(
      path.join(
        path.dirname(fileURLToPath(import.meta.url)),
        '../../../../crates/biorouter-server/src/crew/transfers.rs'
      ),
      'utf8'
    );
    const struct = rust.slice(rust.indexOf('pub struct FileRequest {'));
    const declared = struct.slice(0, struct.indexOf('\n}'));
    for (const key of Object.keys(body)) expect(declared).toContain(`pub ${key}:`);
    expect(
      crewShareRegistrationBody({ ...request, expectedMode: undefined }, '/real/growth.csv')
    ).not.toHaveProperty('expected_mode');
  });
});

describe('shareDroppedFile', () => {
  let file: string;
  let request: CrewShareRequest;
  beforeEach(async () => {
    file = await write('growth.csv');
    request = {
      path: file,
      connectionId: 'conn-1',
      channelId: 'chan-1',
      channelName: 'methods',
      workspaceName: 'chen-lab',
      expectedMode: 'private',
    };
  });

  const deps = (overrides: Partial<ShareDroppedFileDeps> = {}) => {
    const base = {
      autoConfirm: false,
      confirm: vi.fn<ShareDroppedFileDeps['confirm']>(async () => CREW_SHARE_BUTTON_SHARE),
      register: vi.fn<ShareDroppedFileDeps['register']>(async () => ({
        ok: true,
        body: { capability_id: 'cap-1', name: 'growth.csv', size: 8 },
      })),
      discard: vi.fn<ShareDroppedFileDeps['discard']>(async () => undefined),
      isClosed: vi.fn<ShareDroppedFileDeps['isClosed']>(() => false),
      log: vi.fn<ShareDroppedFileDeps['log']>(),
    };
    return Object.assign(base, overrides) as typeof base;
  };

  it('shares only after Share, registering the real path and returning only the capability', async () => {
    const d = deps();
    await expect(shareDroppedFile(request, d)).resolves.toEqual({
      outcome: 'shared',
      capability_id: 'cap-1',
      name: 'growth.csv',
      size: 8,
    });
    expect(d.confirm).toHaveBeenCalledTimes(1);
    expect(d.confirm.mock.calls[0][0]).toMatchObject({
      message: 'Share "growth.csv" (8 bytes) to Crew?',
      detail: `Full path: ${file}\nDestination: #methods in chen-lab\nIt uploads now and appears in #methods when you send your message.`,
    });
    expect(d.register).toHaveBeenCalledWith(crewShareRegistrationBody(request, file));
    expect(d.log).not.toHaveBeenCalled();
  });

  it('does nothing after Cancel', async () => {
    const d = deps({ confirm: vi.fn(async () => 1) });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({ outcome: 'cancelled' });
    expect(d.register).not.toHaveBeenCalled();
  });

  it('treats a dialog that fails as Cancel', async () => {
    const d = deps({
      confirm: vi.fn(async () => {
        throw new Error('window gone');
      }),
    });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({ outcome: 'cancelled' });
    expect(d.register).not.toHaveBeenCalled();
  });

  it('never shows a dialog for a refused file', async () => {
    await fsp.mkdir(path.join(root, 'folder'));
    const d = deps();
    await expect(
      shareDroppedFile({ ...request, path: path.join(root, 'folder') }, d)
    ).resolves.toEqual({
      outcome: 'refused',
      message: crewShareCopy.folder('folder'),
    });
    expect(d.confirm).not.toHaveBeenCalled();
    expect(d.register).not.toHaveBeenCalled();
  });

  it('does not ask when the window is already gone, or register when it closed during the dialog', async () => {
    const gone = deps({ isClosed: vi.fn(() => true) });
    await expect(shareDroppedFile(request, gone)).resolves.toEqual({ outcome: 'cancelled' });
    expect(gone.confirm).not.toHaveBeenCalled();

    let closed = false;
    const closing = deps({
      isClosed: vi.fn(() => closed),
      confirm: vi.fn(async () => {
        closed = true;
        return CREW_SHARE_BUTTON_SHARE;
      }),
    });
    await expect(shareDroppedFile(request, closing)).resolves.toEqual({ outcome: 'cancelled' });
    expect(closing.register).not.toHaveBeenCalled();
  });

  it('refuses a file edited while the dialog was open', async () => {
    const d = deps({
      confirm: vi.fn(async () => {
        await fsp.writeFile(file, 'different length\n');
        return CREW_SHARE_BUTTON_SHARE;
      }),
    });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({
      outcome: 'refused',
      message: crewShareCopy.changed('growth.csv'),
    });
    expect(d.register).not.toHaveBeenCalled();
  });

  it.skipIf(!posix)('refuses a file swapped for a shortcut while the dialog was open', async () => {
    const secret = await write('elsewhere/secret.txt', 'PRIVATE!');
    const d = deps({
      confirm: vi.fn(async () => {
        await fsp.rm(file);
        await fsp.symlink(secret, file);
        return CREW_SHARE_BUTTON_SHARE;
      }),
    });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({
      outcome: 'refused',
      message: crewShareCopy.changed('growth.csv'),
    });
    expect(d.register).not.toHaveBeenCalled();
  });

  it('gives back a capability whose file differs from the one the person accepted', async () => {
    const d = deps({
      register: vi.fn(async () => ({
        ok: true,
        body: { capability_id: 'cap-9', name: 'growth.csv', size: 9_999 },
      })),
    });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({
      outcome: 'refused',
      message: crewShareCopy.changed('growth.csv'),
    });
    expect(d.discard).toHaveBeenCalledWith('cap-9');
  });

  it('gives back the capability when the window closed during registration', async () => {
    let closed = false;
    const d = deps({
      isClosed: vi.fn(() => closed),
      register: vi.fn(async () => {
        closed = true;
        return { ok: true, body: { capability_id: 'cap-3', name: 'growth.csv', size: 8 } };
      }),
    });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({ outcome: 'cancelled' });
    expect(d.discard).toHaveBeenCalledWith('cap-3');
  });

  it.each([
    [
      'a privacy change',
      async () => ({
        ok: false,
        body: {
          error:
            'Crew connection privacy changed; refresh the verified workspace before selecting a file',
        },
      }),
      crewShareCopy.privacyChanged,
    ],
    [
      'another refusal, without repeating the daemon text',
      async () => ({ ok: false, body: { error: 'No such file or directory (os error 2)' } }),
      crewShareCopy.daemonRefused('growth.csv'),
    ],
    [
      'an unreachable daemon',
      async () => {
        throw new Error('fetch failed');
      },
      crewShareCopy.daemonRefused('growth.csv'),
    ],
    [
      'an invalid capability',
      async () => ({ ok: true, body: { capability_id: '../x', name: 'growth.csv', size: 8 } }),
      crewShareCopy.daemonRefused('growth.csv'),
    ],
  ])('answers %s with a plain sentence', async (_label, register, message) => {
    const d = deps({ register: vi.fn<ShareDroppedFileDeps['register']>(register) });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({ outcome: 'refused', message });
    expect(d.discard).not.toHaveBeenCalled();
  });

  it('refuses a credential file after Share with the daemon sentence, and keeps no capability (Q3-15)', async () => {
    const d = deps({
      register: vi.fn(async () => ({
        ok: false,
        body: { code: CREW_FILE_IS_CREDENTIAL, error: 'unused' },
      })),
    });
    await expect(shareDroppedFile(request, d)).resolves.toEqual({
      outcome: 'refused',
      message: crewShareCopy.credential('growth.csv'),
    });
    expect(d.confirm).toHaveBeenCalledTimes(1);
    expect(d.register).toHaveBeenCalledTimes(1);
    expect(d.discard).not.toHaveBeenCalled();

    const auto = deps({ autoConfirm: true, register: d.register });
    await expect(shareDroppedFile(request, auto)).resolves.toEqual({
      outcome: 'refused',
      message: crewShareCopy.credential('growth.csv'),
    });
  });

  it('under the development auto-confirm, skips the dialog but logs the path it confirmed', async () => {
    const d = deps({ autoConfirm: true });
    await expect(shareDroppedFile(request, d)).resolves.toMatchObject({ outcome: 'shared' });
    expect(d.confirm).not.toHaveBeenCalled();
    expect(d.log).toHaveBeenCalledTimes(1);
    expect(d.log.mock.calls[0][0]).toContain(file);
    expect(d.log.mock.calls[0][0]).toContain(DEV_AUTO_CONFIRM_SHARE_ENV);
  });

  it('under the development auto-confirm, still refuses what the rules refuse', async () => {
    const d = deps({ autoConfirm: true });
    await expect(shareDroppedFile({ ...request, path: '' }, d)).resolves.toEqual({
      outcome: 'refused',
      message: crewShareCopy.notAFile,
    });
    expect(d.register).not.toHaveBeenCalled();
    expect(d.log).not.toHaveBeenCalled();
  });
});

describe('resolveDevAutoConfirmShare', () => {
  const allowed = {
    value: '1',
    isPackaged: false,
    developmentProfileRoot: '/tmp/profiles/qa',
    testDriverEnabled: true,
    sharedDaemonEnabled: true,
  };

  it('turns on only with every condition of the approval stdin and the exact switch', () => {
    const gate = resolveDevAutoConfirmShare(allowed);
    expect(gate.enabled).toBe(true);
    expect(gate.notice).toContain('Every confirmed path is logged');
  });

  it('is silently off when the switch is absent', () => {
    expect(resolveDevAutoConfirmShare({ ...allowed, value: undefined })).toEqual({
      enabled: false,
    });
    expect(resolveDevAutoConfirmShare({ ...allowed, value: '' })).toEqual({ enabled: false });
  });

  it.each(['0', 'true', 'yes', ' 1', '1 ', '11'])('stays off, and says so, for %j', (value) => {
    const gate = resolveDevAutoConfirmShare({ ...allowed, value });
    expect(gate.enabled).toBe(false);
    expect(gate.notice).toContain('must be exactly 1');
  });

  it.each([
    ['a packaged app', { isPackaged: true }],
    ['no development profile', { developmentProfileRoot: undefined }],
    ['no ENABLE_PLAYWRIGHT', { testDriverEnabled: false }],
    ['no shared daemon', { sharedDaemonEnabled: false }],
  ])('stays off, and says so, with %s', (_label, change) => {
    const gate = resolveDevAutoConfirmShare({ ...allowed, ...change });
    expect(gate.enabled).toBe(false);
    expect(gate.notice).toContain('the native share confirmation stays on');
  });
});

describe('CrewSharePending', () => {
  it('holds one confirmation per window and frees it after', () => {
    const pending = new CrewSharePending();
    expect(pending.enter(1)).toBe(true);
    expect(pending.enter(1)).toBe(false);
    expect(pending.enter(2)).toBe(true);
    pending.leave(1);
    expect(pending.enter(1)).toBe(true);
  });
});
