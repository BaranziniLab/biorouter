import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it } from 'vitest';
import {
  clearRetiredDictationSettings,
  RETIRED_DICTATION_SETTINGS_KEY,
} from './retiredDictationSettings';

beforeEach(() => {
  localStorage.clear();
});

describe('clearRetiredDictationSettings (DD-89)', () => {
  it('removes the inherited cloud dictation settings and nothing else', () => {
    localStorage.setItem(
      RETIRED_DICTATION_SETTINGS_KEY,
      JSON.stringify({ enabled: true, provider: 'elevenlabs' })
    );
    localStorage.setItem('biorouter-browser-settings', '{}');

    clearRetiredDictationSettings();

    expect(localStorage.getItem(RETIRED_DICTATION_SETTINGS_KEY)).toBeNull();
    expect(localStorage.getItem('biorouter-browser-settings')).toBe('{}');
  });

  it('is a no-op when the key was never written, so every start can call it', () => {
    clearRetiredDictationSettings();
    clearRetiredDictationSettings();
    expect(localStorage.length).toBe(0);
  });

  it('does not throw when storage is blocked', () => {
    const blocked = {
      removeItem: () => {
        throw new DOMException('blocked', 'SecurityError');
      },
    } as unknown as Storage;
    expect(() => clearRetiredDictationSettings(blocked)).not.toThrow();
  });

  it('names the key the deleted settings hook wrote', () => {
    expect(RETIRED_DICTATION_SETTINGS_KEY).toBe('dictation_settings');
  });

  it('runs at renderer startup', () => {
    const renderer = readFileSync(join(__dirname, '..', 'renderer.tsx'), 'utf8');
    expect(renderer).toMatch(/^\s*clearRetiredDictationSettings\(\);$/m);
  });
});
