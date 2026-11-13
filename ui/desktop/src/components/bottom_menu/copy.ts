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
 * One string here is not the footer's own: `MODEL_NOT_RECORDED`, re-exported from the usage
 * surfaces' copy (`components/usage/copy.ts`), so the cost card names a row with no model exactly
 * as Home's usage card and Settings > App > Usage do.
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
  /**
   * The folder item's name: the full path, which the line itself does not show, and the word the
   * line DOES show (`shown`), so the name contains the visible text (WCAG 2.5.3). Where that word
   * is not in the path (`~` for the home folder, `Crew` for the Crew task folder), it comes first:
   * "Working folder: ~ (/Users/me)".
   */
  folderLabel: (dir: string, shown?: string) =>
    shown && !dir.includes(shown) ? `Working folder: ${shown} (${dir})` : `Working folder: ${dir}`,
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
 * What the cost card's By model list (`modelLabel` in ModelBreakdownTable.tsx) calls the ledger's
 * `null` model bucket. Not declared here: the usage surfaces' copy owns the name, so this card,
 * Home's usage card and Settings > App > Usage never name the bucket differently. It is only what a
 * person reads: row keys and the legacy cost map's keys keep `UNKNOWN_MODEL_LABEL`
 * (`hooks/useCostTracking.ts`).
 */
export { MODEL_NOT_RECORDED } from '../usage/copy';

/**
 * The tokens the cost card lists for the chat or for one model: the ledger's four disjoint
 * buckets (`ModelUsageRow`), which add up to the billed total the summary's Tokens value and
 * Settings > App > Usage show. A cache bucket is `null` (or absent) where it was not recorded: the
 * live price's counters never split it out, and history from before cache accounting has none.
 */
export interface CostCardTokens {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens?: number | null;
  cacheCreationTokens?: number | null;
}

/** A no-break space: a figure keeps its word, as a number keeps its unit. */
const NBSP = '\u00a0';

/** "65,408 cache read", in one piece. */
function tokenBucket(count: number, word: string): string {
  return `${count.toLocaleString()} ${word}`.replace(/ /g, NBSP);
}

/**
 * Buckets joined by "·", each separator held to the bucket before it, so the only place a line
 * can break is after a "·". Measured in Chromium at the card's 294px column: an unpriced model's
 * row ("Unavailable · 20,000 in · 1,500 out · 300,000 cache read") and a chat of millions broke
 * between "cache" and "read" with plain spaces; with these they break after the last "·" that fits.
 */
function joinBuckets(parts: string[]): string {
  return parts.join(`${NBSP}· `);
}

/**
 * A detail's lines: `lead`, then each cache bucket that holds tokens, in Settings > App > Usage's
 * order (in, out, cache read, cache write). A bucket with none, or not recorded, says nothing.
 * Input, output and one cache bucket fit the card's 294px column on one line (284px at a million
 * input tokens and ten million cache read, 12px Arial), so they stay on one. With both cache
 * buckets the line runs to 355px at an ordinary chat's counts, so the cache takes a second line of
 * its own instead of breaking wherever the column ends.
 */
function costTokenLines(lead: string[], tokens: CostCardTokens): string[] {
  const cacheRead = tokens.cacheReadTokens ?? 0;
  const cacheWrite = tokens.cacheCreationTokens ?? 0;
  const cache = [
    ...(cacheRead > 0 ? [tokenBucket(cacheRead, 'cache read')] : []),
    ...(cacheWrite > 0 ? [tokenBucket(cacheWrite, 'cache write')] : []),
  ];
  return cache.length > 1
    ? [joinBuckets(lead), joinBuckets(cache)]
    : [joinBuckets([...lead, ...cache])];
}

/**
 * The cost figure's card (`CostTracker`, a `FooterCard`): the chat's total and its tokens, then
 * the cost per model. Money comes in already formatted (`formatCostEstimate` and
 * `formatTooltipMoney` in CostTracker.tsx, which print "Unavailable" for an unknown price); token
 * counts come in as numbers and are grouped here. A detail is one line, or two (`costTokenLines`),
 * drawn as one muted paragraph.
 */
export const COST_CARD_COPY = {
  /** The card's accessible name (it is a `dialog`). */
  cardLabel: 'Cost',
  section: 'Cost of this chat',
  /** The muted line under the total: "22,788 input · 382 output · 65,408 cache read". */
  tokens: (tokens: CostCardTokens) =>
    costTokenLines(
      [tokenBucket(tokens.inputTokens, 'input'), tokenBucket(tokens.outputTokens, 'output')],
      tokens
    ),
  /** When the estimate is partial: a price or a count is missing. */
  partialNote: 'This is a conservative estimate from the token counts and prices available.',
  byModel: 'By model',
  /** The muted line under a model's name: "$0.14 · 22,788 in · 382 out · 65,408 cache read". */
  modelDetail: (cost: string, tokens: CostCardTokens) =>
    costTokenLines(
      [cost, tokenBucket(tokens.inputTokens, 'in'), tokenBucket(tokens.outputTokens, 'out')],
      tokens
    ),
} as const;

