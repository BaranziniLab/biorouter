import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { agentCaptureDirFor } from './biorouterPaths';

/**
 * M1: Electron main writes an agent's screenshots where the daemon reads them, under every
 * profile. The daemon's side is `Paths::in_state_dir("captures")`.
 */
describe('the agent capture folder', () => {
  it('follows BIOROUTER_PATH_ROOT, as the daemon does', () => {
    expect(
      agentCaptureDirFor({
        platform: 'darwin',
        env: { BIOROUTER_PATH_ROOT: '/runs/qa', XDG_STATE_HOME: '/elsewhere' },
        homedir: '/Users/a',
      })
    ).toBe('/runs/qa/state/captures');
  });

  it('is the XDG state folder on macOS and Linux, and the data folder on Windows', () => {
    expect(agentCaptureDirFor({ platform: 'darwin', env: {}, homedir: '/Users/a' })).toBe(
      '/Users/a/.local/state/biorouter/captures'
    );
    expect(
      agentCaptureDirFor({
        platform: 'linux',
        env: { XDG_STATE_HOME: '/xdg/state', BIOROUTER_PATH_ROOT: '   ' },
        homedir: '/home/a',
      })
    ).toBe('/xdg/state/biorouter/captures');
    expect(
      agentCaptureDirFor({
        platform: 'linux',
        env: { XDG_STATE_HOME: 'relative/state' },
        homedir: '/home/a',
      })
    ).toBe('/home/a/.local/state/biorouter/captures');
    expect(
      agentCaptureDirFor({
        platform: 'win32',
        env: { APPDATA: 'C:\\Users\\a\\AppData\\Roaming' },
        homedir: 'C:\\Users\\a',
        pathImpl: path.win32,
        presence: () => 'absent',
      })
    ).toBe('C:\\Users\\a\\AppData\\Roaming\\UCSF\\biorouter\\data\\captures');
  });

  it('names the folder the daemon trusts, and main writes agent captures there', () => {
    const rust = readFileSync(
      path.join(process.cwd(), '..', '..', 'crates/biorouter/src/agents/workspace_extension.rs'),
      'utf8'
    );
    expect(rust).toMatch(/Paths::in_state_dir\("captures"\)/);
    const main = readFileSync(path.join(process.cwd(), 'src', 'main.ts'), 'utf8');
    const body = main.slice(main.indexOf('async function writeAgentScreenshot'));
    expect(body.slice(0, 400)).toContain('ensureAgentCaptureDir()');
    expect(main).not.toMatch(/browser-shot-[^\n]*\n?[^\n]*ensureTempDirExists/);
  });
});
