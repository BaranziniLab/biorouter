/**
 * Every string the Scheduler shows, in one place (Crew's `copy.ts` pattern). Tests import these
 * rather than restating them, so a copy edit is one change.
 *
 * Copy rules (spec section 1): sentence case, "chat" not "conversation", "Biorouter", the
 * typographic ellipsis, no dashes as punctuation, names rather than ids.
 */

/**
 * What deleting a schedule actually does, said in the confirmation.
 *
 * ⚠ It no longer removes the workflow, so it must no longer imply that it might. The previous
 * sentence ("This permanently removes the schedule and its run configuration") was written when
 * a delete unlinked `job.source` unconditionally, and for a schedule added from a workflow row
 * that source was the user's own workflow file. `scheduler::scheduler_owns_source` now confines
 * the unlink to the private copy the scheduler made for itself, so the confirmation names the
 * file that survives. It is a destructive consequence, so it stays visible (principle 2).
 */
export const DELETE_SCHEDULE_MESSAGE =
  'This removes the schedule and stops its future runs. The workflow it runs is left in place. This action cannot be undone.';

export const scheduleCopy = {
  title: 'Scheduler',
  info: 'Runs a saved workflow on a timer. Each run opens a new chat.',
  newSchedule: 'New schedule',
  refresh: 'Refresh schedules',
  refreshTooltip: 'Refresh',

  emptyTitle: 'No schedules yet',
  emptyDescription: 'Run a workflow on a timer.',
  loadFailed: 'Couldn’t load schedules.',

  status: {
    running: 'Running',
    paused: 'Paused',
    failed: 'Failed',
    scheduled: 'Scheduled',
  },
  lastRun: (when: string) => `Last run ${when}`,
  notRunYet: 'Not run yet',

  // Row actions. The names carry the schedule so ten rows' buttons can be told apart.
  view: (name: string) => `View schedule ${name}`,
  pause: 'Pause',
  resume: 'Resume',
  stop: 'Stop',
  pauseNamed: (name: string) => `Pause ${name}`,
  resumeNamed: (name: string) => `Resume ${name}`,
  stopNamed: (name: string) => `Stop ${name}`,
  edit: 'Edit',
  runNow: 'Run now',
  inspectRun: 'Inspect run',
  stopRun: 'Stop run',
  delete: 'Delete',
  moreActionsNamed: (name: string) => `More actions for ${name}`,

  deleteTitle: (name: string) => `Delete ${name}?`,
  deleteConfirm: 'Delete',
  cancel: 'Cancel',

  // Toasts: one line, sentence case, names not ids. Errors keep their reason as the message.
  paused: (name: string) => `Paused ${name}`,
  resumed: (name: string) => `Resumed ${name}`,
  couldNotPause: (name: string) => `Couldn’t pause ${name}`,
  couldNotResume: (name: string) => `Couldn’t resume ${name}`,
  runStarted: 'Run started',
  runStoppedWhileStarting: 'Run stopped while starting',
  couldNotRun: (name: string) => `Couldn’t run ${name}`,
  runStopped: 'Run stopped',
  couldNotStop: 'Couldn’t stop the run',
  couldNotInspect: 'Couldn’t open the run',
  saved: (name: string) => `Saved ${name}`,
  couldNotSave: (name: string) => `Couldn’t save ${name}`,
  couldNotDelete: (name: string) => `Couldn’t delete ${name}`,

  detail: {
    back: 'Back to Scheduler',
    runs: 'Runs',
    workflow: 'Workflow',
    lastRun: 'Last run',
    lastError: 'Last error',
    started: 'Started',
    openChat: 'Open chat',
    recentRuns: 'Recent runs',
    running: 'Running…',
    availableAfterRun: 'Available when this run finishes',
    resumeTooltip: 'Resume automatic runs',
    pauseTooltip: 'Pause automatic runs',
    noRunsTitle: 'No runs yet',
    noRunsDescription: 'Each run opens a new chat.',
    notFoundTitle: 'Schedule not found',
    notFoundDescription: 'It may have been deleted.',
    loadFailed: 'Couldn’t load this schedule.',
    runsLoadFailed: 'Couldn’t load the runs for this schedule.',
    chatLoadFailed: 'Couldn’t load this chat',
    noRunningChat: 'The run has no chat yet.',
    openRun: (name: string) => `Open run ${name}`,
    messages: (count: number) => `${count} ${count === 1 ? 'message' : 'messages'}`,
    tokens: (formatted: string) => `${formatted} tokens`,
    tokensLowerBound:
      'At least this many tokens. Only last-turn usage is recorded for this older chat.',
    tokensBilled: 'Billed tokens across every turn, including recorded cache usage.',
  },

  modal: {
    newTitle: 'New schedule',
    editTitle: 'Edit schedule',
    name: 'Name',
    namePlaceholder: 'daily-summary',
    nameHelper: 'Letters, digits, hyphens and underscores only',
    workflowFile: 'Workflow file',
    workflowPlaceholder: '/path/to/workflow.yaml',
    browse: 'Browse for a workflow file',
    when: 'Schedule',
    create: 'Create schedule',
    creating: 'Creating…',
    save: 'Save changes',
    saving: 'Saving…',
    cancel: 'Cancel',
    workflowRequired: 'Choose a workflow file.',
    wrongFileType: 'Choose a YAML file (.yaml or .yml).',
  },
} as const;
