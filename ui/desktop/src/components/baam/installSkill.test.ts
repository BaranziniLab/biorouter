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
      installed: [{ name: 'single-cell', kind: 'single', skills: ['single-cell'] }],
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
    expect(result.installed).toEqual([{ name: 'single-cell', kind: 'bundle', skills: components }]);
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
});
