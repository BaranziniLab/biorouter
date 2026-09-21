import { describe, expect, it } from 'vitest';
import { readFileSync } from 'fs';
import { resolve } from 'path';

const forgeConfig = readFileSync(resolve(__dirname, '../../forge.config.ts'), 'utf8');

describe('forge win32metadata', () => {
  // Squirrel takes the Start Menu FOLDER from the exe's version-resource
  // CompanyName, not from the nupkg metadata. With `win32metadata` unset,
  // electron-packager writes Electron's own default, and the 1.91.0 installer
  // (the first release to ship one) filed Biorouter under "GitHub, Inc" in
  // every Windows user's Start Menu. The nupkg was already correct, which is
  // exactly why nobody noticed until a real installer existed.
  it('names the company, so the Start Menu folder is ours', () => {
    expect(forgeConfig).toMatch(/win32metadata:\s*\{/);
    expect(forgeConfig).toMatch(/CompanyName:\s*'Baranzini Lab, UCSF'/);
  });

  it('never carries the vendor default that caused this', () => {
    expect(forgeConfig).not.toMatch(/CompanyName:\s*['"]GitHub/i);
  });

  // A win32metadata block that exists but omits CompanyName would satisfy a
  // shape check while restoring the defect, so pin the key itself.
  it('keeps CompanyName inside the win32metadata block', () => {
    const block = /win32metadata:\s*\{([^}]*)\}/.exec(forgeConfig);
    expect(block, 'win32metadata block not found in forge.config.ts').not.toBeNull();
    expect(block?.[1]).toMatch(/CompanyName/);
  });
});
