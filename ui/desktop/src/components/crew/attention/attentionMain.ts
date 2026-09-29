/**
 * The main process's half of Crew's attention signals (M2): what it accepts from a window, the
 * dock badge across windows, and when a notification is shown. Electron-free and free of any
 * renderer import, so `main.ts` can use it and it is tested without a window. Its one import,
 * `utils/untrustedText`, imports nothing.
 */
import { stripHiddenCharacters } from '../../../utils/untrustedText';

/** At most one notification per channel in this long. */
export const NOTIFY_INTERVAL_MS = 60_000;

/** The longest title or body a Crew notification may carry. */
export const NOTIFICATION_TEXT_MAX_CHARS = 300;

/** Connection and channel ids, as every Crew IPC validates them. */
const ID = /^[a-zA-Z0-9_-]{1,128}$/;

/** At most this many Crew notifications in any one interval, whatever channels they name. */
export const NOTIFY_MAX_PER_INTERVAL = 6;

/**
 * At most one notification per key per interval, and at most `maxPerInterval` of any keys: a
 * busy workspace, or a window that names channel after channel, cannot fill the screen.
 */
export class AttentionThrottle {
  private readonly last = new Map<string, number>();
  constructor(
    private readonly intervalMs = NOTIFY_INTERVAL_MS,
    private readonly maxPerInterval = NOTIFY_MAX_PER_INTERVAL
  ) {}
  allow(key: string, now: number): boolean {
    for (const [known, at] of this.last) if (now - at >= this.intervalMs) this.last.delete(known);
    if (this.last.has(key) || this.last.size >= this.maxPerInterval) return false;
    this.last.set(key, now);
    return true;
  }
}

export interface CrewAttentionRequest {
  key: string;
  title: string;
  body: string;
  connectionId: string;
  channelId: string;
}

/** A lone surrogate: half of a character, which no line should carry. */
const LONE_SURROGATE = /\p{Cs}/gu;

/**
 * Private-use characters draw a glyph of some font's choosing. They are neither controls nor
 * format characters, so {@link stripHiddenCharacters} keeps them; a notification line does not.
 */
const PRIVATE_USE = /\p{Co}/gu;

/**
 * One plain line: line breaks (JavaScript's `\s` includes U+2028 and U+2029, the line and
 * paragraph separators) become spaces first, so two lines stay two words; then every control and
 * format character goes through the renderer's one drop set, `stripHiddenCharacters`, and the
 * private-use characters after it. Lone surrogates go before any of that, as `untrustedText.ts`
 * requires: removing a format character between two halves would otherwise fuse them into one
 * character, a private-use or tag character among them, after the pass that could see it.
 */
function line(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const text = stripHiddenCharacters(value.replace(LONE_SURROGATE, '').replace(/\s+/g, ' '))
    .replace(PRIVATE_USE, '')
    .replace(/<[^>]*>/g, '')
    .replace(/ {2,}/g, ' ')
    .trim();
  if (!text) return null;
  const characters = Array.from(text);
  return characters.length > NOTIFICATION_TEXT_MAX_CHARS
    ? `${characters.slice(0, NOTIFICATION_TEXT_MAX_CHARS - 1).join('')}…`
    : text;
}

/** The notification a window asked for, validated and made plain, or `null` to ignore it. */
export function parseAttentionRequest(raw: unknown): CrewAttentionRequest | null {
  if (!raw || typeof raw !== 'object') return null;
  const request = raw as Record<string, unknown>;
  const title = line(request.title);
  const body = line(request.body) ?? '';
  if (
    !title ||
    typeof request.connectionId !== 'string' ||
    typeof request.channelId !== 'string' ||
    !ID.test(request.connectionId) ||
    !ID.test(request.channelId)
  )
    return null;
  return {
    // Keyed by the ids, never by what the window called it, so a window cannot dodge the limit.
    key: `${request.connectionId}:${request.channelId}`,
    title,
    body,
    connectionId: request.connectionId,
    channelId: request.channelId,
  };
}

/**
 * Whether the window that asked may notify now: when it is the one in front (it has already
 * checked that it is not showing Crew), or when no window of the app is in front. Never while
 * another window is in front: that window's own watcher decides for it.
 */
export function attentionNotificationAllowed(
  senderWindowId: number,
  focusedWindowId: number | null
): boolean {
  return focusedWindowId === null || focusedWindowId === senderWindowId;
}

/** Each window's unread count, and the one the dock shows: the largest. */
export class AttentionBadges {
  private readonly counts = new Map<number, number>();

  /** Record a window's count; the dock's count afterwards. */
  set(windowId: number, raw: unknown): number {
    const count =
      typeof raw === 'number' && Number.isSafeInteger(raw) && raw > 0 ? Math.min(raw, 99_999) : 0;
    if (count > 0) this.counts.set(windowId, count);
    else this.counts.delete(windowId);
    return this.badge();
  }

  /** The window closed; the dock's count afterwards. */
  forget(windowId: number): number {
    this.counts.delete(windowId);
    return this.badge();
  }

  badge(): number {
    let largest = 0;
    for (const count of this.counts.values()) largest = Math.max(largest, count);
    return largest;
  }
}
