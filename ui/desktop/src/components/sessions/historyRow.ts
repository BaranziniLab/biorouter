import { chatKindOf, type ChatKindSource } from '../chats/chatKind';

/**
 * Small pure helpers for a chat row's second line in Chat history and the
 * transcript headers. Kept free of React so they are tested on their own.
 */

/**
 * The folder a chat ran in, by name: the last segment of its working directory.
 *
 * A row shows names, not paths (principle 10); the full path stays reachable in
 * the row's tooltip and in the search. Trailing separators are ignored, both
 * separators are understood (a Windows path reaches a macOS window through an
 * import), and the root itself reads as `/`.
 */
export function folderName(path: string | null | undefined): string {
  if (!path) return '';
  const trimmed = path.replace(/[\\/]+$/, '');
  if (!trimmed) return path.startsWith('\\') ? '\\' : '/';
  const segments = trimmed.split(/[\\/]/);
  return segments[segments.length - 1] || trimmed;
}

/** The daemon names a delegated run `Subagent: <label>` (`subagent_tool.rs`). */
const SUBAGENT_NAME_PREFIX = /^Subagent:\s+/i;

/** `living-news-router` becomes `Living news router`: words, sentence case. */
function humanizeSlug(slug: string): string {
  const words = slug.split(/[-_]+/).filter(Boolean).join(' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * The title a History row shows: the chat's name, minus what the kind glyph already says
 * (principle 2, say it once; principle 10, names not IDs; vision V1-R2-10).
 *
 * - An app's chat is stored as `app:<app id>` (`app:<app id>:<worker profile>` for a worker
 *   profile's own chat). The App glyph says "app", and a slug with a colon is a machine
 *   string, so the row reads "Living news router", with the profile after it in parentheses.
 * - A sub-agent run is stored as `Subagent: <label>`. The robot glyph and the rail that ties
 *   it to its parent say that; the row shows the label. A name that is only the prefix is kept
 *   as it is, so a row never renders empty.
 *
 * Display only. The stored name is untouched, the row's filter still matches it (see
 * `matchesFilter`), and every other surface keeps its own spelling.
 */
export function historyRowTitle(session: ChatKindSource): string {
  const name = session.name ?? '';
  const kind = chatKindOf(session);
  if (kind === 'app') {
    const [, id = '', ...profile] = name.trim().split(':');
    const app = humanizeSlug(id.trim());
    if (!app) return name;
    const worker = humanizeSlug(profile.join(':').trim()).toLowerCase();
    return worker ? `${app} (${worker})` : app;
  }
  if (kind === 'subagent') {
    const label = name.replace(SUBAGENT_NAME_PREFIX, '').trim();
    return label || name;
  }
  return name;
}
