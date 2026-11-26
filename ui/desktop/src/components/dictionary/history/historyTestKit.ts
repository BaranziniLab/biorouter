/**
 * Dictations and history answers shaped as `routes/dictation_history.rs` answers them, for the
 * **History** tab's tests.
 */
import type {
  DictationHistoryEntryView,
  DictationHistorySettings,
  DictationHistorySurface,
} from '../../../api';

let next = 0;

/** A dictation `minutesAgo` minutes before `now` (whole seconds, as the store keeps them). */
export function dictation(
  overrides: Partial<DictationHistoryEntryView> & { minutesAgo?: number; now?: number } = {}
): DictationHistoryEntryView {
  next += 1;
  const { minutesAgo = next, now = Date.now(), ...rest } = overrides;
  const created = new Date(Math.floor((now - minutesAgo * 60_000) / 1000) * 1000).toISOString();
  return {
    id: `dch_${String(next).padStart(8, '0')}-0000-4000-8000-000000000000`,
    created_at: created.replace('.000Z', 'Z'),
    text: `Dictation ${next}`,
    tier: 'private',
    surface: 'composer' as DictationHistorySurface,
    duration_ms: 14_200,
    model: 'whisper-large-v3-turbo-q5_0',
    model_name: 'Large v3 Turbo',
    private_terms: false,
    ...rest,
  };
}

export function historySettings(
  overrides: Partial<DictationHistorySettings> = {}
): DictationHistorySettings {
  const allowed = { allowed: true, reason: null };
  return {
    keep_history: true,
    public_by_default: false,
    authority: {
      list_private: allowed,
      write: allowed,
      tier_change: allowed,
      raise_settings: allowed,
    },
    ...overrides,
  };
}

/** A listing page as the store reads it (`parseAs: 'text'`). */
export function historyPage(
  entries: DictationHistoryEntryView[],
  options: { total?: number; next_cursor?: string | null } = {}
) {
  const text = JSON.stringify({
    entries,
    next_cursor: options.next_cursor ?? null,
    total: options.total ?? entries.length,
  });
  return { data: text, response: new Response(text, { status: 200 }) };
}

/** A generated client answer: `{data, response}` for a 2xx, `{error, response}` otherwise. */
export function historyAnswer(status: number, body: unknown) {
  const response = new Response(status === 204 || status === 304 ? null : JSON.stringify(body), {
    status,
  });
  return response.ok ? { data: body, response } : { error: body, response };
}
