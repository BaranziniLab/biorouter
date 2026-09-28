import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { KEY_STORAGE_NOTICE, SecureStorageNotice } from './SecureStorageNotice';

// The daemon keeps provider keys in the OS credential store only when one
// answers; every unpackaged build, dev profile, headless Linux, SSH, WSL and
// `biorouter serve` session writes them to a plaintext `secrets.yaml`. The
// renderer is not told which, so no copy may promise the keychain or
// encryption (W2-PRV-3).
describe('SecureStorageNotice', () => {
  it('says where keys are kept without promising the keychain or encryption', () => {
    render(<SecureStorageNotice />);
    const notice = screen.getByText(KEY_STORAGE_NOTICE);
    expect(notice).toBeInTheDocument();
    expect(KEY_STORAGE_NOTICE).toMatch(/when one is available, otherwise/);
    expect(KEY_STORAGE_NOTICE).not.toMatch(/encrypt/i);
    expect(KEY_STORAGE_NOTICE).not.toMatch(/stored securely in the keychain/i);
  });

  it('leaves no provider or onboarding screen claiming encryption or the keychain outright', () => {
    const roots = [join(__dirname, '../../..'), join(__dirname, '../../../../onboarding')];
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) {
          walk(path);
        } else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) {
          const source = readFileSync(path, 'utf8');
          if (/encrypted and stored|stored securely in the keychain/i.test(source)) {
            offenders.push(path);
          }
        }
      }
    };
    roots.forEach(walk);
    expect(offenders).toEqual([]);
  });
});
