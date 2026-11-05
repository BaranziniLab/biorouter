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

  /**
   * The header button's name gains this while subagents run and the rail is
   * hidden: "Chat summary, 3 subagents running".
   */
  buttonWithSubagents: (base: string, running: number) =>
    `${base}, ${running} ${running === 1 ? 'subagent' : 'subagents'} running`,

  /** The subagents and phases sections (`summary/SummarySubagents.tsx`). */
  agents: {
    phases: 'Phases',
    /** Read after the phase count by assistive technology only: "1 of 4 phases complete". */
    phasesCountSuffix: ' phases complete',
    phaseListLabel: 'Phases of the plan',
    phaseProgressLabel: (title: string) => `${title} progress`,
    /** Visible per phase: "2 of 5". Read: "2 of 5 subagents done". */
    phaseAgentsSuffix: ' subagents done',
    subagents: 'Subagents',
    otherSubagents: 'Other subagents',
    subagentListLabel: 'Subagents of this chat',
    /** Read after a list's count: "4 of 10 done". */
    doneSuffix: ' done',
    running: (count: number) => `${count} running`,
    quiet: (count: number) => `${count} quiet`,
    failed: (count: number) => `${count} failed`,
    blocked: (count: number) => `${count} blocked`,
    groups: { running: 'Running', attention: 'Needs attention', done: 'Done' },
    showAll: (count: number) => `Show all ${count}`,
    showFewer: 'Show fewer',
    refreshFailed: 'Subagents could not refresh.',
    noneInPhase: 'No subagents yet',
    status: {
      queued: 'Queued',
      running: 'Running',
      cancelling: 'Stopping',
      completed: 'Completed',
      blocked: 'Blocked',
      incomplete: 'Incomplete',
      failed: 'Failed',
      interrupted: 'Interrupted',
      unknown: 'Status unknown',
    },
    quietFor: (elapsed: string) => `quiet for ${elapsed}`,
    /**
     * A comfortable row's setup counts, drawn as a glyph and a number and read
     * as these words: "2 extensions", "1 skill", "1 knowledge base". Never
     * abbreviated: "1 KB" read as one kilobyte.
     */
    setupCount: {
      extension: (count: number) => `${count} ${count === 1 ? 'extension' : 'extensions'}`,
      skill: (count: number) => `${count} ${count === 1 ? 'skill' : 'skills'}`,
      knowledge: (count: number) =>
        `${count} ${count === 1 ? 'knowledge base' : 'knowledge bases'}`,
    },
    phaseStatus: {
      pending: 'Pending',
      in_progress: 'In progress',
      completed: 'Complete',
      blocked: 'Blocked',
      skipped: 'Skipped',
    },
    phaseKind: {
      plan: 'Plan',
      research: 'Research',
      execution: 'Execution',
      implementation: 'Implementation',
      verification: 'Verification',
      testing: 'Testing',
      auditing: 'Auditing',
      review: 'Review',
      meeting: 'Meeting',
      synthesis: 'Synthesis',
      other: 'Other',
    },
    /** The hover card's labels. */
    card: {
      role: 'Role',
      phase: 'Phase',
      model: 'Model',
      extensions: 'Extensions',
      skills: 'Skills',
      knowledgeBases: 'Knowledge bases',
      tokens: 'Tokens',
      toolCalls: 'Tool calls',
      messages: 'Messages',
      runTime: 'Run time',
      none: 'None',
      private: 'Private',
      openHint: 'Click to open in a tab',
    },
  },

  /**
   * The Decisions section (`summary/SummaryDecisions.tsx`): one row per source,
   * never one per call.
   */
  decisions: {
    title: 'Decisions',
    countSuffix: ' decision calls',
    listLabel: 'Decision sources',
    calls: (n: number) => `${n.toLocaleString()} ${n === 1 ? 'call' : 'calls'}`,
    errors: (n: number) => `${n.toLocaleString()} ${n === 1 ? 'error' : 'errors'}`,
    notChecked: 'Not checked yet',
    last: (answer: string) => `last ${answer}`,
    gateAttempt: (attempt: number, max: number) => `attempt ${attempt} of ${max}`,
    gateBlocked: (max: number) => `blocked after ${max} attempts`,
    monitors: 'Monitors',
    monitorsOn: (n: number) => (n === 0 ? 'none on' : `${n} on`),
    monitorsAdd: 'Add',
    refreshFailed: 'Decisions did not refresh',
    openHint: 'Opens the decision history',
    state: {
      pending: 'Not checked yet',
      armed: 'Watching',
      fired: 'Fired',
      off: 'Off',
      error: 'Error',
      done: 'Decided',
      passed: 'Passed',
      failed: 'Failed',
      blocked: 'Blocked',
    },
    gate: {
      passed: (p: string, t: string) => `Gate ${p} ≥ ${t} passed`,
      failed: (p: string, t: string, attempt: number, max: number) =>
        `Gate ${p} < ${t}, attempt ${attempt} of ${max}`,
      blocked: 'Gate blocked',
      pending: (t: string) => `Gate at ${t}`,
      label: 'Phase gate',
    },
  },

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
    retry: 'Retry',
    action: 'Go home',
  },
} as const;
