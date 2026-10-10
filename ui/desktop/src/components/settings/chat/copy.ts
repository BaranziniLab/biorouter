/**
 * The words of Settings > Chat that WS-SETTINGS-A renders: Approvals, Display, Project, the
 * section headers over WS-SETTINGS-B's rows, and the tool permission dialogs (spec §3.13).
 *
 * Tests import these instead of retyping them. Section help is InfoTip text: full sentences, at
 * most two. Menu descriptions are one line with no trailing period.
 */

export const approvalsCopy = {
  /** Renamed from "Mode", which collided with the light and dark theme. */
  section: 'Approvals',
  mode: 'Approval mode',
  modeHelp: 'How much Biorouter does before it asks you.',
  /** The four modes, in menu order. The keys are stored in `BIOROUTER_MODE`; never rename. */
  modes: [
    {
      key: 'auto',
      label: 'Autonomous',
      description: 'Use tools and change files without asking first',
    },
    {
      key: 'approve',
      label: 'Manual',
      description: 'Ask before using tools, extensions or changing files',
    },
    {
      key: 'smart_approve',
      label: 'Smart',
      description: 'Ask only when an action’s risk needs your approval',
    },
    {
      key: 'chat',
      label: 'Chat only',
      description: 'Talk to the model without tools or extensions',
    },
  ],
  toolPermissions: 'Tool permissions',
  toolPermissionsHelp: 'Choose, per tool, what Manual and Smart modes allow.',
  edit: 'Edit…',
  /** The Edit button's name: its own word plus what it edits (label in name). */
  editToolPermissions: 'Edit tool permissions',
  maxTurns: 'Max turns',
  maxTurnsHelp: 'Agent turns before Biorouter asks you to continue.',
} as const;

export type ApprovalModeKey = (typeof approvalsCopy.modes)[number]['key'];

export const displayCopy = {
  section: 'Display',
  toolCallDetails: 'Tool call details',
  toolCallDetailsHelp: 'Whether tool calls in a chat start open or closed.',
  /** The keys are stored in `localStorage.response_style`; never rename. */
  toolCallOptions: [
    { key: 'detailed', label: 'Expanded' },
    { key: 'concise', label: 'Collapsed' },
  ],
  spellcheck: 'Spellcheck',
  spellcheckHelp: 'Checks spelling in the chat input.',
  restartToApply: 'Restart to apply',
} as const;

export const chatSectionsCopy = {
  capabilities: 'Capabilities',
  /**
   * ⚠ Keep "new chats start with" and "Existing chats keep their current":
   * `ChatSettingsSection.copy.test.ts` pins both, because the switches are defaults for NEW
   * chats and a person must not read them as changing the chat they have open.
   */
  capabilitiesHelp:
    'Choose which built-in abilities new chats start with. Existing chats keep their current capabilities.',
  contexts: 'Contexts',
  contextsHelp:
    'Skills that ship with Biorouter. Office instructions load only for the tasks that need them.',
  appSdk: 'App SDK',
  appSdkHelp:
    'Opt-in safety frameworks for Agent Drafter apps. They are off by default and never apply to normal chat.',
} as const;

export const projectCopy = {
  section: 'Project',
  hints: 'Project hints',
  hintsHelp: 'Extra context Biorouter reads from your project’s .biorouterhints file.',
  edit: 'Edit…',
  editHints: 'Edit project hints',
} as const;

export const permissionDialogCopy = {
  rulesTitle: 'Tool permissions',
  rulesSubtitle: 'How Manual and Smart modes treat each enabled extension’s tools.',
  rulesListLabel: 'Enabled extension permissions',
  loadingExtensions: 'Loading enabled extensions…',
  extensionsFailed: 'Enabled extensions could not be loaded.',
  noExtensions: 'No enabled extension has tools to configure.',
  tryAgain: 'Try again',
  toolsSubtitle: 'How this extension’s tools behave in Manual and Smart modes.',
  loadingTools: 'Loading tools…',
  toolsFailed: 'Tools could not be loaded',
  toolsFailedHelp: 'Check that the extension is installed and can start, then try again.',
  noTools: 'No configurable tools',
  noToolsHelp: 'This extension loaded, but it has no tools.',
  saveFailed: 'Permissions could not be saved.',
  cancel: 'Cancel',
  save: 'Save changes',
  saving: 'Saving…',
  levels: [
    { value: 'always_allow', label: 'Always allow' },
    { value: 'ask_before', label: 'Ask before' },
    { value: 'never_allow', label: 'Never allow' },
  ],
} as const;

export const hintsDialogCopy = {
  title: 'Project hints',
  /** Before the file's path in the dialog's subtitle. */
  found: 'Editing',
  newFile: 'New file at',
  field: 'Hints',
  helper: 'Used by new chats while the Developer extension is on.',
  placeholder: 'Language, frameworks, coding style, important files…',
  readError: (message: string) => `Could not read .biorouterhints: ${message}`,
  accessFailed: 'Biorouter could not open the file.',
  saveFailed: 'Biorouter could not save the file.',
  saved: 'Saved',
  close: 'Close',
  save: 'Save',
  saving: 'Saving…',
} as const;
