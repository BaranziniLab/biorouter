/**
 * Every visible string the Skills view and its two dialogs render, in one place.
 *
 * Tests import these rather than restating them (Crew's `copy.ts` pattern;
 * implementation spec §1, copy rules): sentence case, "Biorouter", "chat", the
 * typographic ellipsis, no dashes as punctuation.
 */
export const SKILLS_COPY = {
  title: 'Skills',
  info: 'Reusable instructions Biorouter follows when a task calls for them.',

  add: 'Add',
  browse: 'Browse marketplace',
  fromSource: 'From a repository or .zip…',
  write: 'Write a skill…',

  /** The provenance groups. The caps style uppercases them; the count follows. */
  groups: {
    biorouter: 'Biorouter',
    extension: (extension: string) => `From ${extension}`,
    other: 'From other agents',
    project: 'From this project',
    matches: 'Matches',
  },

  openFolder: 'Open folder',
  copySkillMd: 'Copy SKILL.md',
  delete: 'Delete…',

  expand: (name: string) => `Expand ${name}`,
  collapse: (name: string) => `Collapse ${name}`,
  bundleSummary: (count: number, version?: string | null) =>
    [`${count} skill${count === 1 ? '' : 's'}`, version].filter(Boolean).join(' · '),
  entryPoint: (name: string) => `Entry point: ${name}`,

  emptyTitle: 'No skills yet',
  emptyDescription: 'Add one from the marketplace, a repository or a .zip file.',
  noMatchTitle: 'No matching skills',
  noMatchDescription: 'Try a different name, description or package.',

  notSaved: (error: string) => `The change was not saved: ${error}`,
  packageRemoved: 'Package removed',
  skillDeleted: 'Skill deleted',
  deleteFailed: 'Delete failed',
  deleteFailedFallback: 'Could not remove it',
  copied: 'SKILL.md copied',
  copyFailed: 'Copy failed',
  copyFailedMessage: 'Could not copy to the clipboard',

  confirmDeletePackageTitle: (name: string) => `Delete package "${name}"?`,
  confirmDeleteSkillTitle: (name: string) => `Delete "${name}"?`,
  confirmDeletePackageMessage: (count: number) =>
    `This removes all ${count} skills in this package from disk. It cannot be undone.`,
  confirmDeleteSkillMessage: 'This removes the skill folder from disk. It cannot be undone.',
  confirmDeletePackage: 'Delete package',
  confirmDelete: 'Delete',
  cancel: 'Cancel',
} as const;

export const ADD_SKILL_COPY = {
  title: 'Add skill',
  repositoryLabel: 'From a repository',
  repositoryHelp:
    'A repository holding several skills stays one package, with its own name and entry point.',
  repositoryPlaceholder: 'https://github.com/owner/repo',
  lookUp: 'Look up',
  dropZone: 'Drop a .zip here',
  /** The drop zone's name: it also opens a file chooser, which the visible line does not say. */
  dropZoneName: 'Drop a .zip here, or choose a file',
  cancel: 'Cancel',
  install: 'Install',
  installing: 'Installing…',
  installSkill: 'Install skill',
  installSkills: (n: number) => `Install ${n} skills`,
  installSeparately: 'Install separately',
  installBundle: 'Install as one bundle',
  installFailed: 'Install failed',
  installed: (n: number) => `Installed ${n} skill${n === 1 ? '' : 's'}`,
  fileCount: (n: number) => `${n} file${n === 1 ? '' : 's'}`,
  entryPoint: (name: string) => `Entry point: ${name}`,
  remoteDaemon:
    'Biorouter is running on another machine, so it cannot read a file you drop here. Copy the ' +
    'skill onto that machine and add it with `biorouter skill install <path>`, or paste a ' +
    'repository URL above.',
  requestFailed: 'the request failed.',
} as const;

export const CUSTOM_SKILL_COPY = {
  title: 'Write a skill',
  subtitle: 'Name and description are required.',
  help: 'Saved as a folder with SKILL.md in Biorouter Skills.',
  editorLabel: 'SKILL.md',
  invalid: 'Add YAML frontmatter with a name and a description.',
  cancel: 'Cancel',
  save: 'Save skill',
  saving: 'Saving…',
  saved: 'Skill saved',
  saveFailed: 'Save failed',
  couldNotWrite: (path: string) => `Could not write to ${path}`,
} as const;
