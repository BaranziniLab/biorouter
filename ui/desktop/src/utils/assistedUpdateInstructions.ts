/**
 * The text the assisted-update dialog shows once an update has been downloaded.
 *
 * ⚠ The instructions must match the ARTIFACT, not the platform.
 *
 * They were keyed on `process.platform` alone, which was correct exactly once.
 * The first version of that dialog told every user to "drag the new
 * Biorouter.app to your Applications folder", and on Windows there is no `.app`
 * and no Applications folder, so the user was asked to quit the app on the
 * strength of steps they could not follow. Keying on the platform fixed that
 * case and reintroduced it the moment one platform started shipping two
 * different artifacts: Windows now publishes both `Biorouter-Setup-<ver>.exe`
 * and `Biorouter-win32-x64-<ver>.zip`, `githubUpdater` prefers the installer,
 * and the platform-keyed text still described extracting a zip that was never
 * downloaded.
 *
 * So the input here is the downloaded file's own path. A new artifact type
 * means a new branch in one pure function with a test beside it, rather than a
 * silent mismatch inside an ipcMain handler that nothing can exercise.
 */

export type AssistedUpdateKind = 'windows-installer' | 'windows-zip' | 'linux-package' | 'macos-app';

/** Classify a downloaded update by its file extension. */
export function assistedUpdateKind(updatePath: string, platform: NodeJS.Platform): AssistedUpdateKind {
  const lower = updatePath.toLowerCase();
  if (platform === 'win32') {
    return lower.endsWith('.exe') ? 'windows-installer' : 'windows-zip';
  }
  if (platform === 'linux') {
    return 'linux-package';
  }
  return 'macos-app';
}

const PRESERVED = 'Your settings, chats and extensions live outside the app folder and are preserved.';

/**
 * The dialog body for a downloaded update.
 *
 * `updatePath` is the file that was actually fetched, so a release that ships
 * only the old zip still gets the old instructions and a release that ships the
 * installer gets the installer ones.
 */
export function assistedUpdateInstructions(updatePath: string, platform: NodeJS.Platform): string {
  switch (assistedUpdateKind(updatePath, platform)) {
    case 'windows-installer':
      // Squirrel replaces the installed app directory and keeps the existing
      // shortcuts, so there is no folder for the user to swap by hand. The app
      // still has to quit first: it cannot replace itself while running.
      return [
        'The update has been downloaded as an installer.',
        '',
        '1. Click "Open Folder" to reveal Biorouter Setup',
        '2. Quit Biorouter (this app will close)',
        '3. Run the installer, which upgrades your existing installation in place',
        '',
        PRESERVED,
      ].join('\n');

    case 'windows-zip':
      return [
        'The update has been downloaded. Biorouter cannot replace itself while it is running on Windows, so the last step is manual:',
        '',
        '1. Click "Open Folder" to reveal the downloaded Biorouter zip',
        '2. Quit Biorouter (this app will close)',
        '3. Extract the zip and replace your existing Biorouter folder with it',
        '4. Launch Biorouter again',
        '',
        PRESERVED,
      ].join('\n');

    case 'linux-package':
      return [
        'The update has been downloaded.',
        '',
        '1. Click "Open Folder" to reveal the downloaded package',
        '2. Quit Biorouter (this app will close)',
        '3. Install the .deb or .rpm with your package manager',
        '4. Launch Biorouter again',
        '',
        'Your settings, chats and extensions are preserved.',
      ].join('\n');

    case 'macos-app':
      return [
        'The update has been downloaded and extracted. To complete the installation:',
        '',
        '1. Click "Open Folder" to view the new Biorouter.app',
        '2. Quit Biorouter (this app will close)',
        '3. Drag the new Biorouter.app to your Applications folder',
        '4. Replace the existing app when prompted',
        '',
        'The update will be available the next time you launch Biorouter.',
      ].join('\n');
  }
}
