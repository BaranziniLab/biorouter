import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  DICTATION_ACTIONS,
  DICTATION_LABELS,
  DICTATION_MESSAGES,
  FN_USAGE_ACTIONS,
  INSERT_REFUSAL_MESSAGES,
  SETUP_SUGGEST_LAST_SENTENCE,
  SETUP_WHERE_THIS_COMPUTER,
  WHERE_THIS_COMPUTER,
  dictationMessage,
  fillMessage,
  formatDictationClock,
  insertRefusalMessage,
  setupWhereOnServer,
  whereOnServer,
} from './copy';

const repo = resolve(__dirname, '../../../../..');
const catalogPath = resolve(repo, 'docs/dictation/surfaces.md');

/** Split a Markdown table row, honouring `\|` inside a cell. */
function cells(line: string): string[] {
  const out: string[] = [];
  let current = '';
  for (let i = 0; i < line.length; i++) {
    if (line[i] === '\\' && line[i + 1] === '|') {
      current += '|';
      i++;
    } else if (line[i] === '|') {
      out.push(current.trim());
      current = '';
    } else {
      current += line[i];
    }
  }
  out.push(current.trim());
  return out.slice(1, -1);
}

/**
 * The catalog writes the buttons a sentence carries, and notes on where it is
 * shown, in a trailing parenthesis: "(button **Retry**)", "(where is …)",
 * "(shown under …)". The renderer keeps those as labels, not as text.
 */
function withoutNote(text: string): string {
  if (!text.endsWith(')')) return text;
  let depth = 0;
  for (let i = text.length - 1; i >= 0; i--) {
    if (text[i] === ')') depth++;
    else if (text[i] === '(') {
      depth--;
      if (depth === 0) {
        const note = text.slice(i + 1, -1);
        const isNote =
          text[i - 1] === ' ' &&
          (note.includes('**') || note.startsWith('where is') || note.startsWith('shown under'));
        return isNote ? text.slice(0, i - 1) : text;
      }
    }
  }
  return text;
}

function catalog(): Map<string, string> {
  const rows = new Map<string, string>();
  for (const line of readFileSync(catalogPath, 'utf8').split('\n')) {
    if (!line.startsWith('| MSG-')) continue;
    const [id, text] = cells(line);
    rows.set(id, withoutNote(text.replace(/\\</g, '<')));
  }
  return rows;
}

