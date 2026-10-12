/**
 * The words of Settings > App > Configuration and its editor. Tests import these instead of
 * retyping them.
 *
 * The section is one row. Its value is the provider the settings below belong to; the sentence
 * that used to sit under the header ("Edit your Biorouter configuration settings (current settings
 * for X)") is the row's InfoTip plus that value.
 *
 * ⚠ Locked, and not in this file: the privacy summary's sentence ("These are changed in …", with
 * the "Go to Privacy" link) is the W2-PRV-8 copy and stays in `ConfigSettings.tsx` beside the
 * constant it quotes.
 */
export const configCopy = {
  section: 'Configuration',
  row: 'Configuration',
  help: 'Edit the settings Biorouter keeps in its configuration file.',
  /** The row button's visible word; its name is the longer one (label in name). */
  edit: 'Edit…',
  editName: 'Edit configuration',
  /** The editor's title. No icon, and no lowercase brand. */
  dialogTitle: 'Configuration',
  empty: 'No configuration settings found.',
  resetChanges: 'Reset changes',
  done: 'Done',
  saveLabel: (name: string) => `Save ${name}`,
  savedTitle: 'Configuration updated',
  saved: (name: string) => `Saved "${name}"`,
  saveFailedTitle: 'Save failed',
  notSaved: (name: string, reason: string) => `"${name}" was not saved. ${reason}`,
  saveFailed: (name: string) => `Failed to save "${name}"`,
  resetTitle: 'Configuration reset',
  reset: 'All changes have been reverted',
} as const;
