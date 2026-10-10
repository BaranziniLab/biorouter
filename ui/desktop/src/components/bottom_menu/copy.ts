/**
 * The composer pickers' strings: the Tools chip and its three lists, the model
 * and effort chip, and the footer line (folder, context, cost).
 *
 * One place, so the tests import what the interface says instead of restating
 * it (Crew's `copy.ts` pattern). Sentence case, the noun is "chat", the
 * ellipsis is "…", and nothing here is punctuated with a dash. Help text is at
 * most two sentences, because it is read in an InfoTip.
 *
 * Not here: the privacy sentences the daemon serves, the locked privacy copy in
 * `components/privacy/**`, and the effort labels and descriptions, which live
 * beside the store that owns the values (`store/reasoningEffort.ts`).
 */

function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

export const TOOLS_COPY = {
  chip: 'Tools',
  /** The chip's accessible name: the three numbers its one figure adds up. */
  chipLabel: (extensions: number, skills: number, knowledge: number) =>
    `Tools: ${count(extensions, 'extension', 'extensions')}, ${count(skills, 'skill', 'skills')}, ${count(knowledge, 'knowledge base', 'knowledge bases')}`,
  chipTooltip: (extensions: number, skills: number, knowledge: number) =>
    `${count(extensions, 'extension', 'extensions')} · ${count(skills, 'skill', 'skills')} · ${count(knowledge, 'knowledge base', 'knowledge bases')}`,
  segmentsLabel: 'Show',
  segments: {
    extensions: 'Extensions',
    skills: 'Skills',
    knowledge: 'Knowledge',
  },
  /** The scope line above each list, and the InfoTip beside it. */
  scope: {
    thisChat: 'This chat',
    nextChat: 'Next chat',
    newChats: 'New chats',
    defaultForChats: 'Default',
    about: 'scope',
  },
} as const;

export const EXTENSIONS_COPY = {
  listLabel: 'Extensions',
  search: 'Search extensions…',
  helpChat:
    'Changes apply to this chat only. Settings → Extensions sets the default for new chats.',
  helpNextChat:
    'Changes apply to the next chat you start. Settings → Extensions sets the default for all new chats.',
  enableAll: (n: number) => `Enable all (${n})`,
  disableAll: (n: number) => `Disable all (${n})`,
  empty: 'No extensions available',
  noMatch: 'No extensions found',
  /** §14.5, visible on the row: a refusal is never tooltip-only. */
  pairingRefused: 'Unavailable in this chat (public model)',
  pairingRefusedReason: 'Unavailable in this chat: a private extension needs a private model',
  toggleErrorTitle: 'Extension toggle error',
  startChatFirst: 'Start a chat first.',
  toggleFailed: (name: string, enabling: boolean) =>
    `${name} could not be ${enabling ? 'enabled' : 'disabled'}.`,
  bulkFailed: 'The extension selection could not be updated.',
  refreshErrorTitle: 'Extension refresh error',
  refreshFailed: 'The latest extension state could not be refreshed.',
} as const;

export const SKILLS_COPY = {
  listLabel: 'Skills',
  search: 'Search skills…',
  helpChat: 'Changes apply to this chat only.',
  helpNewChats: 'Changes apply to new chats. A chat you already started keeps its own list.',
  enableAll: (n: number) => `Enable all (${n})`,
  disableAll: (n: number) => `Disable all (${n})`,
  loading: 'Loading skills…',
  empty: 'No skills available',
  noMatch: 'No skills found',
  bundleCount: (n: number) => count(n, 'skill', 'skills'),
  bundleMembers: (names: string[]) => `Includes ${names.join(', ')}`,
  updateFailedTitle: 'Skill update failed',
  updateFailed: (reason: string) => `The change was not saved: ${reason}`,
} as const;

export const KNOWLEDGE_COPY = {
  listLabel: 'Knowledge bases',
  search: 'Search knowledge bases…',
  helpChat: 'Hidden bases stay out of this chat. The agent cannot search or read them here.',
  helpDefault: 'Chats that have not changed their own list use this one.',
  showAll: (n: number) => `Show all (${n})`,
  hideAll: (n: number) => `Hide all (${n})`,
  empty: 'No knowledge bases available',
  noMatch: 'No knowledge bases found',
  /** One verb pair per popup (#82): this list is about VISIBILITY. */
  rowLabel: (name: string, hidden: boolean) =>
    `${name}, ${hidden ? 'hidden from' : 'visible to'} this chat`,
} as const;

export const MODEL_COPY = {
  /** The chip's tooltip and the menu header use the friendly name; this is the full id. */
  fullIdTooltip: (id: string) => `Model: ${id}`,
  currentModel: 'Current model',
  changeModel: 'Change model…',
  leadWorker: 'Lead and worker…',
  effortGroup: 'Effort',
  aboutModel: 'this model',
  noModelTooltip: 'Opens the provider catalog',
} as const;

export const FOOTER_COPY = {
  /** Shown only below half: a full window says nothing worth reading. */
  contextLeft: (pct: number) => `${pct}% left`,
  contextTooltip: (pct: number, used: string, total: string) =>
    `${pct}% context left · ${used} of ${total}`,
  contextLabel: 'Context',
  contextUsed: 'Context window used',
  compact: 'Compact chat',
  nothingToCompact: 'Nothing to compact yet',
  thresholdLabel: 'Auto-compact threshold',
  thresholdTooltip: (pct: number) => `Auto-compact at ${pct}%`,
  costUnavailable: 'Chat cost unavailable',
  cost: (label: string, estimated: boolean) =>
    `${estimated ? 'Estimated chat total' : 'Chat cost'} ${label}`,
} as const;
