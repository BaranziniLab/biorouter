import type { Message } from '../../api';
import {
  getElicitationContent,
  getSecretRequestContent,
  getTextContent,
  getToolConfirmationContent,
  getToolRequests,
} from '../../types/message';

/**
 * The transcript's vertical rhythm, as a pure function (spec 3.6, chat-interface
 * B1, B2, B14): 24px between turns, 8px between the blocks of one turn, 4px
 * between consecutive tool rows.
 *
 * It replaces three boxes that each declared part of the gap: a 28px reserved
 * meta row under every message, a 24px `.assistant:has(+ .user)` padding and a
 * 16px `mt-[16px]` on the user root, which together put about 84px between an
 * answer and the next question and only 12 to 16px between blocks of one
 * answer. One gap per row, decided here, is the whole rule.
 */
export type RowGap = 'first' | 'turn' | 'block' | 'tool' | 'none';

/**
 * The margin class each gap draws. `none` is a row that renders nothing.
 *
 * The first row also takes 20px of padding at the top of the transcript: room
 * for its floating actions, which rise up to 20px above a message (above the
 * user bubble, `message-meta.css`) and would otherwise be clipped by the
 * scroll area's top edge.
 */
export const ROW_GAP_CLASS: Record<RowGap, string> = {
  first: 'mt-0 pt-5',
  turn: 'mt-6',
  block: 'mt-2',
  tool: 'mt-1',
  none: 'mt-0',
};

const INFO_MSG = /<info-msg>[\s\S]*?<\/info-msg>(\s*\\?")?/gi;
const THINK = /<think>([\s\S]*?)<\/think>/i;

/**
 * An assistant message's text as the transcript shows it: the injected
 * context blocks (model-only) removed, and the chain of thought split off.
 */
export function splitAssistantText(message: Message): {
  visibleText: string;
  cotText: string | null;
} {
  const text = getTextContent(message).replace(INFO_MSG, '').trim();
  const match = text.match(THINK);
  if (!match) return { visibleText: text, cotText: null };
  return { visibleText: text.replace(THINK, '').trim(), cotText: match[1].trim() || null };
}

/**
 * A user message that carries only tool results (or nothing at all) draws no
 * bubble in the transcript.
 */
export function isToolResponseOnly(message: Message): boolean {
  return message.content.every((c) => c.type === 'toolResponse');
}

function hasCard(message: Message): boolean {
  return (
    getToolConfirmationContent(message) !== undefined ||
    getElicitationContent(message) !== undefined ||
    getSecretRequestContent(message) !== undefined
  );
}

/** The assistant's first visible block is a tool row: no thinking and no prose above it. */
function startsWithToolRow(message: Message): boolean {
  if (getToolRequests(message).length === 0) return false;
  const { visibleText, cotText } = splitAssistantText(message);
  return !visibleText && !cotText;
}

/** The assistant's last visible block is a tool row: no card under it. */
function endsWithToolRow(message: Message): boolean {
  return getToolRequests(message).length > 0 && !hasCard(message);
}

function isInlineNotification(message: Message): boolean {
  return message.content.some(
    (content) =>
      content.type === 'systemNotification' && content.notificationType === 'inlineMessage'
  );
}

/**
 * An agent message that draws no block: no thought, no words, no tool row, no
 * card and no inline notice. The stream leaves these behind between tool calls
 * (an empty text part the model sent before its next call), and each one sat in
 * the list as a zero-height row with a block's 8px margin, so two tool rows in
 * separate messages stood 16px apart instead of 4px (vision V2-03). Such a row
 * takes no gap and is invisible to its neighbours' rhythm.
 *
 * Mirrors what `BioRouterMessage` and the list's inline-notice branch render.
 */
export function drawsNoBlock(message: Message): boolean {
  if (isInlineNotification(message)) return false;
  if (getToolRequests(message).length > 0 || hasCard(message)) return false;
  const { visibleText, cotText } = splitAssistantText(message);
  return !visibleText && !cotText;
}

/** A message the transcript shows nothing for, on either side. */
function isBlank(message: Message, user: boolean): boolean {
  return user ? isToolResponseOnly(message) : drawsNoBlock(message);
}

/**
 * The gap above every message, index for index. `isUser` is the list's own
 * predicate, so a surface that decides roles differently is not second-guessed.
 *
 * A message that draws nothing takes `none` and is skipped as a neighbour: the
 * gap of the next visible block is decided by the last VISIBLE block, so tool
 * rows are 4px apart whether they share a message or not.
 */
export function messageGaps(messages: Message[], isUser: (message: Message) => boolean): RowGap[] {
  let previous: { role: 'user' | 'assistant'; message: Message } | null = null;
  return messages.map((message) => {
    if (!message.metadata.userVisible) return 'none';
    const user = isUser(message);
    if (isBlank(message, user)) return 'none';
    const role = user ? 'user' : 'assistant';
    let gap: RowGap;
    if (!previous) gap = 'first';
    else if (previous.role !== role) gap = 'turn';
    else if (
      role === 'assistant' &&
      endsWithToolRow(previous.message) &&
      startsWithToolRow(message)
    )
      gap = 'tool';
    else gap = 'block';
    previous = { role, message };
    return gap;
  });
}

/**
 * The gap above the trailing activity line: the agent's line, so a turn's
 * width after a question, a tool row's pitch after a tool row, a block's
 * otherwise.
 */
export function trailingGap(messages: Message[], isUser: (message: Message) => boolean): RowGap {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!message.metadata.userVisible) continue;
    const user = isUser(message);
    if (isBlank(message, user)) continue;
    if (user) return 'turn';
    return endsWithToolRow(message) ? 'tool' : 'block';
  }
  return 'first';
}
