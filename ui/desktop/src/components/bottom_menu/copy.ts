/**
 * The composer pickers' strings: the Tools chip and its three lists, the model
 * and effort chip, the footer line (folder, context, cost) and the three cards
 * its items open (`FooterCard`: the working folder, the context window, the
 * cost).
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
  /** Counts stay available to assistive technology without opening the popover. */
  chipLabel: (extensions: number, skills: number, knowledge: number) =>
    `Tools: ${count(extensions, 'extension', 'extensions')}, ${count(skills, 'skill', 'skills')}, ${count(knowledge, 'knowledge base', 'knowledge bases')}`,
  chipTooltip: (extensions: number, skills: number, knowledge: number) =>
    `${count(extensions + skills + knowledge, 'tool', 'tools')} enabled · ${count(extensions, 'extension', 'extensions')} · ${count(skills, 'skill', 'skills')} · ${count(knowledge, 'knowledge base', 'knowledge bases')}`,
  segmentsLabel: 'Tool lists',
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
    defaultForChats: 'Default for chats',
    /** The InfoTip's name reads "About where changes apply". */
    about: 'where changes apply',
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
  /** The Effort InfoTip's name reads "About effort". */
  aboutEffort: 'effort',
  aboutModel: 'this model',
  noModelTooltip: 'Opens the provider catalog',
} as const;

/** What the backend calls a grant's access (`access_name` in `folder_access.rs`), in words. */
function grantAccess(access: string): string {
  if (access === 'read-only') return 'Read only';
  if (access === 'read/write') return 'Read and write';
  // A value this build does not know is shown as the backend wrote it, never guessed at.
  return access;
}

/** Whether a grant reaches the folder's subfolders, or only the files it was given for. */
function grantScope(descendants: boolean): string {
  return descendants ? 'includes subfolders' : 'listed files only';
}

/**
 * The working folder's card (`FolderAccessCard`, a `FooterCard`): it opens from the footer's folder
 * chip and names the folder the chat works in, lets an empty chat choose another, and lists every
 * other folder the chat may use, each with Revoke. It replaced the footer's "Folder access" button
 * and its dialog.
 */
export const FOLDER_ACCESS_COPY = {
  /** The card's accessible name (it is a `dialog`). */
  cardLabel: 'Working folder',
  /** The first section's label. */
  workingFolder: 'Working folder',
  /** An existing chat whose folder has not been read yet, or whose row names none. */
  workingFolderUnknown: 'Not known yet.',
  /** #44: once the chat has messages the folder cannot change, and the card says so. */
  lockedNote: 'Fixed once the chat has messages.',
  /** The empty chat's way to another folder (the native chooser). */
  chooseFolder: 'Choose folder…',
  otherFolders: 'Other approved folders',
  loading: 'Checking folder access…',
  loadFailed: 'Folder access could not be loaded.',
  none: 'None.',
  grantAccess,
  grantScope,
  /** The muted line under a granted folder's path. */
  grantDetail: (access: string, descendants: boolean) =>
    `${grantAccess(access)} · ${grantScope(descendants)}`,
  revoke: 'Revoke',
  /** The button's name says which folder, so a list of them is not a list of "Revoke". */
  revokeLabel: (folder: string) => `Revoke access to ${folder}`,
  revokeFailed: 'Access could not be revoked. The grant is still listed; try again.',
  /** Follows `folderAccessLifetime()`, the sentence about when a session grant ends. */
  onceNotListed: 'Folders allowed once are not listed.',
  approvalKey: 'Folder approval key',
  approvalKeyHelp:
    'Use the separate key configured with biorouter serve --folder-access-approval. It is cleared after each revocation.',
  /**
   * What folder approval covers, said exactly, on Home and in a chat. Only the Developer and Files
   * extensions' editor, file analysis and image tools ask (`admit_file_access` in
   * `agents/extension_manager.rs`, `file_access_plan` in `biorouter-mcp/src/file_access.rs`); the
   * shell and the PDF, Word and Excel tools never do. A tool can still ask first as the person's
   * permission mode says, so nothing here claims that nothing else asks.
   */
  whoAsks:
    'The file editor, file analysis and image tools ask before they use another folder. Shell commands and document tools do not ask.',
} as const;

