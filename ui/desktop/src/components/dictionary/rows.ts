/**
 * What the Dictionary page shows for its rows, free of React ("Groups" and
 * "Rows" in `docs/dictation/dictionary.md`): which group a row is in, its
 * second line, its meta and which warning comes before **Make public…**.
 */
import type { DictionaryEntryView } from '../../api';
import {
  DICTIONARY_LABELS,
  ROW_PRIVATE_INSTITUTIONS,
  dictionaryMessage,
  joinInstitutions,
  type DictionaryMessageId,
} from './copy';
import { fillMessage } from '../dictation/copy';

export type GroupKey = 'suggested' | 'agent' | 'user' | 'chats' | 'imported' | 'matches';

export interface EntryGroup {
  key: GroupKey;
  title: string;
  entries: DictionaryEntryView[];
}

/** Which group a row belongs to when no filter is typed. */
export function groupOf(entry: DictionaryEntryView): Exclude<GroupKey, 'matches'> {
  if (entry.status === 'suggested') return 'suggested';
  switch (entry.source) {
    case 'agent':
      return entry.reviewed ? 'chats' : 'agent';
    case 'auto':
    case 'correction':
      return 'chats';
    case 'import':
      return 'imported';
    case 'user':
      return 'user';
  }
}

/**
 * A case folded match of the written form, the spoken forms and the note, the
 * fields `GET /dictionary/entries?q=` searches.
 */
export function matchesQuery(entry: DictionaryEntryView, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  const fields = [entry.written, ...entry.spoken_forms, entry.note ?? ''];
  return fields.some((field) => field.toLocaleLowerCase().includes(needle));
}

// One collator each, made once. `localeCompare` with options builds a collator on every call,
// which at the 5,000 row cap was most of a sort's cost (about 60,000 comparisons).
const IGNORING_CASE = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true });
const EXACTLY = new Intl.Collator();

/** Alphabetical, ignoring case, so a row keeps its place when its usage changes. */
export function byWritten(a: DictionaryEntryView, b: DictionaryEntryView): number {
  return (
    IGNORING_CASE.compare(a.written, b.written) ||
    EXACTLY.compare(a.written, b.written) ||
    EXACTLY.compare(a.id, b.id)
  );
}

const ORDER: Exclude<GroupKey, 'matches'>[] = ['suggested', 'agent', 'user', 'chats', 'imported'];

/**
 * The groups, in the page's order: **Suggested** and **Added by the agent**
 * only when present, **Added by you** always (when the dictionary holds any
 * row), **From your chats** and **Imported** when present. A typed filter
 * folds every group into one **Matches** list, empty when nothing matches.
 */
export function groupEntries(entries: readonly DictionaryEntryView[], query = ''): EntryGroup[] {
  if (query.trim()) {
    const matches = entries.filter((entry) => matchesQuery(entry, query)).sort(byWritten);
    return [{ key: 'matches', title: DICTIONARY_LABELS.groups.matches, entries: matches }];
  }
  if (entries.length === 0) return [];
  const buckets = new Map<Exclude<GroupKey, 'matches'>, DictionaryEntryView[]>();
  for (const entry of entries) {
    const key = groupOf(entry);
    const bucket = buckets.get(key);
    if (bucket) bucket.push(entry);
    else buckets.set(key, [entry]);
  }
  return ORDER.filter((key) => key === 'user' || (buckets.get(key)?.length ?? 0) > 0).map(
    (key) => ({
      key,
      title: DICTIONARY_LABELS.groups[key],
      entries: (buckets.get(key) ?? []).sort(byWritten),
    })
  );
}

/** The rows **Review all…** offers: suggestions and the agent's rows not yet kept. */
export function reviewRows(entries: readonly DictionaryEntryView[]): DictionaryEntryView[] {
  return entries.filter((entry) => {
    const group = groupOf(entry);
    return group === 'suggested' || group === 'agent';
  });
}

/**
 * Where a private row is used, the start of its second line: MSG-ROW-PRIVATE
 * for no institution (`null`), MSG-ROW-PRIVATE-LOCAL for an unknown one (`[]`),
 * MSG-ROW-PRIVATE-INSTITUTION with the institution's published name (else its
 * id) for one, and its two or more form, the names joined with "and", for more.
 * `null` for a public row.
 */
export function privateLine(entry: DictionaryEntryView): string | null {
  if (entry.tier !== 'private') return null;
  const institutions = entry.affiliation?.institutions;
  if (!institutions) return dictionaryMessage('MSG-ROW-PRIVATE');
  if (institutions.length === 0) return dictionaryMessage('MSG-ROW-PRIVATE-LOCAL');
  const names = institutions.map((institution) => institution.name?.trim() || institution.id);
  if (names.length === 1) {
    return dictionaryMessage('MSG-ROW-PRIVATE-INSTITUTION', { institution: names[0] });
  }
  return fillMessage(ROW_PRIVATE_INSTITUTIONS, { institutions: joinInstitutions(names) });
}

/**
 * The second line: "Said as …" for the spoken forms, else the note, else where
 * the row came from; on a private row it starts with where the row is used,
 * then " · " and the rest.
 */
export function lineTwo(entry: DictionaryEntryView): string {
  const rest =
    entry.spoken_forms.length > 0
      ? DICTIONARY_LABELS.saidAs(entry.spoken_forms)
      : entry.note?.trim()
        ? entry.note.trim()
        : DICTIONARY_LABELS.sources[entry.source];
  const where = privateLine(entry);
  return where ? `${where} · ${rest}` : rest;
}

/**
 * The meta at rest: "12 uses" for an active row, present only when the answer
 * carries usage, which it does for a caller with private standing. A row never
 * used carries `uses: 0` and no `last_used_at` at all.
 */
export function usesMeta(entry: DictionaryEntryView): string | null {
  if (entry.status !== 'active' || typeof entry.uses !== 'number') return null;
  return entry.uses > 0 ? DICTIONARY_LABELS.uses(entry.uses) : DICTIONARY_LABELS.notUsed;
}

/**
 * The warning before **Make public…**: MSG-PUBLICIZE-WARN for a row that shows
 * it came from a chat, which the daemon answers as `came_from_a_chat`
 * (`Entry::came_from_a_chat`), and MSG-PUBLICIZE-WARN-NO-CHAT for every other
 * private row. The page never spells that rule again.
 */
export function publicizeWarning(entry: DictionaryEntryView): DictionaryMessageId {
  return entry.came_from_a_chat === true ? 'MSG-PUBLICIZE-WARN' : 'MSG-PUBLICIZE-WARN-NO-CHAT';
}

/** What a row the person just acted on says while it waits for the pointer to leave. */
export type SettledKind = keyof typeof DICTIONARY_LABELS.settled;
