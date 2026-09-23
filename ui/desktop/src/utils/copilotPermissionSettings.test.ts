import { describe, expect, it } from 'vitest';
import {
  CopilotPermissionSettings,
  copilotPermissionSettingsUrl,
} from './copilotPermissionSettings';

describe('Copilot permission settings destinations', () => {
  it('allows only fixed macOS privacy panes for a local backend', () => {
    expect(copilotPermissionSettingsUrl('darwin', false, 'accessibility')).toBe(
      'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility'
    );
    expect(copilotPermissionSettingsUrl('darwin', false, 'screen_recording')).toBe(
      'x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture'
    );
    expect(() => copilotPermissionSettingsUrl('darwin', false, 'https://example.com')).toThrow(
      'Unknown'
    );
  });
  it('refuses remote backends and other platforms', () => {
    expect(() => copilotPermissionSettingsUrl('darwin', true, 'accessibility')).toThrow(
      'backend computer'
    );
    for (const platform of ['win32', 'linux']) {
      expect(() => copilotPermissionSettingsUrl(platform, false, 'accessibility')).toThrow(
        'only on macOS'
      );
    }
  });
});

describe('permission settings use the connected window backend', () => {
  it('keeps existing windows bound when saved backend preferences change', () => {
    const registry = new CopilotPermissionSettings();
    const localWindow = {};
    const remoteWindow = {};
    const settings = { externalBackend: false };
    registry.bindWindow(localWindow, settings.externalBackend);
    settings.externalBackend = true;
    registry.bindWindow(remoteWindow, settings.externalBackend);
    expect(registry.urlForWindow(localWindow, 'darwin', 'accessibility')).toContain(
      'Privacy_Accessibility'
    );
    settings.externalBackend = false;
    expect(() => registry.urlForWindow(remoteWindow, 'darwin', 'accessibility')).toThrow(
      'backend computer'
    );
    expect(() => registry.urlForWindow({}, 'darwin', 'accessibility')).toThrow(
      'connected Biorouter chat window'
    );
  });
});
