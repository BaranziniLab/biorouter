export function copilotPermissionSettingsUrl(
  platform: string,
  externalBackend: boolean,
  permission: unknown
): string {
  if (externalBackend) {
    throw new Error('Change OS permissions on the backend computer, then check again.');
  }
  if (platform !== 'darwin') {
    throw new Error('Permission settings shortcuts are available only on macOS.');
  }
  switch (permission) {
    case 'accessibility':
      return 'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility';
    case 'screen_recording':
      return 'x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture';
    default:
      throw new Error('Unknown Biorouter Copilot permission.');
  }
}

export class CopilotPermissionSettings {
  private readonly externalBackends = new WeakMap<object, boolean>();

  bindWindow(window: object, externalBackend: boolean): void {
    this.externalBackends.set(window, externalBackend);
  }

  urlForWindow(window: object, platform: string, permission: unknown): string {
    const externalBackend = this.externalBackends.get(window);
    if (externalBackend === undefined) {
      throw new Error('Open permission settings from a connected Biorouter chat window.');
    }
    return copilotPermissionSettingsUrl(platform, externalBackend, permission);
  }
}
