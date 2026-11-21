import { describe, expect, it } from 'vitest';
import { HOST_MANAGED_MODEL_HELP, HOST_MANAGED_ONBOARDING_HELP } from './hostManagedModelCopy';

describe('headless provider setup guidance', () => {
  it.each([HOST_MANAGED_MODEL_HELP, HOST_MANAGED_ONBOARDING_HELP])(
    'names a secure browser route before the host fallback',
    (help) => {
      expect(help).toMatch(/HTTPS/);
      expect(help).toMatch(/SSH/);
      expect(help).toMatch(/SSM/);
      expect(help).toMatch(/loopback/);
      expect(help.indexOf('HTTPS')).toBeLessThan(help.indexOf('biorouter configure'));
      expect(help).toMatch(/private conversation/);
    }
  );
});
