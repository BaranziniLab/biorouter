// @vitest-environment node
import { createHash } from 'node:crypto';
import path from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { crewBrokerDirectory, saveCrewBroker, type SaveCrewBrokerDeps } from './crewBrokerExport';

const BYTES = Buffer.from('\x7fELF fake broker');
const SHA = createHash('sha256').update(BYTES).digest('hex');
const DIR = '/app/Resources/bin/crew-broker/linux-x86_64';

function deps(overrides: Partial<SaveCrewBrokerDeps> = {}) {
  const files: Record<string, Buffer> = {
    [path.join(DIR, 'manifest.json')]: Buffer.from(
      JSON.stringify({ version: '1.94.0', sha256: SHA, size: BYTES.length })
    ),
    [path.join(DIR, 'biorouter-crew')]: BYTES,
  };
  return {
    brokerDirectory: vi.fn(() => DIR),
    readFile: vi.fn(async (file: string) => {
      const found = files[file];
      if (!found) throw new Error('ENOENT');
      return found;
    }),
    writeFile: vi.fn(async () => undefined),
    defaultFolder: vi.fn(() => '/Users/alice/Downloads'),
    showSaveDialog: vi.fn(async () => ({
      canceled: false,
      filePath: '/Users/alice/Downloads/biorouter-crew-1.94.0-linux-x86_64',
    })),
    ...overrides,
  } satisfies SaveCrewBrokerDeps;
}

describe('Save Crew’s server program (OW-9)', () => {
  it('saves the bundled broker once it matches the version and digest shown', async () => {
    const d = deps();
    await expect(saveCrewBroker({ version: '1.94.0', sha256: SHA }, d)).resolves.toEqual({
      success: true,
      filePath: '/Users/alice/Downloads/biorouter-crew-1.94.0-linux-x86_64',
    });
    expect(d.showSaveDialog).toHaveBeenCalledWith(
      expect.objectContaining({
        defaultPath: '/Users/alice/Downloads/biorouter-crew-1.94.0-linux-x86_64',
      })
    );
    expect(d.writeFile).toHaveBeenCalledWith(
      '/Users/alice/Downloads/biorouter-crew-1.94.0-linux-x86_64',
      BYTES
    );
  });

  it.each([
    ['another digest than the one shown', { version: '1.94.0', sha256: 'b'.repeat(64) }],
    ['another version than the one shown', { version: '1.93.0', sha256: SHA }],
  ])('saves nothing, and opens no dialog, for %s', async (_case, expected) => {
    const d = deps();
    const result = await saveCrewBroker(expected, d);
    expect(result).toMatchObject({ success: false });
    expect(d.showSaveDialog).not.toHaveBeenCalled();
    expect(d.writeFile).not.toHaveBeenCalled();
  });

  it('refuses bytes that do not hash to their own manifest', async () => {
    const d = deps({
      readFile: vi.fn(async (file: string) =>
        file.endsWith('manifest.json')
          ? Buffer.from(JSON.stringify({ version: '1.94.0', sha256: SHA }))
          : Buffer.from('tampered')
      ),
    });
    await expect(saveCrewBroker({ version: '1.94.0', sha256: SHA }, d)).resolves.toMatchObject({
      success: false,
    });
    expect(d.writeFile).not.toHaveBeenCalled();
  });

  it('saves a development copy without a manifest on its digest alone', async () => {
    const d = deps({
      readFile: vi.fn(async (file: string) => {
        if (file.endsWith('manifest.json')) throw new Error('ENOENT');
        return BYTES;
      }),
    });
    await expect(saveCrewBroker({ version: '0.0.1', sha256: SHA }, d)).resolves.toMatchObject({
      success: true,
    });
    await expect(
      saveCrewBroker({ version: '0.0.1', sha256: 'b'.repeat(64) }, d)
    ).resolves.toMatchObject({ success: false });
  });

  it('refuses a request that is not a version and a digest', async () => {
    const d = deps();
    for (const bad of [null, {}, { version: '1.94.0; rm', sha256: SHA }, { version: '1.94.0' }]) {
      await expect(saveCrewBroker(bad, d)).resolves.toMatchObject({ success: false });
    }
    expect(d.readFile).not.toHaveBeenCalled();
  });

  it('says so when this build carries no broker, and writes nothing when the save is cancelled', async () => {
    await expect(
      saveCrewBroker({ version: '1.94.0', sha256: SHA }, deps({ brokerDirectory: () => null }))
    ).resolves.toMatchObject({ success: false });
    const d = deps({ showSaveDialog: vi.fn(async () => ({ canceled: true })) });
    await expect(saveCrewBroker({ version: '1.94.0', sha256: SHA }, d)).resolves.toEqual({
      success: false,
      canceled: true,
    });
    expect(d.writeFile).not.toHaveBeenCalled();
  });

  it('finds the bundle where the daemon does: the override first, else beside biorouterd', () => {
    const exists = (file: string) => file.startsWith('/dev/broker/') || file.startsWith('/res/');
    expect(
      crewBrokerDirectory(
        { isPackaged: true },
        { BIOROUTER_CREW_BROKER_DIR: '/dev/broker' },
        exists,
        '/res'
      )
    ).toBe('/dev/broker');
    expect(
      crewBrokerDirectory(
        { isPackaged: true },
        { BIOROUTER_CREW_BROKER_DIR: '/missing' },
        exists,
        '/res'
      )
    ).toBeNull();
    expect(crewBrokerDirectory({ isPackaged: true }, {}, exists, '/res')).toBe(
      path.join('/res', 'bin', 'crew-broker', 'linux-x86_64')
    );
    expect(
      crewBrokerDirectory({ isPackaged: false }, {}, () => false, '/res', '/repo/ui/desktop')
    ).toBeNull();
  });
});