describe('dictation copy', () => {
  it('matches the message catalog in docs/dictation/surfaces.md word for word', () => {
    const rows = catalog();
    for (const [id, text] of Object.entries(DICTATION_MESSAGES)) {
      expect(rows.has(id), `${id} is not in the catalog`).toBe(true);
      expect(text, id).toBe(rows.get(id));
    }
  });

  it('carries every renderer sentence the capture and transport modules show', () => {
    for (const id of [
      'MSG-INSECURE',
      'MSG-MIC-DENIED-MAC',
      'MSG-MIC-DENIED-WIN',
      'MSG-MIC-RESTRICTED',
      'MSG-MIC-GRANTED-FAILED',
      'MSG-MIC-BROWSER',
      'MSG-NO-DEVICE',
      'MSG-MIC-BUSY',
      'MSG-413',
      'MSG-WAIT',
      'MSG-SERVER-BUSY',
      'MSG-SERVER-BUSY-FAILED',
      'MSG-RECORDING-FAILED',
      'MSG-LOST',
      'MSG-ENGINE-FAILED',
      'MSG-CAP-WARN',
      'MSG-CAP-STOPPED',
      'MSG-TOO-SHORT',
      'MSG-SILENT',
      'MSG-NO-AUDIO-YET',
      'MSG-NOTHING',
      'MSG-REMOTE-OLD',
    ]) {
      expect(DICTATION_MESSAGES).toHaveProperty(id);
    }
  });

  it('carries every sentence typing the text where the person is shows (DD-97)', () => {
    for (const id of [
      'MSG-INSERT-LOST',
      'MSG-INSERT-NO-PERMISSION',
      'MSG-INSERT-SECURE',
      'MSG-INSERT-TARGET-CHANGED',
      'MSG-INSERT-FAILED',
      'MSG-INSERT-HELP',
      'MSG-INSERT-PERMISSION',
    ]) {
      expect(DICTATION_MESSAGES).toHaveProperty(id);
    }
    // The buttons the catalog gives these sentences are labels here, not part of the text.
    const raw = readFileSync(catalogPath, 'utf8');
    expect(raw).toContain(
      `| MSG-INSERT-LOST | ${DICTATION_MESSAGES['MSG-INSERT-LOST']} (buttons **${DICTATION_ACTIONS.insertHere}**, **${DICTATION_ACTIONS.copy}**) |`
    );
    expect(raw).toContain(
      `${DICTATION_MESSAGES['MSG-INSERT-PERMISSION']} (button **${DICTATION_ACTIONS.openAccessibility}**) |`
    );
  });

  it('names a notification for every reason the insertion helper refuses, filled with the paste key', () => {
    expect(Object.keys(INSERT_REFUSAL_MESSAGES).sort()).toEqual(
      ['failed', 'no-permission', 'secure-input', 'target-changed'].sort()
    );
    for (const reason of Object.keys(INSERT_REFUSAL_MESSAGES) as Array<
      keyof typeof INSERT_REFUSAL_MESSAGES
    >) {
      const sentence = insertRefusalMessage(reason, '⌘V');
      expect(sentence, reason).not.toMatch(/<[^>]+>/);
      expect(sentence, reason).toBe(
        dictationMessage(INSERT_REFUSAL_MESSAGES[reason], { key: '⌘V' })
      );
    }
    expect(insertRefusalMessage('target-changed', '⌘V')).toBe(
      'You switched apps before the text was ready, so Biorouter copied it. Paste with ⌘V.'
    );
  });

  it('warns that macOS acts on fn for a double-tap as well as a hold, for every fn action', () => {
    for (const action of Object.values(FN_USAGE_ACTIONS)) {
      const sentence = dictationMessage('MSG-FN-CONFLICT', { action });
      expect(sentence).toContain(`macOS also uses fn to ${action}, so a hold or a double-tap`);
      expect(sentence).not.toMatch(/<[^>]+>/);
    }
    expect(DICTATION_MESSAGES['MSG-FN-CONFLICT-UNKNOWN']).toContain('a double-tap can do it twice');
  });

  it('keeps the disclosures true now that the text is kept in the dictation history (DD-96)', () => {
    expect(DICTATION_MESSAGES['MSG-DISCLOSE-HOST-DETAIL']).toContain(
      'does not save your voice or send it anywhere else'
    );
    expect(DICTATION_MESSAGES['MSG-DISCLOSE-HOST-DETAIL']).toContain('dictation history');
    expect(DICTATION_MESSAGES['MSG-CLIPBOARD-EXPLAIN']).toContain('dictation history');
    expect(DICTATION_MESSAGES['MSG-CLIPBOARD-EXPLAIN']).toContain(
      'Biorouter puts back what was there'
    );
  });

  it('keeps the two other endings of MSG-SETUP-SUGGEST as the catalog words them', () => {
    const row = readFileSync(catalogPath, 'utf8')
      .split('\n')
      .find((line) => line.startsWith('| MSG-SETUP-SUGGEST'))!;
    expect(
      DICTATION_MESSAGES['MSG-SETUP-SUGGEST'].endsWith(SETUP_SUGGEST_LAST_SENTENCE.on_request)
    ).toBe(true);
    expect(row).toContain(`"${SETUP_SUGGEST_LAST_SENTENCE.suggest}"`);
    expect(row).toContain(`"${SETUP_SUGGEST_LAST_SENTENCE.add}"`);
  });

  it('writes plainly: no dashes and none of the words the style guide rules out', () => {
    const banned =
      /\b(seamless|robust|leverage|delve|crucial|comprehensive|empower|streamline|elevate|unlock|harness|journey|landscape|realm|utilize|facilitate|effortless|intuitive|powerful|furthermore|moreover|additionally|simply|ensure|recipe)\b/i;
    const all = [
      ...Object.values(DICTATION_MESSAGES),
      ...Object.values(DICTATION_ACTIONS),
      ...Object.values(DICTATION_LABELS),
      ...Object.values(SETUP_SUGGEST_LAST_SENTENCE),
    ];
    for (const text of all) {
      expect(text, text).not.toMatch(/[–—]/);
      expect(text, text).not.toMatch(banned);
    }
  });

  it('fills placeholders and leaves unfilled ones as written', () => {
    expect(dictationMessage('MSG-SERVER-BUSY', { n: '4' })).toBe(
      'The server is busy transcribing for other people. Trying again in 4 seconds.'
    );
    expect(fillMessage('run --from <file> into <folder>', { folder: '/models' })).toBe(
      'run --from <file> into /models'
    );
    expect(
      dictationMessage('MSG-ENGINE-FAILED', { where: 'the Biorouter server at lab.example' })
    ).toContain('Transcription failed on the Biorouter server at lab.example, so');
    expect(
      dictationMessage('MSG-REMOTE-OLD', { "this app's version": '1.95.0', host: 'srv' })
    ).toBe(
      'The Biorouter service on srv is a version without local dictation. Update it to 1.95.0 or later, or turn off Use external server in Settings > App.'
    );
  });

  it('fills <where> with the form each sentence takes', () => {
    // MSG-ENGINE-FAILED already says "on" before <where>.
    expect(dictationMessage('MSG-ENGINE-FAILED', { where: WHERE_THIS_COMPUTER })).toBe(
      'Transcription failed on this computer, so no text was produced. Your recording is kept for 5 minutes.'
    );
    expect(dictationMessage('MSG-ENGINE-FAILED', { where: whereOnServer('') })).toContain(
      'failed on the Biorouter server, so'
    );
    // MSG-SETUP does not, so its <where> carries it.
    const setup = (where: string) =>
      dictationMessage('MSG-SETUP', {
        where,
        name: 'Light',
        size: '264 MB',
        folder: '~/models',
      });
    expect(setup(SETUP_WHERE_THIS_COMPUTER)).toMatch(
      /^Dictation turns your speech into text on this computer\. It needs/
    );
    expect(setup(setupWhereOnServer('lab.example:8765'))).toMatch(
      /^Dictation turns your speech into text on the Biorouter server at lab\.example:8765\. It needs/
    );
    expect(setup(SETUP_WHERE_THIS_COMPUTER)).not.toMatch(/<[^>]+>/);
  });

  it('formats the recording timer as m:ss', () => {
    expect(formatDictationClock(0)).toBe('0:00');
    expect(formatDictationClock(7.9)).toBe('0:07');
    expect(formatDictationClock(65)).toBe('1:05');
    expect(formatDictationClock(600)).toBe('10:00');
    expect(formatDictationClock(-3)).toBe('0:00');
  });
});
