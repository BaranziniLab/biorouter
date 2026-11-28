import { describe, expect, it } from 'vitest';
import { groupByChatDate } from '../../../utils/chatDateBuckets';
import { DICTIONARY_MESSAGES } from '../copy';
import { linesInView } from '../rowWindow';
import {
  expandedLine,
  groupDictationsByDate,
  historyPolicyOf,
  historyPublicizeWarning,
  metaLine,
  surfaceWord,
  tierChangeReason,
  turnOnGate,
} from './historyRows';
import { dictation, historySettings } from './historyTestKit';

/** Noon on Thursday 26 November 2026, local time. */
const NOW = new Date(2026, 10, 26, 12, 0, 0).getTime();
const DAY = 86_400_000;

describe('the date buckets', () => {
  const at = (msAgo: number) => dictation({ now: NOW, minutesAgo: msAgo / 60_000 });

  it('uses the chat lists’ own buckets and words, newest first', () => {
    const entries = [at(60_000), at(DAY), at(3 * DAY), at(20 * DAY), at(60 * DAY), at(400 * DAY)];
    const groups = groupDictationsByDate(entries, NOW);
    expect(groups.map((group) => group.title)).toEqual([
      'Today',
      'Yesterday',
      'Previous 7 days',
      'Previous 30 days',
      'September',
      'October 2025',
    ]);
  });

  it('groups exactly as the shared helper does, asking once per day', () => {
    const entries = Array.from({ length: 400 }, (_, index) =>
      at((index * 31 * 3_600_000 + index * 7_919) % (500 * DAY))
    ).sort((a, b) => b.created_at.localeCompare(a.created_at));
    const ours = groupDictationsByDate(entries, NOW);
    const shared = groupByChatDate(entries, (entry) => entry.created_at, NOW);
    expect(ours.map((group) => [group.key, group.title, group.entries.map((e) => e.id)])).toEqual(
      shared.map((group) => [group.key, group.label, group.items.map((e) => e.id)])
    );
  });

  it('keeps every entry’s order inside a bucket', () => {
    const entries = [at(60_000), at(120_000), at(180_000)];
    expect(groupDictationsByDate(entries, NOW)[0].entries).toEqual(entries);
  });
});

describe('a row’s lines', () => {
  it('reads the time, where it went and how long', () => {
    const entry = dictation({
      now: NOW,
      minutesAgo: 141,
      surface: 'composer',
      duration_ms: 14_200,
    });
    expect(metaLine(entry, NOW)).toBe('9:39 AM · Chat · 14 s');
  });

  it('names every surface in the history’s words', () => {
    expect(
      (['composer', 'crew', 'clipboard', 'other_app', 'app_preview', 'terminal'] as const).map(
        surfaceWord
      )
    ).toEqual(['Chat', 'Crew', 'Clipboard', 'Another app', 'App preview', 'Terminal']);
  });

  it('names the tier, the full time and the model only when expanded', () => {
    const entry = dictation({
      created_at: '2026-11-26T15:39:12Z',
      tier: 'private',
      surface: 'composer',
      duration_ms: 14_000,
      model_name: 'Large v3 Turbo',
    });
    const line = expandedLine(entry);
    expect(line.startsWith('Private · Nov 26, 2026, ')).toBe(true);
    expect(line.endsWith(' · Chat · 14 s · Large v3 Turbo')).toBe(true);
    expect(expandedLine({ ...entry, tier: 'public', model_name: null })).toMatch(
      /^Public · .* · whisper-large-v3-turbo-q5_0$/
    );
  });
});

describe('the refusals a row shows before the click', () => {
  it('asks the stronger warning when private terms were used', () => {
    expect(historyPublicizeWarning(dictation({ private_terms: true }))).toBe(
      'MSG-HISTORY-PUBLICIZE-WARN-TERMS'
    );
    expect(historyPublicizeWarning(dictation({ private_terms: false }))).toBe(
      'MSG-HISTORY-PUBLICIZE-WARN'
    );
    expect(historyPublicizeWarning(dictation({ private_terms: null }))).toBe(
      'MSG-HISTORY-PUBLICIZE-WARN'
    );
  });

  it('fills the host sentence’s verb and id for the change this row would make', () => {
    const hostReason = DICTIONARY_MESSAGES['MSG-HISTORY-HOST-TIER'].replace('<verb>', 'publicize');
    const privateOne = dictation({ tier: 'private' });
    const publicOne = dictation({ tier: 'public' });
    expect(tierChangeReason(hostReason, privateOne)).toBe(
      `Do this on the server: \`biorouter dictation history publicize ${privateOne.id}\` (as the service user).`
    );
    expect(tierChangeReason(hostReason, publicOne)).toContain(`privatize ${publicOne.id}`);
    expect(tierChangeReason('Your organization manages this.', publicOne)).toBe(
      'Your organization manages this.'
    );
    expect(tierChangeReason(null, publicOne)).toBeNull();
  });

  it('lets Turn on be pressed only where turning the history on is admitted', () => {
    expect(turnOnGate(null)).toEqual({ disabled: true, reason: null });
    expect(turnOnGate(historySettings({ keep_history: false }))).toEqual({
      disabled: false,
      reason: null,
    });
    const refused = historySettings({
      keep_history: false,
      authority: {
        ...historySettings().authority,
        raise_settings: {
          allowed: false,
          reason: DICTIONARY_MESSAGES['MSG-HISTORY-HOST-SETTINGS'],
        },
      },
    });
    expect(turnOnGate(refused)).toEqual({
      disabled: true,
      reason:
        'Do this on the server: `biorouter dictation history settings --keep on` (as the service user).',
    });
  });

  it('reads managed history from the status, and nothing from a daemon that sends none', () => {
    expect(historyPolicyOf(null)).toBeNull();
    expect(historyPolicyOf({ policy: {} })).toBeNull();
    expect(
      historyPolicyOf({ policy: { history: { allowed: false, reason: 'Managed.' } } })
    ).toEqual({ allowed: false, reason: 'Managed.' });
  });
});

describe('the line arithmetic with an expanded row', () => {
  // One group: its heading is line 0, its first row line 1, rows on a 50px pitch from y = 20.
  const boxes = [{ rowsLine: 1, rows: 100, top: 0, rowsTop: 20 }];
  const tall = [{ line: 3, extra: 500 }];
  const lineAt = (y: number, extra = tall) => linesInView(boxes, y, y + 1, 50, extra)!.first;

  it('finds the expanded row over all of its height, and every row below it shifted by its extra', () => {
    expect(lineAt(20 + 2 * 50 + 10)).toBe(3);
    expect(lineAt(20 + 2 * 50 + 540)).toBe(3);
    expect(lineAt(20 + 3 * 50 + 500 + 1)).toBe(4);
    expect(lineAt(20 + 50 * 50 + 500 + 1)).toBe(51);
    // Without it, the same point reads ten lines too far down.
    expect(lineAt(20 + 50 * 50 + 500 + 1, [])).toBe(61);
  });

  it('leaves the rows above an expanded row as they were', () => {
    expect(lineAt(20 + 49)).toBe(1);
    expect(lineAt(20 + 50)).toBe(2);
  });
});
