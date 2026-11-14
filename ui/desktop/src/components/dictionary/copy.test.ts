import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  DICTIONARY_LABELS,
  DICTIONARY_MESSAGES,
  ROW_PRIVATE_INSTITUTIONS,
  dictionaryMessage,
  joinInstitutions,
  validationSentence,
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
 * The catalog keeps a sentence's buttons, and notes on where it shows, in a
 * trailing parenthesis ("(buttons **Make public**, **Cancel**)", "(a row that
 * names two or more institutions reads …)"); the renderer keeps those as labels
 * or as a second form, not as part of the text.
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
          text[i - 1] === ' ' && (note.includes('**') || note.startsWith('a row that names'));
        return isNote ? text.slice(0, i - 1) : text;
      }
    }
  }
  return text;
}

function catalogRows(): Map<string, { text: string; raw: string }> {
  const rows = new Map<string, { text: string; raw: string }>();
  for (const line of readFileSync(catalogPath, 'utf8').split('\n')) {
    if (!line.startsWith('| MSG-')) continue;
    const [id, text] = cells(line);
    rows.set(id, { text: withoutNote(text.replace(/\\</g, '<')), raw: line });
  }
  return rows;
}

describe('Dictionary page copy', () => {
  it('matches the message catalog in docs/dictation/surfaces.md word for word', () => {
    const rows = catalogRows();
    for (const [id, text] of Object.entries(DICTIONARY_MESSAGES)) {
      expect(rows.has(id), `${id} is not in the catalog`).toBe(true);
      expect(text, id).toBe(rows.get(id)!.text);
    }
  });

  it('keeps the two or more institutions form of MSG-ROW-PRIVATE-INSTITUTION as the catalog words it', () => {
    const row = catalogRows().get('MSG-ROW-PRIVATE-INSTITUTION')!.raw.replace(/\\</g, '<');
    expect(row).toContain(`"${ROW_PRIVATE_INSTITUTIONS}"`);
  });

  it('carries every sentence the page and its dialogs show', () => {
    for (const id of [
      'MSG-PUBLICIZE-WARN',
      'MSG-PUBLICIZE-WARN-NO-CHAT',
      'MSG-ROW-PRIVATE',
      'MSG-ROW-PRIVATE-INSTITUTION',
      'MSG-ROW-PRIVATE-LOCAL',
      'MSG-EXPORT-PRIVATE',
      'MSG-EMPTY',
    ]) {
      expect(DICTIONARY_MESSAGES).toHaveProperty(id);
    }
  });

  it('gives every validation code the store answers its catalog sentence', () => {
    const store = readFileSync(
      resolve(repo, 'crates/biorouter-dictation/src/store/validate.rs'),
      'utf8'
    );
    const codes = [...store.matchAll(/Self::\w+ => "([a-z_]+)"/g)].map((match) => match[1]);
    expect(codes.length).toBeGreaterThan(10);
    for (const code of [...codes, 'cap', 'unreadable']) {
      expect(validationSentence(code), code).not.toBeNull();
    }
    expect(validationSentence('empty')).toBe(DICTIONARY_MESSAGES['MSG-TERM-PLAIN']);
    expect(validationSentence('something_new')).toBeNull();
  });

  it('joins institutions the catalog way', () => {
    expect(joinInstitutions(['UCSF'])).toBe('UCSF');
    expect(joinInstitutions(['UCSF', 'Stanford'])).toBe('UCSF and Stanford');
    expect(joinInstitutions(['A', 'B', 'C'])).toBe('A, B and C');
  });

  it('fills placeholders and leaves an unfilled one as written', () => {
    expect(dictionaryMessage('MSG-IMPORT-PLAN', { new: '12', existing: '3', skipped: '1' })).toBe(
      '12 new · 3 already in your dictionary · 1 skipped'
    );
    expect(dictionaryMessage('MSG-DELETE-TERM')).toContain('<term>');
  });

  it('writes plainly: no dashes and none of the words the style guide rules out', () => {
    const banned =
      /\b(seamless|robust|leverage|delve|crucial|comprehensive|empower|streamline|elevate|unlock|harness|journey|landscape|realm|utilize|facilitate|effortless|intuitive|powerful|furthermore|moreover|additionally|simply|ensure|recipe)\b/i;
    const labels: string[] = [];
    const walk = (value: unknown) => {
      if (typeof value === 'string') labels.push(value);
      else if (typeof value === 'function') {
        for (const argument of [2, ['epas one', 'e pass one'], 'EPAS1']) {
          try {
            labels.push(
              String((value as (arg: unknown, other?: unknown) => string)(argument, 'X'))
            );
            break;
          } catch {
            // The next argument shape.
          }
        }
      } else if (value && typeof value === 'object') Object.values(value).forEach(walk);
    };
    walk(DICTIONARY_LABELS);
    for (const text of [
      ...Object.values(DICTIONARY_MESSAGES),
      ROW_PRIVATE_INSTITUTIONS,
      ...labels,
    ]) {
      expect(text, text).not.toMatch(/[–—]/);
      expect(text, text).not.toMatch(banned);
    }
  });
});