export const FOOTER_COPY = {
  /** The folder item's name: the full path, which the line itself does not show. */
  folderLabel: (dir: string) => `Working folder: ${dir}`,
  /** The folder item's name while an existing chat's folder is not known yet. */
  folderUnknownLabel: 'Working folder: not known yet',
  folderUpdateFailedTitle: 'Working folder update failed',
  folderUpdateFailed: 'The working folder could not be changed.',
  /**
   * What is left of the window: beside the ring below half (a full window says nothing worth
   * reading), and the context card's reading.
   */
  contextLeft: (pct: number) => `${pct}% left`,
  /** The ring's accessible name, which says the whole reading without opening the card. */
  contextTooltip: (pct: number, used: string, total: string) =>
    `${pct}% context left · ${used} of ${total}`,
  costUnavailable: 'Chat cost unavailable',
  cost: (label: string, estimated: boolean) =>
    `${estimated ? 'Estimated chat total' : 'Chat cost'} ${label}`,
} as const;

/**
 * The context ring's card (`ContextWindowIndicator`, a `FooterCard`): how much of the window is
 * left, the bar with the auto-compact marker, and Compact. Figures come in already formatted
 * (`fmt` in ContextWindowIndicator.tsx: "92k", "1.1M").
 */
export const CONTEXT_CARD_COPY = {
  /** The card's accessible name (it is a `dialog`). */
  cardLabel: 'Context',
  section: 'Context window',
  /** The section's reading: what is LEFT, as the ring draws it. */
  left: FOOTER_COPY.contextLeft,
  /** The muted line under it: what is USED, as the bar fills. */
  used: (used: string, total: string) => `${used} of ${total} tokens used`,
  /** The bar's accessible name. */
  barLabel: 'Context window used',
  /** The marker's accessible name; it is a slider. */
  thresholdLabel: 'Auto-compact threshold',
  /** The marker's `aria-valuetext`, in place of the hover tooltip it used to have. */
  thresholdValue: (pct: number) => `Auto-compact at ${pct}%`,
  /** The marker's `aria-valuetext` while the saved threshold turns automatic compaction off. */
  thresholdOff: 'Automatic compaction off',
  /** The note under the bar; `pct` is what Biorouter compacts at, which the marker may not reach. */
  autoCompactNote: (pct: number) =>
    `Biorouter compacts this chat automatically at ${pct}% used. Drag the marker to change it.`,
  /**
   * The note while the saved threshold is 0 or 1 or outside them, which the daemon reads as off
   * (`check_if_compaction_needed`). Moving the marker saves a share inside them.
   */
  autoCompactOff: 'Automatic compaction is off. Drag the marker to turn it on.',
  compact: 'Compact chat',
  /** Under a disabled Compact, while the chat has used no context. */
  nothingToCompact: 'Nothing to compact yet.',
} as const;

/**
 * The cost figure's card (`CostTracker`, a `FooterCard`): the chat's total and its tokens, then
 * the cost per model. Money comes in already formatted (`formatCostEstimate` and
 * `formatTooltipMoney` in CostTracker.tsx, which print "Unavailable" for an unknown price); token
 * counts come in as numbers and are grouped here.
 */
export const COST_CARD_COPY = {
  /** The card's accessible name (it is a `dialog`). */
  cardLabel: 'Cost',
  section: 'Cost of this chat',
  /** The muted line under the total. */
  tokens: (input: number, output: number) =>
    `${input.toLocaleString()} input tokens · ${output.toLocaleString()} output tokens`,
  /** When the estimate is partial: a price or a count is missing. */
  partialNote: 'This is a conservative estimate from the token counts and prices available.',
  byModel: 'By model',
  /** The muted line under a model's name. */
  modelDetail: (cost: string, input: number, output: number) =>
    `${cost} · ${input.toLocaleString()} in · ${output.toLocaleString()} out`,
} as const;