/**
 * The cost card's Limit section (`UsageLimitSection` in CostTracker.tsx), the footer trigger's
 * limit state, and the composer notice a paused chat shows (`ChatInput`). Amounts come in already
 * formatted (`formatLimitUsd`, `formatLimitTokens` in components/usage/usageLimits.ts). Where the
 * monthly and default limits are set is Settings > App > Usage (`settingsLimitsCopy`).
 */
export const USAGE_LIMIT_COPY = {
  section: 'Limit',
  checking: 'Checking the limit…',
  loadFailed: 'The limit could not be loaded.',
  noLimit: 'No limit',
  /** Where the limit in force comes from, under its value. */
  source: {
    chat: 'Set for this chat',
    default: 'Default from Settings',
    unlimited: 'No limit for this chat',
    none: 'No default limit is set',
  },
  used: (used: string, limit: string) => `${used} of ${limit} used`,
  /** "12,000 of 100,000 tokens used": the counts come in as numbers. */
  usedTokens: (used: number, limit: number) =>
    `${Math.round(used).toLocaleString()} of ${Math.round(limit).toLocaleString()} tokens used`,
  usedOnly: (used: string) => `${used} used`,
  /**
   * Who the used line counts. A chat's limit counts the chat and every subagent it started, while
   * "Cost of this chat" above it counts the chat's own requests only (its side calls, such as the
   * title, included), so the line says so.
   */
  chatUsed: (reading: string) => `This chat and its subagents: ${reading}`,
  /** In a subagent's chat the line is the parent's, which counts the parent and its subagents. */
  parentUsed: (reading: string) => `The chat that started this one, with its subagents: ${reading}`,
  month: (reading: string) => `This month: ${reading}`,
  pausedChat: 'This chat reached its limit. Raise or remove it to continue.',
  pausedMonth: 'The monthly limit is reached. Raise it in Settings to continue.',
  /** Paused because the next request is estimated to pass the limit, not because it is reached. */
  pausedChatNext: "The next request would pass this chat's limit. Raise or remove it to continue.",
  pausedMonthNext:
    'The next request would pass the monthly limit. Raise it in Settings to continue.',
  /** "Next request: about $0.0074, $0.0035 left." Amounts come in formatted. */
  nextRequest: (estimate: string, left: string) => `Next request: about ${estimate}, ${left} left.`,
  /** One request alone is estimated past the whole limit. */
  nextRequestTooBig: (estimate: string) =>
    `One request is estimated at ${estimate}, more than the whole limit.`,
  warnChat: 'This chat is near its limit.',
  warnMonth: 'Usage is near the monthly limit.',
  chatDollarInert: 'Nothing this chat used has a price, so its dollar limit cannot stop it.',
  monthDollarInert: 'Nothing this month has a price, so the monthly dollar limit cannot stop it.',
  parentChat: 'This chat counts toward the limit of the chat that started it.',
  browserNote: 'Limits can be changed in the Biorouter desktop app.',
  noChatYet: 'A limit can be set once the chat has started. Until then the default applies.',
  setLimit: 'Set limit…',
  changeLimit: 'Change limit…',
  noLimitForChat: 'No limit',
  noLimitForChatName: 'No limit for this chat',
  useDefault: 'Use default',
  fieldLabel: 'Limit for this chat',
  /** The field's accessible name, which says the unit the visible label leaves to the switch. */
  fieldNameDollars: 'Limit for this chat, in dollars',
  fieldNameTokens: 'Limit for this chat, in tokens',
  unitLabel: 'Limit unit',
  dollars: 'Dollars',
  tokens: 'Tokens',
  save: 'Save',
  cancel: 'Cancel',
  invalidUsd: 'Enter an amount above $0.',
  invalidUsdPrecision: 'Use at most 4 decimal places, such as 0.0125.',
  invalidTokens: 'Enter a number of tokens above 0, such as 500k.',
  /** The dollar unit is chosen for a chat whose model has no price. */
  unpricedDollar:
    "This chat's model has no price, so a dollar limit cannot stop it. Set a token limit instead.",
  saveFailed: 'The limit could not be saved. Try again.',
  needUser: 'Biorouter could not confirm this change came from you. Try again.',
  openSettings: 'Open usage settings',
  /** The footer trigger's name when there is no cost figure to show. */
  triggerLabel: 'Usage and limit',
  /** Added to the trigger's name for a chat near or at a limit. */
  triggerWarn: 'near its usage limit',
  triggerPaused: 'paused by a usage limit',
  /** The one-line composer notice while a limit holds the chat. */
  noticeChat: 'This chat reached its usage limit.',
  noticeMonth: 'The monthly usage limit is reached.',
  noticeChatNext: "This chat's next request would pass its usage limit.",
  noticeMonthNext: 'The next request would pass the monthly usage limit.',
  noticeChatAction: 'Change limit',
  noticeMonthAction: 'Open usage settings',
  /** Send's tooltip while the chat is paused. */
  sendPaused: 'Paused by a usage limit',
} as const;
