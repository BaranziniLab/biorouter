/**
 * The macOS System Settings panes Biorouter Copilot's setup opens (local backend only).
 *
 * ⚠ Both grants belong to the Biorouter Copilot bundle inside the app, never to Biorouter itself
 * (AD-04 in docs/design/copilot-agent-desktop.md): the engine runs with responsibility disclaimed,
 * so a command Biorouter's shell starts can never borrow them. A pane lists Biorouter Copilot only
 * after the engine has asked, which is what setup's Request access does
 * (`POST /agent/computer_use/setup/request`); these links are for a grant that was refused.
 */
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
