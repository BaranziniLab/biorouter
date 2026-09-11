/** The shape both selection endpoints answer with — GET /active and POST /active. */
type SelectionPayload =
  | { primary_kb?: string | null; active_kb?: string | null; hidden_kbs?: string[] | null }
  | undefined;

/** `active_kb` is the deprecated mirror, read so a fresh renderer keeps working
 * against a daemon that predates `primary_kb`. */
export function readPrimary(data: SelectionPayload): string | null {
  return data?.primary_kb ?? data?.active_kb ?? null;
}

/** `null` means "this answer did not state a set" (a daemon that predates the
 * field) — distinct from an empty set, and the caller must leave what it has
 * rather than erase the session's whole working set. */
export function readHidden(data: SelectionPayload): string[] | null {
  return Array.isArray(data?.hidden_kbs)
    ? data.hidden_kbs.filter((id): id is string => typeof id === 'string')
    : null;
}
