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
export function assistedUpdateKind(updatePath: string, platform: typeof process.platform): AssistedUpdateKind {
  const lower = updatePath.toLowerCase();
  if (platform === 'win32') {
    return lower.endsWith('.exe') ? 'windows-installer' : 'windows-zip';
  }
  if (platform === 'linux') {
    return 'linux-package';
  }
  return 'macos-app';
}

/**
 * Whether this kind of update destroys a RUNNING install if the user starts it
 * without quitting first.
 *
 * Squirrel's full install removes the existing app directory before checking
 * whether anything holds a handle in it, so it deletes `Update.exe`, `packages\`
 * and the root stub, then throws. Both shortcuts survive and point at a file
 * that is gone. Re-running with the app closed repairs it, and no user data is
 * at risk (settings and chats live outside the app folder), but the user's
 * experience is "I ran the update and Biorouter will not start".
 */
export function updateRequiresQuitFirst(kind: AssistedUpdateKind): boolean {
  return kind === 'windows-installer';
}

const PRESERVED = 'Your settings, chats and extensions live outside the app folder and are preserved.';

/**
 * The dialog body for a downloaded update.
 *
 * `updatePath` is the file that was actually fetched, so a release that ships
 * only the old zip still gets the old instructions and a release that ships the
 * installer gets the installer ones.
 */
export function assistedUpdateInstructions(updatePath: string, platform: typeof process.platform): string {
  switch (assistedUpdateKind(updatePath, platform)) {
    case 'windows-installer':
      // Squirrel replaces the installed app directory and keeps the existing
      // shortcuts, so there is no folder for the user to swap by hand. The app
      // still has to quit first: it cannot replace itself while running.
      return [
        'The update has been downloaded as an installer.',
        '',
        '1. Click "Open Folder & Quit" to reveal Biorouter Setup and close Biorouter',
        '2. Run the installer once Biorouter has closed',
        '',
        'Biorouter must be closed before the installer runs. Squirrel deletes the existing '
          + 'app directory before it checks whether anything is using it, so running the '
          + 'installer over a running Biorouter leaves a half-removed install and shortcuts '
          + 'that no longer work. Running it again with Biorouter closed repairs that.',
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
