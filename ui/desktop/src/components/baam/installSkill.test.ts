import { beforeEach, describe, expect, it, vi } from 'vitest';
import { installRegistrySkill } from './installSkill';
import type { RegistrySkill } from './registry';

const mocks = vi.hoisted(() => ({
  installSkillPackage: vi.fn(),
  downloadRegistryAsset: vi.fn(),
}));

vi.mock('../../api', () => ({
  installSkillPackage: (...args: unknown[]) => mocks.installSkillPackage(...args),
}));

const skill = { name: 'single-cell', download: 'https://example/x.zip' } as RegistrySkill;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.downloadRegistryAsset.mockResolvedValue({ path: '/tmp/single-cell.zip' });
  // @ts-expect-error test shim
  window.electron = { downloadRegistryAsset: mocks.downloadRegistryAsset };
});

describe('installRegistrySkill', () => {
  /**
   * Parity: a marketplace asset goes through the same import pipeline a
   * repository URL, a local ZIP and the CLI do. It used to be extracted by the
   * renderer and written file by file, so a multi-skill asset got the same
   * flattening a pasted URL did.
   */
  it('installs through the shared importer rather than unzipping in the renderer', async () => {
    mocks.installSkillPackage.mockResolvedValue({
      data: {
        status: 'installed',
        preview: {},
        installed: [{ displayName: 'single-cell', kind: 'single', skills: ['single-cell'] }],
      },
    });
    const result = await installRegistrySkill(skill);
    expect(result).toEqual({
      ok: true,
      name: 'single-cell',
      installed: [
        { name: 'single-cell', kind: 'single', skills: ['single-cell'], replaced: false },
      ],
    });
    expect(mocks.installSkillPackage.mock.calls[0][0].body).toEqual({
      filePath: '/tmp/single-cell.zip',
    });
  });

  /// A marketplace package is ONE installed unit holding several skills. The
  /// modal's toast counts skills from this, so it must come back whole.
  it("returns the daemon's account of a package: its name and every component", async () => {
    const components = ['single-cell-qc', 'single-cell-clustering', 'single-cell-annotation'];
    mocks.installSkillPackage.mockResolvedValue({
      data: {
        status: 'installed',
        preview: {},
        installed: [{ displayName: 'single-cell', kind: 'bundle', skills: components }],
      },
    });
    const result = await installRegistrySkill(skill);
    expect(result.installed).toEqual([
      { name: 'single-cell', kind: 'bundle', skills: components, replaced: false },
    ]);
  });

  it('reports a download failure without calling the importer', async () => {
    mocks.downloadRegistryAsset.mockResolvedValue({ error: 'network down' });
    const result = await installRegistrySkill(skill);
    expect(result).toEqual({ ok: false, name: 'single-cell', error: 'network down' });
    expect(mocks.installSkillPackage).not.toHaveBeenCalled();
  });

  it('surfaces an ambiguous asset as a question rather than resolving it', async () => {
    mocks.installSkillPackage.mockResolvedValue({
      data: {
        status: 'needsChoice',
        planId: 'plan-3',
        preview: {
          ambiguity: { reason: 'Cannot tell', components: ['a', 'b'] },
          components: [{ name: 'a' }, { name: 'b' }],
        },
      },
    });
    const result = await installRegistrySkill(skill);
    expect(result.ok).toBe(false);
    expect(result.needsChoice).toEqual({
      planId: 'plan-3',
      reason: 'Cannot tell',
      components: ['a', 'b'],
    });
  });

  it('reports an install refusal', async () => {
    mocks.installSkillPackage.mockRejectedValue(new Error('disk full'));
    const result = await installRegistrySkill(skill);
    expect(result).toEqual({ ok: false, name: 'single-cell', error: 'disk full' });
  });

  /// `biorouter serve` answers the download with BOTH keys, one of them null.
  /// `'error' in dl` read that as a failure, so in a browser no Browse install
  /// ever reached the importer: "Alignment: failed", and nothing installed.
  it("installs when the download answers with a path and a null error (serve's shape)", async () => {
    mocks.downloadRegistryAsset.mockResolvedValue({
      path: '/tmp/biorouter-registry/ab12/single-cell.zip',
      error: null,
    });
    mocks.installSkillPackage.mockResolvedValue({
      data: { status: 'installed', preview: {}, installed: [] },
    });
    const result = await installRegistrySkill(skill);
    expect(result.ok).toBe(true);
    expect(mocks.installSkillPackage.mock.calls[0][0].body).toEqual({
      filePath: '/tmp/biorouter-registry/ab12/single-cell.zip',
    });
  });

  it('reports a failed download in either shape, with a sentence', async () => {
    mocks.downloadRegistryAsset.mockResolvedValue({
      path: null,
      error: 'Download failed: HTTP 404',
    });
    expect(await installRegistrySkill(skill)).toEqual({
      ok: false,
      name: 'single-cell',
      error: 'Download failed: HTTP 404',
    });
    // An unreachable daemon: the browser shim has no body at all.
    mocks.downloadRegistryAsset.mockResolvedValue(null);
    expect((await installRegistrySkill(skill)).error).toBe('Could not download single-cell');
    expect(mocks.installSkillPackage).not.toHaveBeenCalled();
  });

  /// The generated client throws the response BODY, and this route answers a
  /// refusal with a plain string. Only an `Error` was read, so every explained
  /// refusal surfaced as "Could not install <name>".
  it("keeps the daemon's reason when it refuses the install", async () => {
    mocks.installSkillPackage.mockRejectedValue(
      'could not install `single-cell`: the skills directory is read-only'
    );
    expect((await installRegistrySkill(skill)).error).toBe(
      'could not install `single-cell`: the skills directory is read-only'
    );
    mocks.installSkillPackage.mockRejectedValue({ message: 'nothing selected to install' });
    expect((await installRegistrySkill(skill)).error).toBe('nothing selected to install');
    mocks.installSkillPackage.mockRejectedValue({});
    expect((await installRegistrySkill(skill)).error).toBe('Could not install single-cell');
  });

  it('says when an install replaced one already there', async () => {
    mocks.installSkillPackage.mockResolvedValue({
      data: {
        status: 'installed',
        preview: {},
        installed: [{ displayName: 'single-cell', kind: 'bundle', skills: ['a'], replaced: true }],
      },
    });
    expect((await installRegistrySkill(skill)).installed?.[0].replaced).toBe(true);
  });
});
