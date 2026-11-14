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
  /** The tab list's accessible name (the band's title names the page, not its switch). */
  tabsLabel: 'Settings sections',
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
    'When an agent opens a chat or starts a subagent, you get a notification instead of a tab. Subagents still run; open them from the chat summary.',
  /**
   * The daemon reads it (`BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS`); off by default. "Never open
   * tabs automatically" wins on the daemon, so the help names it instead of disabling the switch.
   */
  openSubagentTabs: 'Open subagent tabs automatically',
  openSubagentTabsHelp:
    'When off, subagents run in the background and the chat summary lists them. When on, each opens in its own tab, unless “Never open tabs automatically” is on.',
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
  preparing: (version: string | null | undefined) =>
    `Preparing ${version ? `version ${version}` : 'the update'} to install…`,
  restarting: 'Restarting to update…',
  restartToUpdate: 'Restart to update',
  tryAgain: 'Try again',
  failed: 'Could not complete the update',
  downloadFromSite: 'Download from Biorouter',
  feedback: 'Feedback',
  /** The in-chat reporter, behind the Feedback row's InfoTip. */
  feedbackHelp: 'To report a bug from a chat, type /bug there.',
  reportBug: 'Report a bug',
  requestFeature: 'Request a feature',
  /** The accessible description of a control that leaves the app, as the glyph says it to the eye. */
  opensInBrowser: 'Opens in your browser',
} as const;

/**
 * Server, between About and the Danger zone (PR-29, S17). The row label is pinned by copy that
 * points people at it ("Settings > App > Use external server": MSG-REMOTE-NO-KEY and the tray's
 * old backend card), so a rename changes those sentences too.
 */
export const serverCopy = {
  section: 'Server',
  sectionHelp:
    'Biorouter starts its own server on this computer unless you connect it to one running elsewhere.',
  useExternal: 'Use external server',
  useExternalHelp:
    'Connect to a Biorouter server running elsewhere. New windows use it after you restart Biorouter.',
  url: 'Server URL',
  urlPlaceholder: 'http://127.0.0.1:3000',
  urlWrongScheme: 'The address must start with http or https',
  urlInvalid: 'This is not a valid address',
  secret: 'Secret Key',
  secretPlaceholder: 'The server’s secret key',
  secretHelp: 'The secret key the server was started with (BIOROUTER_SERVER__SECRET_KEY).',
  userActionKey: 'User Action Key',
  userActionKeyPlaceholder: 'The key the server was started with',
  userActionKeyHelp:
    'Proves a request came from you rather than from the model. Without it, private chats cannot be opened, branched or reported through this server.',
  restartLead: 'Note:',
  restart:
    'Changes take effect after you restart Biorouter. New chat windows connect to the external server.',
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
  /** The dialog's one destructive button; it reads `resetEverything` when every box is checked. */
  reset: 'Reset',
  resetEverything: 'Reset everything',
  resetting: 'Resetting…',
  complete: 'Reset complete. Factory defaults have been restored for the selected areas.',
  completeToastTitle: 'Reset complete',
  completeToast: (count: number) => `${count} ${count === 1 ? 'area was' : 'areas were'} restored.`,
  failedToastTitle: 'Reset failed',
  failedFallback: 'Biorouter could not reset the selected data.',
} as const;
