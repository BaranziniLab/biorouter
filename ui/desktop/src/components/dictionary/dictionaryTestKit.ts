/**
 * Rows and settings answers shaped as PR-16's routes answer them, for the
 * Dictionary page's tests. A caller with private standing gets the usage
 * figures, and a row never used has `uses: 0` and no `last_used_at` at all.
 */
import type { DictionaryEntryView, DictionarySettings } from '../../api';

let next = 0;

export function entry(overrides: Partial<DictionaryEntryView> = {}): DictionaryEntryView {
  next += 1;
  return {
    id: `dct_${String(next).padStart(8, '0')}-0000-4000-8000-000000000000`,
    rev: 1,
    written: `Term ${next}`,
    tier: 'public',
    status: 'active',
    source: 'user',
    reviewed: true,
    spoken_forms: [],
    misrecognitions: [],
    kind: null,
    match: 'fuzzy',
    bias: 'auto',
    context_cues: [],
    note: null,
    reason: null,
    provenance: null,
    affiliation: null,
    created_at: '2026-11-01T09:00:00Z',
    updated_at: '2026-11-04T17:02:11Z',
    uses: 0,
    corrected_to: 0,
    ...overrides,
  };
}

export function settings(overrides: Partial<DictionarySettings> = {}): DictionarySettings {
  const allowed = { allowed: true, reason: null };
  return {
    suggest_from_chats: false,
    learn_from_chats: false,
    agent_writes: 'on_request',
    authority: {
      write: allowed,
      delete: allowed,
      add_private: allowed,
      tier_change: allowed,
      raise_settings: allowed,
    },
    ...overrides,
  };
}

/** A generated client answer: `{data, response}` for a 2xx, `{error, response}` otherwise. */
export function answer(status: number, body: unknown, headers: Record<string, string> = {}) {
  const response = new Response(status === 204 || status === 304 ? null : JSON.stringify(body), {
    status,
    headers,
  });
  return response.ok ? { data: body, response } : { error: body, response };
}

/** A listing page answered as text (`parseAs: 'text'`), as the store reads it. */
export function listing(entries: DictionaryEntryView[], next_cursor: string | null = null) {
  const text = JSON.stringify({ entries, next_cursor });
  return { data: text, response: new Response(text, { status: 200 }) };
}
