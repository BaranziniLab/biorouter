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
