import type { OwnAgentChat } from '../timeline';

/**
 * Which of the viewer's own chats each of their agent's runs posted as, remembered on this
 * computer (Q4-12).
 *
 * A post reads "Your agent · {chat title}" from the viewer's grant list, and the daemon lists one
 * grant per chat: a revoke, a re-grant (a new run) or an expiry took the old run out of the list,
 * and every post that run made fell back to "Your agent @crew_jack" — yesterday's posts included.
 * So every chat grant the layout sees is remembered here by its run, and the timeline reads the
 * remembered runs together with the current ones.
 *
 * - Only what `ownAgentChatsFrom` built from THIS device's own grants is ever written, so it names
 *   only the viewer's own chats; the timeline still applies it only to posts the viewer's agent
 *   wrote.
 * - One record per connection (`crew:agentChats:v1:{connectionId}`), at most
 *   {@link AGENT_CHAT_MEMORY_LIMIT} runs, the oldest dropped first.
 * - `localStorage`, with every access in try/catch: storage that is missing, full or refused
 *   leaves the byline to the current grants, as before.
 *
 * Display only. It grants, restores and decides nothing; the daemon's grant list is still the one
 * answer to what a chat may do.
 */

export const AGENT_CHAT_MEMORY_LIMIT = 200;
/** A title longer than this is not a title anyone reads whole; it is cut before it is stored. */
const TITLE_LIMIT = 300;

export const agentChatMemoryKey = (connectionId: string) => `crew:agentChats:v1:${connectionId}`;

interface StoredChat {
  run: string;
  title: string;
  session: string;
}

function parse(raw: string | null): StoredChat[] {
  if (!raw) return [];
  const value: unknown = JSON.parse(raw);
  if (!Array.isArray(value)) return [];
  return value.flatMap((item): StoredChat[] => {
    if (!item || typeof item !== 'object') return [];
    const { run, title, session } = item as Record<string, unknown>;
    if (typeof run !== 'string' || !run) return [];
    if (typeof title !== 'string' || !title.trim()) return [];
    if (typeof session !== 'string' || !session) return [];
    return [{ run, title: title.trim(), session }];
  });
}

function read(connectionId: string): StoredChat[] {
  try {
    return parse(globalThis.localStorage?.getItem(agentChatMemoryKey(connectionId)) ?? null);
  } catch {
    return [];
  }
}

/** The remembered chats of this connection, by run. Empty when there are none or none can be read. */
export function rememberedAgentChats(connectionId: string): Map<string, OwnAgentChat> {
  return new Map(
    read(connectionId).map(({ run, title, session }) => [run, { title, sessionId: session }])
  );
}

/**
 * Remember these chats (the current ones, by run) for this connection: the newest last, the
 * oldest dropped past the limit. Writes nothing when nothing changed.
 */
export function rememberAgentChats(
  connectionId: string,
  chats: ReadonlyMap<string, OwnAgentChat>
): void {
  if (!connectionId || chats.size === 0) return;
  try {
    const storage = globalThis.localStorage;
    if (!storage) return;
    const key = agentChatMemoryKey(connectionId);
    const before = storage.getItem(key);
    let stored: StoredChat[] = [];
    try {
      stored = parse(before);
    } catch {
      stored = [];
    }
    const fresh: StoredChat[] = [...chats].map(([run, chat]) => ({
      run,
      title: chat.title.slice(0, TITLE_LIMIT),
      session: chat.sessionId,
    }));
    const next = [...stored.filter((item) => !chats.has(item.run)), ...fresh].slice(
      -AGENT_CHAT_MEMORY_LIMIT
    );
    const text = JSON.stringify(next);
    if (text !== before) storage.setItem(key, text);
  } catch {
    // A per-viewer convenience: without storage the byline follows the current grants only.
  }
}

/**
 * The chats the timeline heads posts with: the remembered runs, with the current grants over them
 * (a current title wins over a remembered one).
 */
export function withRememberedAgentChats(
  connectionId: string,
  current: ReadonlyMap<string, OwnAgentChat>
): ReadonlyMap<string, OwnAgentChat> {
  const merged = rememberedAgentChats(connectionId);
  for (const [run, chat] of current) merged.set(run, chat);
  return merged;
}
