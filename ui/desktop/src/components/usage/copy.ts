import type { UsageRange } from '../../utils/usageStats';

/**
 * Every string the usage surfaces show: Home's usage card and Settings > App >
 * Usage (spec 3.9). Tests import these rather than repeating them.
 *
 * Copy rules (spec section 1): sentence case, "chat", no dashes as
 * punctuation; an InfoTip is at most two full sentences.
 */

const days = (count: number) => (count === 1 ? '1 day' : `${count.toLocaleString('en-US')} days`);

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
  notRecorded: 'Not recorded',
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
  rowLabel: 'This month',
  rowHelp: (month: string) =>
    `Billed tokens, estimated cost and model turns for ${month}, across all chats.`,
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
