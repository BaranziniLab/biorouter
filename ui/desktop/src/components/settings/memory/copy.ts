/**
 * The words of Settings > Chat > Memory. Tests import these instead of retyping them.
 *
 * Help is InfoTip text, two sentences at most. The audience sentence ("who can read this store")
 * is the one the section exists to make, so it is also said in full in the delete confirmation,
 * where it decides something.
 */
export const memoryCopy = {
  section: 'Memory',
  help: 'What Biorouter has been asked to remember, and everything it can disclose when a chat asks to read it.',
  refresh: 'Refresh memory',
  global: {
    heading: 'Global',
    help: 'Shared by every chat on this computer.',
  },
  local: {
    heading: 'This project',
    help: 'Only chats opened in this project can read it.',
  },
  /** The local store when the window has no working folder. */
  noProject: 'No project open',
  noProjectHelp:
    'Local memories live in the .biorouter/memory folder of a project and are managed from a window opened there.',
  /** An empty store, in one muted row. */
  empty: 'Nothing has been remembered yet.',
  emptyHelp: 'Biorouter adds a memory only when it asks you and you agree.',
} as const;
