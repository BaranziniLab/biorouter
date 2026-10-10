/**
 * Every string Chat history and the two transcript pages show, in one place
 * (spec 3.4, copy rules in section 1). Tests import these rather than spelling
 * the words again, so a copy change is one edit and the assertions follow it.
 *
 * The rules they follow: sentence case, "chat" (never "conversation"), the
 * typographic ellipsis, no dashes as punctuation, and explanations in an
 * InfoTip rather than a paragraph. Privacy refusals are not here: they live in
 * `declassifyOnBrowser.ts`, which is frozen.
 */

/* ------------------------------------------------------------------ History */

export const HISTORY_TITLE = 'Chat history';
/** InfoTip on the band title. Two sentences at most. */
export const HISTORY_INFO =
  'Every chat you have had with Biorouter. Open one to pick it up where you left off.';
/** The band filter's placeholder carries the shortcut, so the title needs no sentence for it. */
export const historySearchPlaceholder = (shortcut: string) => `Search history… ${shortcut}`;

export const SHOW_SUBAGENT_RUNS = 'Show subagent runs';
export const IMPORT_CHAT = 'Import chat';

export const LOADING_HISTORY = 'Loading chat history';
export const LOADING_MORE_CHATS = 'Loading more chats…';

export const HISTORY_LOAD_ERROR_TITLE = "Couldn't load your chat history";
export const HISTORY_LOAD_ERROR = 'Try again in a moment.';
export const TRY_AGAIN = 'Try again';

export const HISTORY_EMPTY_TITLE = 'No chats yet';
export const HISTORY_EMPTY = 'Your chats will appear here.';
export const START_A_CHAT = 'Start a chat';

export const HISTORY_NO_MATCH_TITLE = 'No matching chats';
export const HISTORY_NO_MATCH = 'Try a different name, folder, or chat ID.';

/* --------------------------------------------------------------------- Rows */

export const openChatLabel = (name: string) => `Open chat ${name}`;
export const RENAME = 'Rename';
export const renameLabel = (name: string) => `Rename ${name}`;
export const MORE_ACTIONS = 'More actions';
export const moreActionsLabel = (name: string) => `More actions for ${name}`;
export const deleteLabel = (name: string) => `Delete ${name}`;
export const SUBAGENT_BADGE = 'Subagent';
export const branchedFrom = (name: string) => `Branched from ${name}`;

export const MESSAGES_STAT = 'Messages';
export const BILLED_TOKENS_STAT = 'Billed tokens';
export const BILLED_TOKENS_LOWER_BOUND =
  'At least this many tokens. Only last-turn usage is available for this older chat.';
export const BILLED_TOKENS_EXACT =
  'Billed tokens across every turn, including recorded cache usage.';
export const EXTENSIONS_STAT = 'Extensions';

/* -------------------------------------------------------------- Row actions */

export const MAKE_CHAT_PUBLIC = 'Make this chat public';

export const RENAME_TITLE = 'Rename chat';
export const RENAME_PLACEHOLDER = 'Chat name';
export const CANCEL = 'Cancel';
export const SAVE = 'Save';
export const SAVING = 'Saving…';

/* The delete dialog and the menu labels are the row menu's own (`chats/copy.ts`),
   shared with the sidebar so the two never word the same act differently. */
export const CHAT_DELETED = 'Chat deleted';

export const CHAT_IMPORTED = 'Chat imported';

/* ------------------------------------------------------------------- Import */

export const IMPORT_TITLE = 'Import chat';
export const IMPORT_DROP = 'Drop a JSON file here';
export const IMPORT_BROWSE = 'or click to browse';
export const IMPORTING = 'Importing…';
export const IMPORT_NOT_JSON = 'Choose a JSON file.';
export const IMPORT_INVALID_JSON = 'That file is not valid JSON.';

/* ---------------------------------------------------------- Saved transcript */

export const BACK = 'Back';
export const SHARE = 'Share';
export const SHARING = 'Sharing…';
export const SHARE_NOT_SET_UP = 'Chat sharing is not set up on this install.';
export const RESUME = 'Resume';
export const LOADING_CHAT = 'Loading chat…';
export const CHAT_LOAD_ERROR_TITLE = "Couldn't load this chat";
export const NO_MESSAGES_TITLE = 'No messages in this chat';
export const NO_MESSAGES = 'Nothing was said in this chat.';
export const SEARCH_CHAT_PLACEHOLDER = 'Search this chat…';

export const SHARE_FAILED = "Couldn't share chat";
export const SHARE_READ_FAILED = 'The chat could not be read for sharing. Nothing was shared.';
export const SHARE_DIALOG_TITLE = 'Share chat';
export const SHARE_DIALOG_SUBTITLE = 'Anyone with this link can read this chat.';
export const COPY_LINK = 'Copy link';
export const LINK_COPIED = 'Link copied';
export const COPY_LINK_FAILED = "Couldn't copy link";
export const COPY_LINK_FAILED_MSG = 'The chat link could not be copied to the clipboard.';
export const DONE = 'Done';
export const RESUME_FAILED = "Couldn't open this chat";

/** "12 messages", the count every transcript header shows. */
export const messageCount = (count: number) => `${count} ${count === 1 ? 'message' : 'messages'}`;
/** InfoTip on a saved transcript's title: where the chat ran, which only a tooltip should name. */
export const savedChatInfo = (workingDir: string) => `This chat ran in ${workingDir}.`;

/* ------------------------------------------------------------ Shared chat */

export const SHARED_CHAT = 'Shared chat';
export const SHARED_CHAT_INFO = 'A read-only copy of a chat someone shared with you.';
export const YOU = 'You';
export const BIOROUTER = 'Biorouter';
