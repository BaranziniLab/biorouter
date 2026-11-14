import { describe, expect, it } from 'vitest';
import {
  fakeMicrophoneSwitches,
  micSettingsUrl,
  normalizeMicAccessStatus,
} from './micPermissionSettings';

describe('microphone settings page', () => {
  it('opens the fixed microphone pages on macOS and Windows, and none on Linux', () => {
    expect(micSettingsUrl('darwin')).toBe(
      'x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone'
    );
    expect(micSettingsUrl('win32')).toBe('ms-settings:privacy-microphone');
    expect(micSettingsUrl('linux')).toBeNull();
    expect(micSettingsUrl('freebsd')).toBeNull();
  });
});

describe('microphone access status', () => {
  it('passes the five known answers through on macOS and Windows', () => {
    for (const status of ['not-determined', 'granted', 'denied', 'restricted', 'unknown']) {
      expect(normalizeMicAccessStatus('darwin', status)).toBe(status);
      expect(normalizeMicAccessStatus('win32', status)).toBe(status);
    }
  });
  it('is unknown on Linux and for anything Electron might add later', () => {
    expect(normalizeMicAccessStatus('linux', 'granted')).toBe('unknown');
    expect(normalizeMicAccessStatus('darwin', 'limited')).toBe('unknown');
    expect(normalizeMicAccessStatus('darwin', undefined)).toBe('unknown');
  });
});

describe('fake microphone switches', () => {
  it('are empty for a packaged app, whatever the environment says', () => {
    expect(
      fakeMicrophoneSwitches({ isPackaged: true, wavPath: '/tmp/tone.wav', platform: 'darwin' })
    ).toEqual([]);
  });

  it('are empty unless the variable names a file', () => {
    for (const wavPath of [undefined, '', '   ']) {
      expect(fakeMicrophoneSwitches({ isPackaged: false, wavPath, platform: 'linux' })).toEqual([]);
    }
  });

  it('replace the device with the WAV, played once, and nothing else on Linux', () => {
    expect(
      fakeMicrophoneSwitches({ isPackaged: false, wavPath: '/tmp/tone.wav', platform: 'linux' })
    ).toEqual([
      { name: 'use-fake-device-for-media-stream' },
      { name: 'use-file-for-fake-audio-capture', value: '/tmp/tone.wav%noloop' },
    ]);
  });

  it('lift the audio service sandbox on macOS, keeping features already disabled', () => {
    const switches = fakeMicrophoneSwitches({
      isPackaged: false,
      wavPath: '/tmp/tone.wav',
      platform: 'darwin',
      existingDisabledFeatures: 'SomethingElse',
    });
    expect(switches).toContainEqual({
      name: 'disable-features',
      value: 'SomethingElse,AudioServiceSandbox',
    });
    expect(
      fakeMicrophoneSwitches({
        isPackaged: false,
        wavPath: '/tmp/tone.wav',
        platform: 'darwin',
        existingDisabledFeatures: 'AudioServiceSandbox',
      })
    ).toContainEqual({ name: 'disable-features', value: 'AudioServiceSandbox' });
  });

  it('never fake the permission prompt', () => {
    const names = fakeMicrophoneSwitches({
      isPackaged: false,
      wavPath: '/tmp/tone.wav',
      platform: 'darwin',
    }).map((entry) => entry.name);
    expect(names.some((name) => name.includes('fake-ui'))).toBe(false);
  });
});
