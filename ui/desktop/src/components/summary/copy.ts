/**
 * Every string the Chat summary (the docked rail, its popover fallback and the
 * header button) shows. Tests import these, so a copy change is one edit.
 *
 * Sentence case, "chat" not "conversation", no dashes as punctuation. The minus
 * in the code delta is U+2212, a sign rather than punctuation.
 */
export const summaryCopy = {
  /** The rail's and the popover's accessible name, and the button's base name. */
  title: 'Chat summary',
  showSummary: 'Show summary',
  hideSummary: 'Hide summary',

  todo: 'To do',
  /** The visible count: "2 of 5". */
  todoCount: (done: number, total: number) => `${done} of ${total}`,
  /** Read after the count by assistive technology only: "2 of 5 complete". */
  todoCountSuffix: ' complete',
  todoListLabel: 'To do tasks',
  todoProgressLabel: 'Completed tasks',
  /** The header button's name while the rail is hidden and a step is running. */
  buttonWithProgress: (done: number, total: number) =>
    `Chat summary, ${done} of ${total} to do items complete`,
  todoStatus: {
    pending: 'Pending',
    in_progress: 'In progress',
    blocked: 'Blocked',
    completed: 'Complete',
  },
  todoRefreshFailed: 'To do could not refresh.',
  retry: 'Retry',

  statsLabel: 'Chat statistics',
  toolCalls: 'Tool calls',
  tokens: 'Tokens',
  tokensHelp:
    'Billed tokens across every model in this chat. N/A when a model has no certified total.',
  artifacts: 'Artifacts',
  code: 'Code',
  /** The code delta's accessible reading, since "+12 −3" is not words. */
  codeDelta: (added: number, removed: number) => `${added} lines added, ${removed} removed`,

  makeWorkflow: 'Make workflow',
  workflow: 'Workflow',
  diagnostics: 'Diagnostics',

  /** The Stop & send banner above the composer, one line per state. */
  stopAndSend: {
    owned: 'Stop & send is waiting',
    settling: 'Stop & send is settling',
    foreign: 'Stop & send is open in another window',
    takeOver: 'Take over',
    abandon: 'Abandon',
    helpLabel: 'Stop & send',
    help: {
      owned:
        'A previous Stop & send is ready. Re-enter the message you want to send; Biorouter will not guess or resend lost composer text.',
      settling:
        'A previous Stop & send is still settling. Recover it explicitly or abandon the stopped-turn continuation.',
      foreign:
        'Another window owns a pending Stop & send. Take it over here or abandon the stopped-turn continuation before sending.',
    },
  },

  /** The chat load error. */
  loadError: {
    title: 'Could not load this chat',
    action: 'Go home',
  },
} as const;
