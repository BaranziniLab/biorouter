import type { SidebarGroupBy, SidebarSortBy } from './sidebarChatView';

/**
 * The sidebar's words, in one place so tests import them.
 *
 * ⚠ The nav labels (Home, New chat, Crew, the Components children, Settings)
 * are NOT here and must not move here: `landing/scripts/check-consistency.mjs`
 * greps `label: '…'` literals in `AppSidebar.tsx` and requires each one in the
 * landing site's app mockup (spec section 1, copy rules).
 */
export const sidebarCopy = {
  components: 'Components',
  chats: {
    header: 'Chats',
    viewOptions: 'View options',
    allChats: 'All chats',
    empty: 'No chats yet',
    loading: 'Loading chats…',
    loadingMore: 'Loading more chats…',
    showMore: 'Show more',
    showLess: 'Show less',
    openChat: (title: string) => `Open chat: ${title}`,
    openRunningChat: (title: string) => `Open ongoing chat: ${title}`,
    moreActions: (title: string) => `More actions for ${title}`,
    untitled: 'Untitled chat',
    messages: (count: number) => `${count} ${count === 1 ? 'message' : 'messages'}`,
    /**
     * The hover card's note on a chat a public decision model made public
     * (DR-33, owner decision 3): no glyph, just the reason.
     */
    publicBecause: (model: string, place: string, others = 0) => {
      const first = `Public: its decision model ${model} runs on ${place}`;
      if (others <= 0) return `${first}.`;
      return others === 1
        ? `${first}, and 1 other decision model is public too.`
        : `${first}, and ${others} other decision models are public too.`;
    },
  },
  folder: {
    newChat: (folder: string) => `New chat in ${folder}`,
    copyPath: 'Copy folder path',
    pathCopied: 'Folder path copied',
    copyFailed: "Couldn't copy the folder path",
    moreActions: (folder: string) => `More actions for ${folder}`,
  },
  view: {
    groupBy: 'Group by',
    sortBy: 'Sort by',
    groupByOptions: {
      date: 'Date',
      folder: 'Folder',
      none: 'None',
    } satisfies Record<SidebarGroupBy, string>,
    sortByOptions: {
      activity: 'Last activity',
      created: 'Created',
      name: 'Name',
    } satisfies Record<SidebarSortBy, string>,
  },
  update: {
    row: (version?: string | null) => (version ? `Update to ${version}` : 'Update available'),
  },
  shortcut: {
    newChat: (isMac: boolean) => (isMac ? '⌘T' : 'Ctrl+T'),
  },
  documentTitle: {
    separator: ' · ',
    history: 'Chat history',
  },
} as const;
