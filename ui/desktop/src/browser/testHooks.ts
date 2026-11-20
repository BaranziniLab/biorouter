/**
 * Whether the Biorouter Browser honours its Electron test hooks: fixture host names mapped to
 * loopback (`BIOROUTER_BROWSER_TEST_HOSTS`) and downloads saved without a dialog
 * (`BIOROUTER_BROWSER_TEST_DOWNLOAD_DIR`).
 *
 * ⚠ Both are bypasses by design: a fixture host declared `public` reaches loopback without the
 * person, and a test download skips the save dialog. So they need BOTH an unpackaged app and
 * `BIOROUTER_E2E=1`. A packaged app never reads either variable, whatever its environment holds.
 */
export function browserTestHooksEnabled(
  env: Record<string, string | undefined>,
  isPackaged: boolean
): boolean {
  return !isPackaged && env.BIOROUTER_E2E === '1';
}
