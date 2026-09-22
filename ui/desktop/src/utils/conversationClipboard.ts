export function writeConversationId(
  sender: { isAppWindow: boolean; isMainFrame: boolean },
  id: unknown,
  write: (text: string) => void
): void {
  if (!sender.isAppWindow || !sender.isMainFrame)
    throw new Error('Clipboard writes require the app main frame.');
  if (typeof id !== 'string' || id.length === 0 || id.length > 512)
    throw new Error('Conversation ID must be a nonempty string of at most 512 characters.');
  write(id);
}

export function writeSelectedText(
  sender: { isAppWindow: boolean; isMainFrame: boolean },
  text: unknown,
  write: (text: string) => void
): void {
  if (!sender.isAppWindow || !sender.isMainFrame)
    throw new Error('Clipboard writes require the app main frame.');
  if (typeof text !== 'string' || text.length === 0 || text.length > 1_000_000)
    throw new Error('Select between 1 and 1,000,000 characters to copy.');
  write(text);
}
