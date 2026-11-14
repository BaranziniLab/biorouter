/**
 * The words of a `biorouter serve` page's update (SD-19). The titles and
 * buttons are the desktop updater's own (`UpdateAvailableModal`, Settings ›
 * About), so a person who has updated the app reads the same words here. One
 * line is visible; anything longer sits behind an InfoTip or a Details
 * disclosure (the owner's no-walls-of-text rule).
 */
export const serveUpdateCopy = {
  title: {
    checking: 'Checking for updates…',
    downloading: 'Downloading update…',
    ready: 'Update ready to install',
    available: 'Update available',
    installed: 'Update installed',
    installing: 'Updating…',
    restarting: 'Restarting…',
    failed: 'Update failed',
    rolledBack: 'Update didn’t finish',
  },
  current: 'Current',
  newVersion: 'New version',
  // The desktop updater's own line (UPDATE_COPY in UpdateAvailableModal).
  downloadingLine: 'The update is downloading in the background, so you can keep working.',
  downloadingLabel: (version: string | null | undefined) =>
    `Downloading update${version ? ` ${version}` : ''}`,
  readyLine: (version: string | null | undefined) =>
    `Biorouter restarts on ${version ?? 'the new version'} and this page reloads by itself.`,
  readyHelp:
    'Chats, settings and extensions are kept. Anything still running, such as a chat that is working, stops.',
  runningLine: (chats: number, jobs = 0) => {
    if (jobs === 0) {
      return chats === 1
        ? '1 chat is still working and will stop.'
        : `${chats} chats are still working and will stop.`;
    }
    const jobWords = jobs === 1 ? '1 background job' : `${jobs} background jobs`;
    if (chats === 0) {
      return jobs === 1
        ? `${jobWords} is still running and will stop.`
        : `${jobWords} are still running and will stop.`;
    }
    const chatWords = chats === 1 ? '1 chat' : `${chats} chats`;
    return `${chatWords} and ${jobWords} are still running and will stop.`;
  },
  adminLine: 'Updating needs administrator rights on this server.',
  adminCommandLabel: 'update command',
  adminHelp:
    'Run it in a terminal on the server: it checks the download’s SHA-256, then installs it. Then come back here to restart.',
  installedLine: (version: string | null | undefined) =>
    `Biorouter ${version ?? ''} is installed. Restart to use it.`.replace('  ', ' '),
  manual: {
    source: 'This copy was built from source. Update it with git pull and a new build.',
    windows: 'Install the update with the Biorouter app on this computer.',
    unrecognised: 'This copy can’t update itself. Install the new version from the download page.',
    not_writable:
      'This account can’t replace the app. Install the new version from the download page.',
  } as Record<string, string>,
  installingLine: (version: string | null | undefined) => `Installing ${version ?? 'the update'}…`,
  restartingLine: 'This page reloads when Biorouter is back.',
  slowLine: 'Biorouter hasn’t come back yet. Check the server’s terminal.',
  failedFallback: 'Something went wrong while updating.',
  rolledBackLine: (from: string, to: string) =>
    `Biorouter ${to} didn’t start, so ${from} was put back.`,
  failedLine: (from: string, to: string) =>
    `Biorouter ${to} didn’t start, and ${from} couldn’t be put back.`,
  whatHappened: 'What happened',
  updatedToast: (version: string) => `Biorouter updated to ${version}`,
  testFeed: 'From a test feed',
  button: {
    notNow: 'Not now',
    later: 'Later',
    restartAndUpdate: 'Restart & update',
    restart: 'Restart',
    checkAgain: 'Check again',
    tryAgain: 'Try again',
    downloadFromSite: 'Download from Biorouter',
    close: 'Close',
  },
  settings: {
    checkForUpdates: 'Check for updates',
    checking: 'Checking for updates…',
    upToDate: 'Biorouter is up to date',
    downloading: (version: string | null | undefined) => `Downloading ${version ?? 'update'}…`,
    ready: (version: string | null | undefined) =>
      version ? `Version ${version} is ready` : 'The update is ready',
    needsAdmin: (version: string | null | undefined) =>
      `Version ${version ?? 'new'} needs an administrator`,
    installed: (version: string | null | undefined) => `Version ${version ?? 'new'} is installed`,
    manual: (version: string | null | undefined) => `Version ${version ?? 'new'} is available`,
    restarting: 'Restarting to update…',
    failed: 'Could not complete the update',
    restartToUpdate: 'Restart to update',
    update: 'Update…',
    versionHelp: 'Biorouter checks for updates on this server. Installing one restarts it.',
    unavailableHelp:
      'This server updates outside the browser: upgrade Biorouter on it and restart biorouter serve.',
  },
} as const;

export const DOWNLOAD_WEBSITE_URL = 'https://biorouter.ucsf.edu/download';
