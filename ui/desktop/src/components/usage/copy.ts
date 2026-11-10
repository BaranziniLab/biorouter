import type { UsageRange } from '../../utils/usageStats';

/**
 * Every string the usage surfaces show: Home's usage card and Settings > App >
 * Usage (spec 3.9). Tests import these rather than repeating them.
 *
 * Copy rules (spec section 1): sentence case, "chat", no dashes as
 * punctuation; an InfoTip is at most two full sentences.
 */

const days = (count: number) => (count === 1 ? '1 day' : `${count.toLocaleString('en-US')} days`);

/**
 * The one name for usage saved without a model: the report's and the ledger's `null` model
 * bucket (turns recorded before model attribution, or from a provider that reported no model).
 * Declared once here and read by every surface that lists usage by model: Home's usage card
 * (`homeUsageCopy.notRecorded`), Settings > App > Usage's model table (`modelLabel` in
 * UsagePanel.tsx) and the composer's cost card (`modelLabel` in bottom_menu/ModelBreakdownTable.tsx,
 * through the re-export in bottom_menu/copy.ts). A second name for the same bucket reads as a second
 * bucket; `copy.test.ts` ties the three to this constant. Row keys keep `UNKNOWN_MODEL_LABEL`
 * (`hooks/useCostTracking.ts`), which no one reads.
 */
export const MODEL_NOT_RECORDED = 'Not recorded';

export const homeUsageCopy = {
  /** The card's region name. */
  region: 'Usage',
  tabsLabel: 'Usage view',
  overview: 'Overview',
  models: 'Models',
  /** The Models tab reads the billing report, whose scope differs from Overview. */
  modelsHelp: 'Includes deleted chats and subagents.',
  modelsEmpty: 'No model usage in this period.',
  other: 'Other',
  /** Usage the report holds without a model name: last row, and the tile when it is all there is. */
  notRecorded: MODEL_NOT_RECORDED,
  notRecordedHelp: 'Usage saved without a model name. The shares above count named models only.',
  rangeLabel: 'Time range',
  ranges: {
    '1y': { label: '1y', name: 'Last year' },
    '30d': { label: '30d', name: 'Last 30 days' },
    '7d': { label: '7d', name: 'Last 7 days' },
  } satisfies Record<UsageRange, { label: string; name: string }>,
  /** The loading card's status name; it shows no visible text. */
  loading: 'Loading usage activity',
  /** A figure that has not arrived yet. */
  pending: '…',
  none: 'None',
  chats: 'Chats',
  messages: 'Messages',
  messagesHelp:
    'Counts messages on days with a new chat or billed tokens, so this is a lower bound.',
  tokens: 'Tokens',
  tokensPartialHelp: 'Some days lack full token records, so this total is a lower bound.',
  tokensUnavailable: 'Unavailable',
  tokensUnavailableHelp: 'No trustworthy token total was recorded for this period.',
  activeDays: 'Active days',
  streak: 'Streak',
  favoriteModel: 'Favorite model',
  longestStreak: 'Longest streak',
  dayUnit: (count: number) => (count === 1 ? 'day' : 'days'),
  streakHelp: (longest: number) => `Longest in the last year: ${days(longest)}.`,
} as const;

export const settingsUsageCopy = {
  sectionTitle: 'Usage',
  sectionHelp: 'Billed tokens and estimated cost for the current month, grouped by day and model.',
  /** The same help when Settings > App > Show costs is off: it must not mention a price. */
  sectionHelpNoCost: 'Billed tokens for the current month, grouped by day and model.',
  rowLabel: 'This month',
  rowHelp: (month: string) =>
    `Billed tokens, estimated cost and model turns for ${month}, across all chats.`,
  rowHelpNoCost: (month: string) => `Billed tokens and model turns for ${month}, across all chats.`,
  details: 'Details',
  /** The Details button's accessible name. */
  detailsName: 'Open detailed usage report',
  dialogTitle: (monthLabel: string) => `Usage, ${monthLabel}`,
  loadError: 'Usage data could not be loaded. Check the backend connection and try again.',
  loadErrorStale: ' Showing the last loaded month-to-date report.',
  retry: 'Retry',
  refreshing: 'Refreshing usage…',
  tokens: 'Tokens',
  tokensHelp: 'Billed tokens recorded this month, including deleted chats and subagents.',
  tokensCacheHelp: (read: string, write: string) =>
    `Billed tokens recorded this month, including deleted chats and subagents. Cache read ${read}, cache write ${write}.`,
  cost: 'Est. cost',
  costHelp: 'Based on stored model pricing.',
  turns: 'Turns',
  turnsHelp: 'Completed model turns.',
  conservative: 'Known conservative subtotal',
  tokenBudget: 'Token budget',
  dollarBudget: 'Dollar budget',
  byDay: 'By day',
  byModel: 'By model',
  empty: 'No usage this month.',
  day: 'Day',
  model: 'Model',
  tokenFlow: 'Token flow',
  billed: 'Billed',
  costColumn: 'Cost',
  unattributedProvider: 'Unattributed provider',
  estimates: 'Some totals are estimates.',
  estimatesLabel: 'these totals',
  estimatesHelp:
    'Some token, cache or pricing records were not kept, so those figures are lower bounds.',
  incompleteNote:
    'Some historical token details were not recorded. Totals are conservative, and empty cache or cost fields stay hidden.',
  unpricedNote:
    'Some historical usage has no stored model or pricing attribution, so its cost cannot be recovered.',
  cacheExcludedNote:
    'Some cache cost is unavailable because cache pricing or historical accounting is incomplete.',
} as const;

/**
 * Settings > App > Usage's Limits group (`settings/usage/UsageLimitsRows.tsx`): the monthly limit
 * and the default limit per chat. Each is "No limit" until a person sets one, in tokens or, while
 * Show costs is on, dollars. A chat's own limit is set from the cost in its composer
 * (`USAGE_LIMIT_COPY` in bottom_menu/copy.ts).
 */
export const settingsLimitsCopy = {
  groupTitle: 'Limits',
  monthly: 'Monthly limit',
  monthlyHelp:
    'Stops every chat before its next model request once this month reaches it. Resets on the 1st, local time.',
  defaultPerChat: 'Default limit per chat',
  defaultPerChatHelp:
    "Applies to every chat without its own limit. Set a chat's own limit from the cost under its composer.",
  noLimit: 'No limit',
  /** A row's value before the first read of the month has answered. */
  pending: '…',
  /** A dollar limit while Show costs is off: no figure is shown. */
  dollarLimitHidden: 'Dollar limit',
  change: 'Change',
  changeName: (label: string) => `Change ${label.toLowerCase()}`,
  amountLabel: (label: string) => `${label} amount`,
  unitLabel: 'Limit unit',
  dollars: 'Dollars',
  tokens: 'Tokens',
  save: 'Save',
  removeLimit: 'No limit',
  removeLimitName: (label: string) => `Remove the ${label.toLowerCase()}`,
  cancel: 'Cancel',
  invalidUsd: 'Enter an amount above $0.',
  invalidTokens: 'Enter a number of tokens above 0, such as 5m.',
  saveFailed: 'The limit could not be saved. Try again.',
  needUser: 'Biorouter could not confirm this change came from you. Try again.',
  browserNote: 'Change limits in the Biorouter desktop app.',
} as const;
