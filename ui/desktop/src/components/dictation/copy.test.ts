import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { fitsInfoTip } from '../../utils/proseFold';
import {
  DICTATION_ACTIONS,
  DICTATION_LABELS,
  DICTATION_MESSAGES,
  FN_KEY_LABELS,
  FN_MODIFIER_LABELS,
  FN_USAGE_ACTIONS,
  INSERT_REFUSAL_MESSAGES,
  INSERT_RESTORE_MESSAGES,
  SETUP_SUGGEST_LAST_SENTENCE,
  SETUP_WHERE_THIS_COMPUTER,
  WHERE_THIS_COMPUTER,
  dictationMessage,
  fillMessage,
  formatDictationClock,
  insertRefusalMessage,
  insertRestoreMessage,
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

  // DD-101 (2026-11-27): a modifier plus fn does the same job when another app also uses fn.
  it('carries every sentence of the alternative key, in copy.ts and in the catalog alike', () => {
    const rows = catalog();
    for (const id of [
      'MSG-FN-KEY-HELP',
      'MSG-FN-KEY-AUTO',
      'MSG-FN-KEY-SWITCHED',
      'MSG-FN-KEY-SHARED',
      'MSG-FN-MODIFIER-HELP',
      'MSG-FN-USE-ALT-COLLISION',
      'MSG-FN-USE-ALT-CHOSEN',
    ]) {
      expect(DICTATION_MESSAGES, id).toHaveProperty(id);
      expect(rows.has(id), `${id} is not in the catalog`).toBe(true);
    }
    // Filled with the default key, Option+fn since the owner's ruling O-49 (2026-11-27); these
    // read Control+fn while O-46 made Control the default. Control+fn is still filled below.
    const fill = { alt: 'Option+fn', app: 'Wispr Flow' };
    expect(dictationMessage('MSG-FN-KEY-AUTO', fill)).toBe('Using fn. Option+fn works too.');
    expect(dictationMessage('MSG-FN-KEY-SWITCHED', fill)).toBe(
      'Using Option+fn, because Wispr Flow also uses fn.'
    );
    expect(dictationMessage('MSG-FN-KEY-SHARED', fill)).toBe(
      'Wispr Flow also uses fn, so one press may start both.'
    );
    expect(dictationMessage('MSG-FN-KEY-HELP', { alt: 'Option+fn' })).toBe(
      'Automatic uses fn, or Option+fn while another app that uses fn is open, and Option+fn always works too. Choose fn or Option+fn to use one key only.'
    );
    expect(dictationMessage('MSG-FN-KEY-AUTO', { alt: 'Control+fn' })).toBe(
      'Using fn. Control+fn works too.'
    );
    expect(DICTATION_MESSAGES['MSG-FN-MODIFIER-HELP']).toBe(
      'Command is left out because macOS uses it for shortcuts.'
    );
    expect(dictationMessage('MSG-FN-USE-ALT-COLLISION', fill)).toBe(
      'Wispr Flow also uses fn. Hold Option+fn to dictate in Biorouter, or double-tap it for hands-free.'
    );
    expect(dictationMessage('MSG-FN-USE-ALT-CHOSEN', { alt: 'Shift+fn' })).toBe(
      'Biorouter dictates with Shift+fn. Hold it to talk, or double-tap it for hands-free.'
    );
    // The one-time key notice links to Settings, as the one-time conflict note does.
    const raw = readFileSync(catalogPath, 'utf8');
    for (const id of ['MSG-FN-USE-ALT-COLLISION', 'MSG-FN-USE-ALT-CHOSEN'] as const) {
      expect(raw).toContain(
        `| ${id} | ${DICTATION_MESSAGES[id].replace(/</g, '\\<')} (button **${DICTATION_ACTIONS.moreInSettings}**) |`
      );
    }
  });

  it('names the key that works now in the fn help and the other apps help', () => {
    for (const key of Object.values(FN_KEY_LABELS).filter((label) => label !== 'Automatic')) {
      for (const id of ['MSG-FN-HELP', 'MSG-FN-OTHER-APPS'] as const) {
        const sentence = dictationMessage(id, { key });
        expect(sentence, id).not.toMatch(/<[^>]+>/);
        expect(sentence, id).toContain(key);
      }
      expect(fillMessage(DICTATION_LABELS.fnGestureWith, { key })).toBe(
        `hold or double-tap ${key}`
      );
    }
    // The label that always said fn is gone, so no tooltip can name a key that does not dictate.
    expect(DICTATION_LABELS).not.toHaveProperty('fnGesture');
    expect(dictationMessage('MSG-FN-HELP', { key: 'Control+fn' })).toBe(
      'Hold Control+fn to talk and let go to insert. Double-tap Control+fn to dictate hands-free, and double-tap again to stop.'
    );
    expect(dictationMessage('MSG-FN-OTHER-APPS', { key: 'fn' })).toBe(
      'Hold or double-tap fn in any app while Biorouter is open. The text goes into that app, or to the clipboard when Biorouter cannot type there.'
    );
    // Still true with protocol version 3, which also says which modifier is held with fn.
    expect(DICTATION_MESSAGES['MSG-FN-PERMISSION']).toContain(
      'Biorouter keeps only whether fn is held and which of Shift, Control or Option is held with it'
    );
  });

  // design.md 9.16: an InfoTip holds two sentences at most, about 160 characters, filled with
  // the longest key name it can carry.
  it('keeps every fn and insertion InfoTip within an InfoTip, whichever key it names', () => {
    const keys = Object.entries(FN_KEY_LABELS)
      .filter(([value]) => value !== 'automatic')
      .map(([, label]) => label);
    for (const key of keys) {
      for (const id of [
        'MSG-FN-HELP',
        'MSG-FN-OTHER-APPS',
        'MSG-FN-KEY-HELP',
        'MSG-FN-MODIFIER-HELP',
        'MSG-INSERT-HELP',
      ] as const) {
        const tip = dictationMessage(id, { key, alt: key });
        expect(fitsInfoTip(tip), `${id} with ${key}: ${tip.length} characters`).toBe(true);
      }
    }
  });

  it('labels the two fn key rows, their options and the tray item, with Command never offered', () => {
    expect(DICTATION_ACTIONS.dictationKey).toBe('Dictation key');
    expect(DICTATION_ACTIONS.keyHeldWithFn).toBe('Key held with fn');
    expect(DICTATION_ACTIONS.copyLastDictation).toBe('Copy last dictation');
    expect(FN_KEY_LABELS).toEqual({
      automatic: 'Automatic',
      fn: 'fn',
      control: 'Control+fn',
      option: 'Option+fn',
      shift: 'Shift+fn',
    });
    expect(FN_MODIFIER_LABELS).toEqual({ control: 'Control', option: 'Option', shift: 'Shift' });
    expect(JSON.stringify({ FN_KEY_LABELS, FN_MODIFIER_LABELS })).not.toMatch(/command|⌘/i);
  });

  // DD-102 (2026-11-27): what the insertion helper says it did with the clipboard after it typed.
  it('names a notification for each clipboard restore that did not put everything back', () => {
    const rows = catalog();
    for (const id of [
      'MSG-INSERT-CLIPBOARD-PARTIAL',
      'MSG-INSERT-CLIPBOARD-KEPT',
      'MSG-INSERT-PASSWORD-CLEARED',
    ]) {
      expect(DICTATION_MESSAGES, id).toHaveProperty(id);
      expect(rows.has(id), `${id} is not in the catalog`).toBe(true);
    }
    expect(INSERT_RESTORE_MESSAGES).toEqual({
      partial: 'MSG-INSERT-CLIPBOARD-PARTIAL',
      'kept-text': 'MSG-INSERT-CLIPBOARD-KEPT',
      cleared: 'MSG-INSERT-PASSWORD-CLEARED',
    });
    expect(insertRestoreMessage('all')).toBeNull();
    expect(insertRestoreMessage('moved')).toBeNull();
    expect(insertRestoreMessage(undefined)).toBeNull();
    expect(insertRestoreMessage('partial')).toBe(
      'Typed into the app, but Biorouter could not put back everything that was on your clipboard.'
    );
    expect(insertRestoreMessage('kept-text')).toBe(
      'Typed into the app. The text is still on your clipboard, because Biorouter could not put back what was there.'
    );
    expect(insertRestoreMessage('cleared')).toBe(
      'Typed into the app. Your clipboard held a password, so Biorouter cleared it instead of putting it back.'
    );
    expect(DICTATION_MESSAGES['MSG-INSERT-HELP']).toBe(
      'Biorouter pastes the text into the app you are using, then puts back what was on your clipboard. A copied password is cleared, and password fields are skipped.'
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
      ...Object.values(FN_KEY_LABELS),
      ...Object.values(FN_MODIFIER_LABELS),
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
