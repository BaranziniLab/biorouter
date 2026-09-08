import { afterEach, describe, expect, it, vi } from 'vitest';
import { getExtensionScrollBehavior } from './ExtensionsView';

describe('getExtensionScrollBehavior', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    Reflect.deleteProperty(window, 'matchMedia');
  });

  it('uses instant scrolling when reduced motion is requested', () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({ matches: true }) as never),
    });

    expect(getExtensionScrollBehavior()).toBe('auto');
  });

  it('uses smooth scrolling otherwise', () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({ matches: false }) as never),
    });

    expect(getExtensionScrollBehavior()).toBe('smooth');
  });
});
