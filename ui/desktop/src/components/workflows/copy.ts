/**
 * Every string Workflows shows, in one place (Crew's `copy.ts` pattern). Tests import these
 * rather than restating them, so a copy edit is one change.
 *
 * Copy rules (spec section 1): sentence case, "chat" not "conversation", "Biorouter", the
 * typographic ellipsis, no dashes as punctuation. Explanations live in InfoTips (at most two
 * sentences); errors, refusals and the consequence of a destructive action stay visible.
 */

export const workflowsCopy = {
  title: 'Workflows',
  info: 'Reusable chat setups. Start one from here or with its slash command.',
  filterLabel: 'Filter workflows',
  importWorkflow: 'Import workflow',
  newWorkflow: 'New workflow',

  emptyTitle: 'No workflows yet',
  emptyDescription: 'Save a chat as a workflow, or start a new one.',
  noMatchTitle: 'No matching workflows',
  noMatchDescription: 'Try another search.',
  loadFailedTitle: 'Couldn’t load workflows',
  tryAgain: 'Try again',

  // Row. The action names carry the workflow, so ten rows' buttons can be told apart.
  run: 'Run',
  runNamed: (title: string) => `Run ${title}`,
  moreActionsNamed: (title: string) => `More actions for ${title}`,
  runInNewWindow: 'Run in new window',
  edit: 'Edit',
  slashCommandItem: 'Slash command…',
  scheduleItem: 'Schedule…',
  copyLink: 'Copy link',
  copyYaml: 'Copy YAML',
  exportToFile: 'Export to file…',
  delete: 'Delete',
  scheduleBadgeLabel: 'Schedule',
  modified: (when: string) => `Modified ${when}`,

  deleteTitle: (title: string) => `Delete ${title}?`,
  // A destructive consequence: stays visible at the moment of decision (principle 2).
  deleteMessage: 'This permanently removes the workflow file. This action cannot be undone.',
  deleteConfirm: 'Delete',
  cancel: 'Cancel',

  // Toasts: one line. Errors keep their reason in the message.
  linkCopied: 'Link copied',
  yamlCopied: 'YAML copied',
  savedTo: (file: string) => `Saved to ${file}`,
  deleted: (title: string) => `Deleted ${title}`,
  couldNotCopyLink: 'Couldn’t copy the link',
  couldNotCopyYaml: 'Couldn’t copy the YAML',
  couldNotExport: 'Couldn’t export the workflow',
  couldNotDelete: (title: string) => `Couldn’t delete ${title}`,
  exportDialogTitle: 'Export workflow',
  yamlFiles: 'YAML files',
  allFiles: 'All files',

  slash: {
    title: 'Slash command',
    inputLabel: 'Command',
    placeholder: 'command-name',
    // The one live helper line under the field; an error replaces it.
    helper: (cmd: string) => `Type /${cmd} in any chat`,
    remove: 'Remove',
    save: 'Save',
    saving: 'Saving…',
    working: 'Working…',
    saved: (cmd: string) => `/${cmd} saved`,
    removed: 'Slash command removed',
    couldNotSave: 'Couldn’t save the slash command',
    couldNotRemove: 'Couldn’t remove the slash command',
  },

  schedule: {
    title: 'Schedule',
    remove: 'Remove schedule',
    save: 'Save',
    saving: 'Saving…',
    working: 'Working…',
    // Named: the confirmation is the last chance to notice the wrong row's button was pressed.
    saved: (title: string) => `Schedule saved for ${title}`,
    removed: (title: string) => `Schedule removed from ${title}`,
    notSaved: 'Schedule not saved',
    notRemoved: 'Schedule not removed',
  },

  importDialog: {
    title: 'Import workflow',
    drop: 'Drop a YAML or JSON file here',
    browse: 'or click to browse',
    importing: 'Importing…',
    wrongType: 'Choose a YAML or JSON workflow file.',
    imported: (title: string) => `Imported ${title}`,
    cancel: 'Cancel',
  },
} as const;
