/**
 * The operating system's microphone setting, as the desktop main process sees
 * it (DD-65), and the test switches that replace the microphone with a WAV file.
 *
 * Deliberately separate from `copilotPermissionSettings.ts`: that helper refuses
 * on an external backend and off macOS, because Copilot's permissions belong to
 * the computer the backend runs on. The microphone always belongs to the
 * computer the app runs on, whatever backend the window talks to, so dictation
 * never asks the backend which settings page to open.
 */

/** What `systemPreferences.getMediaAccessStatus('microphone')` can answer, plus Linux's `unknown`. */
export type MicAccessStatus = 'not-determined' | 'granted' | 'denied' | 'restricted' | 'unknown';

const KNOWN_STATUSES: readonly MicAccessStatus[] = [
  'not-determined',
  'granted',
  'denied',
  'restricted',
  'unknown',
];

/**
 * The status `dictation:mic-status` answers. Linux has no per app microphone
 * setting that Electron can read, so it is always `unknown`; any value Electron
 * adds later reads as `unknown` too rather than reaching the renderer raw.
 */
export function normalizeMicAccessStatus(platform: string, raw: unknown): MicAccessStatus {
  if (platform !== 'darwin' && platform !== 'win32') return 'unknown';
  return KNOWN_STATUSES.includes(raw as MicAccessStatus) ? (raw as MicAccessStatus) : 'unknown';
}

/**
 * The page of the system settings that holds the microphone switch, or `null`
 * where there is no such page to open (Linux), in which case the interface
 * offers no **Open settings** button.
 */
export function micSettingsUrl(platform: string): string | null {
  switch (platform) {
    case 'darwin':
      return 'x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone';
    case 'win32':
      return 'ms-settings:privacy-microphone';
    default:
      return null;
  }
}

/** The environment variable that names the WAV a development build hears instead of a microphone. */
export const FAKE_MIC_WAV_ENV = 'BIOROUTER_TEST_FAKE_MIC_WAV';

export type CommandLineSwitch = { name: string; value?: string };

/**
 * Chromium switches that feed a WAV file to `getUserMedia` in place of a
 * microphone, for tests that need real audio through the real permission
 * handler. Empty for a packaged app, and empty unless the variable names a file.
 *
 * Two switches, never a third:
 *
 * - `use-fake-device-for-media-stream` and `use-file-for-fake-audio-capture`
 *   (with `%noloop`, so the file plays once and then the device is silent)
 *   replace the device and nothing else. The permission handler still decides,
 *   so a test still sees audio with video and a foreign document refused.
 * - Chromium's switch that fakes the permission prompt as well is never added
 *   (a source guard fails on its name): measured on Electron 39.8.10, it
 *   bypassed the session's permission handler entirely, with zero handler calls
 *   and audio plus video granted under a deny all policy.
 *
 * On macOS the audio service's sandbox cannot read the WAV, so the device
 * delivers digital silence (RMS 0, measured); `AudioServiceSandbox` is added to
 * whatever `disable-features` already holds. The renderer's own sandbox is
 * untouched.
 */
export function fakeMicrophoneSwitches(options: {
  isPackaged: boolean;
  wavPath: string | undefined;
  platform: string;
  existingDisabledFeatures?: string;
}): CommandLineSwitch[] {
  const wav = options.wavPath?.trim();
  if (options.isPackaged || !wav) return [];
  const switches: CommandLineSwitch[] = [
    { name: 'use-fake-device-for-media-stream' },
    { name: 'use-file-for-fake-audio-capture', value: `${wav}%noloop` },
  ];
  if (options.platform === 'darwin') {
    const features = (options.existingDisabledFeatures ?? '')
      .split(',')
      .map((feature) => feature.trim())
      .filter(Boolean);
    if (!features.includes('AudioServiceSandbox')) features.push('AudioServiceSandbox');
    switches.push({ name: 'disable-features', value: features.join(',') });
  }
  return switches;
}
