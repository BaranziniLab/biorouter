/**
 * The words of the Settings shell and its App tab (codex simplicity redesign, spec §3.13).
 *
 * Tests import these instead of retyping them. A switch's accessible name IS its row label
 * (spec 2.6: names equal labels, so a person driving the app by voice says what they read), so a
 * label changed here renames the control and every test that queries it, together.
 *
 * Help strings are InfoTip text: full sentences, at most two, about 160 characters. Labels and
 * one-line descriptions take no trailing period.
 */

export const settingsShellCopy = {
  title: 'Settings',
  /** Pinned by daemon copy ("Settings > Models", "Settings > App > Privacy"); never rename. */
  tabs: { models: 'Models', chat: 'Chat', app: 'App' },
} as const;

export const generalCopy = {
  section: 'General',
  notifications: 'Notifications',
  notificationsHelp: 'Managed by your operating system.',
  /** macOS 13 renamed System Preferences to System Settings. */
  openNotificationsMac: 'Open System Settings',
  openNotificationsOther: 'Open system settings',
  menuBar: 'Show in menu bar',
  dock: 'Show in Dock',
  preventSleep: 'Prevent sleep while running',
  preventSleepHelp:
    'Keeps your computer awake while Biorouter runs a task. The screen can still lock.',
  /** Folded in from the old one-row Workspace section. The daemon reads the switch. */
  announceOnly: 'Never open tabs automatically',
  announceOnlyHelp:
    'When an agent opens a chat or starts a subagent, you get a notification instead of a tab. Subagents still run; open them from History.',
  showCosts: 'Show costs',
  showCostsHelp: 'Model prices and usage costs, in chats and on this page.',
} as const;

export const appearanceCopy = {
  section: 'Appearance',
  theme: 'Theme',
  themeOptions: { light: 'Light', dark: 'Dark', system: 'System' },
  palette: 'Color palette',
  textSize: 'Text size',
  textSizeOptions: { standard: 'Standard', large: 'Large', larger: 'Larger' },
} as const;

export const aboutCopy = {
  section: 'About',
  version: 'Version',
  /** Shown when the build reports no version at all. */
  development: 'Development',
  versionHelp: 'Biorouter downloads updates on its own. Restart to use the new version.',
  checkForUpdates: 'Check for updates',
  checking: 'Checking for updates…',
  upToDate: 'Biorouter is up to date',
  downloading: (version: string | null | undefined) => `Downloading ${version ?? 'update'}…`,
  downloadingLabel: (version: string | null | undefined) => `Downloading ${version ?? 'update'}`,
  ready: (version: string | null | undefined) =>
    version ? `Version ${version} is ready` : 'The update is ready',
  restartToUpdate: 'Restart to update',
  failed: 'Could not complete the update',
  downloadFromSite: 'Download from Biorouter',
  feedback: 'Feedback',
  reportBug: 'Report a bug',
  requestFeature: 'Request a feature',
} as const;

export const resetCopy = {
  section: 'Danger zone',
  row: 'Reset data',
  rowHelp:
    'Clears the areas you choose and restores built-in content. Models, credentials and preferences are kept.',
  open: 'Reset…',
  dialogTitle: 'Reset data',
  /** Essential, so it stays visible in the dialog (principle 2: a destructive consequence). */
  permanence: 'Resetting is permanent. Export anything you want to keep first.',
  selectAll: 'Select all',
  clear: 'Clear',
  selectedCount: (selected: number, total: number) => `${selected} of ${total} selected`,
  selectName: (title: string) => `Select ${title} for reset`,
  cancel: 'Cancel',
  resetSelected: 'Reset selected',
  resetEverything: 'Reset everything',
  resetting: 'Resetting…',
  complete: 'Reset complete. Factory defaults have been restored for the selected areas.',
  completeToastTitle: 'Reset complete',
  completeToast: (count: number) => `${count} ${count === 1 ? 'area was' : 'areas were'} restored.`,
  failedToastTitle: 'Reset failed',
  failedFallback: 'Biorouter could not reset the selected data.',
} as const;
