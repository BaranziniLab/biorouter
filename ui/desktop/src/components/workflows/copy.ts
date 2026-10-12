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
  // A menu item that opens a confirmation takes the ellipsis, as Skills' Delete… does (V3-21).
  delete: 'Delete…',
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
  couldNotWriteFile: 'The file couldn’t be written to that location.',
  couldNotDelete: (title: string) => `Couldn’t delete ${title}`,
  exportDialogTitle: 'Export workflow',
  yamlFiles: 'YAML files',
  allFiles: 'All files',

  slash: {
    title: 'Slash command',
    inputLabel: 'Command',
    placeholder: 'command-name',
    // The one live helper line under the field, always present: it says what to do before
    // anything is typed and then follows the name; an error replaces it.
    helperEmpty: 'Type your command in any chat to start this workflow',
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
    // One line, like Knowledge's and Add skill's drop zones; the zone is also a button.
    drop: 'Drop a .yaml or .json file here',
    importing: 'Importing…',
    wrongType: 'Choose a YAML or JSON workflow file.',
    imported: (title: string) => `Imported ${title}`,
    cancel: 'Cancel',
  },
} as const;

/**
 * The workflow editors: `CreateEditWorkflowModal`, `CreateWorkflowFromSessionModal`, the form
 * they share (`shared/WorkflowFormFields.tsx`) and its pickers and sub-editors.
 */
export const workflowFormCopy = {
  newTitle: 'New workflow',
  editTitle: 'Edit workflow',
  fromChatTitle: 'New workflow from this chat',
  cancel: 'Cancel',
  save: 'Save',
  saveAndRun: 'Save and run',
  create: 'Create',
  createAndRun: 'Create and run',
  saving: 'Saving…',
  creating: 'Creating…',
  saved: (title: string) => `Saved ${title}`,
  created: (title: string) => `Created ${title}`,
  couldNotSave: 'Couldn’t save the workflow',
  couldNotCreate: 'Couldn’t create the workflow',
  invalid: 'Fill in the title, the description, and instructions or a prompt.',
  invalidSchema: 'The response schema is not valid JSON.',
  readingChat: 'Reading this chat…',
  refusedTitle: 'A workflow can’t be made from this chat',

  title: 'Title',
  titlePlaceholder: 'Workflow title',
  description: 'Description',
  descriptionPlaceholder: 'What this workflow does',
  instructions: 'Instructions',
  instructionsInfo:
    'Hidden from the user. Use {{parameter_name}} to add a parameter that is filled in at run time.',
  instructionsPlaceholder: 'Detailed instructions for the model',
  prompt: 'Initial prompt',
  optional: '(optional)',
  promptInfo:
    'Sent as the first message when the workflow starts. Needed when there are no instructions.',
  promptPlaceholder: 'The first message',
  openEditor: 'Open editor',

  advanced: 'Advanced',
  /** The closed Advanced section states its defaults (Crew's disclosure rule). */
  advancedSummary: (model: string | null, parameters: number, extensions: number) =>
    [
      model ?? 'Global model',
      parameters === 0
        ? 'no parameters'
        : parameters === 1
          ? '1 parameter'
          : `${parameters} parameters`,
      extensions === 0
        ? 'global extensions'
        : extensions === 1
          ? '1 extension'
          : `${extensions} extensions`,
    ].join(' · '),

  parameters: 'Parameters',
  parametersInfo:
    'Found from {{parameter_name}} in the instructions, the prompt and the starter buttons. You can also add one by name.',
  parameterPlaceholder: 'Parameter name',
  addParameter: 'Add parameter',

  responseSchema: 'Response schema',
  responseSchemaInfo: 'A JSON Schema the model’s answer must follow.',

  model: 'Model',
  modelInfo: 'Leave these blank to use your default model. Temperature runs from 0 to 2.',
  provider: 'Provider',
  modelName: 'Model name',
  temperature: 'Temperature',
  defaultPlaceholder: 'Default',
  temperaturePlaceholder: '0 to 2',

  knowledgeBases: 'Knowledge bases',
  knowledgeBasesInfo:
    'The knowledge bases this workflow can search, and the one it uses by default.',
  noDefaultKnowledgeBase:
    'Chats it starts search every base above, and a write that names no base asks which one to use.',
  noKnowledgeBases: 'No knowledge bases found',
  searchKnowledgeBases: 'Search knowledge bases…',
  skills: 'Skills',
  skillsInfo: 'Skills this workflow prefers when the skills extension is on.',
  noSkills: 'No skills found',
  searchSkills: 'Search skills…',
  extensions: 'Extensions',
  extensionsInfo: 'Extensions on for this workflow. Leave empty to use your global default.',
  noExtensions: 'No extensions configured',
  searchExtensions: 'Search extensions…',

  picker: {
    count: (count: number, noun: string) =>
      count === 0
        ? `No ${noun}s selected`
        : count === 1
          ? `1 ${noun} selected`
          : `${count} ${noun}s selected`,
    default: 'Default',
    defaultNamed: (noun: string, label: string) => `Default ${noun}: ${label}`,
    defaultIs: (label: string) => `Default: ${label}`,
    noDefault: 'No default',
  },

  activities: {
    title: 'Activities',
    info: 'What the chat shows when the workflow starts: an intro message and starter buttons.',
    message: 'Intro message',
    messageInfo: 'Shown at the top of the chat. Markdown works.',
    messagePlaceholder: 'Welcome message',
    buttons: 'Starter buttons',
    buttonsInfo: 'Buttons under the intro message. Each one sends its label as a message.',
    buttonPlaceholder: 'Button label',
    add: 'Add',
    remove: (activity: string) => `Remove ${activity}`,
  },

  instructionsEditor: {
    title: 'Instructions',
    insertExample: 'Insert example',
    save: 'Save',
    cancel: 'Cancel',
  },

  schemaEditor: {
    title: 'Response schema',
    insertExample: 'Insert example',
    invalidJson: 'This is not valid JSON.',
    save: 'Save',
    cancel: 'Cancel',
  },
} as const;
