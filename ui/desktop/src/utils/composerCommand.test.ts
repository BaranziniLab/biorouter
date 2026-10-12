import { describe, expect, it } from 'vitest';
import {
  CHIP_COMMANDS,
  chipCommandOfInsert,
  composerTextOfSent,
  isChipCommand,
  joinLeadingCommand,
  splitLeadingCommand,
  splitSentCommand,
} from './composerCommand';
import { joinComposerText, splitComposerText } from './composerRefs';
import { refTag } from './resourceRefs';

// Bodies chosen to hit every edge of the claim: nothing, whitespace at the end,
// the bare command, a longer word that starts with it, the wrong case, the
// command twice, and the command somewhere other than the start.
const BODIES = [
  '',
  ' ',
  'hi',
  'hi ',
  'ends in a newline\n',
  '/bug',
  '/bug ',
  '/bug  ',
  '/bug x',
  '/bug x ',
  '/bug x\n',
  '/bug\nx',
  '/bug\tx',
  '/bugs',
  '/bugs x',
  '/Bug x',
  '/BUG x',
  '/bug /bug x',
  'x /bug y',
  ' /bug x',
];

describe('splitLeadingCommand / joinLeadingCommand', () => {
  // The property the composer rests on (see the header of composerRefs.ts):
  // whatever the user typed comes back character for character, or React
  // reassigns the textarea and the caret jumps.
  it('round-trips every body exactly', () => {
    for (const body of BODIES) {
      const { command, prose } = splitLeadingCommand(body);
      expect(joinLeadingCommand(command, prose), JSON.stringify(body)).toBe(body);
    }
  });

  it('is stable: splitting the joined form gives the same split', () => {
    for (const body of BODIES) {
      const once = splitLeadingCommand(body);
      expect(splitLeadingCommand(joinLeadingCommand(once.command, once.prose))).toEqual(once);
    }
  });

  it('claims only a lowercase /bug followed by a space at the very start', () => {
    expect(splitLeadingCommand('/bug x')).toEqual({ command: 'bug', prose: 'x' });
    expect(splitLeadingCommand('/bug ')).toEqual({ command: 'bug', prose: '' });
    // One space is the separator; a second one is prose the user typed.
    expect(splitLeadingCommand('/bug  x')).toEqual({ command: 'bug', prose: ' x' });
    expect(splitLeadingCommand('/bug x\n')).toEqual({ command: 'bug', prose: 'x\n' });

    for (const body of ['', '/bug', '/bugs', '/bugs x', '/Bug x', '/BUG x', 'x /bug y']) {
      expect(splitLeadingCommand(body), JSON.stringify(body)).toEqual({
        command: null,
        prose: body,
      });
    }
    // Only the space finishes the command word in the composer: a newline or a
    // tab right after it stays text the user can see and edit.
    expect(splitLeadingCommand('/bug\nx').command).toBeNull();
    expect(splitLeadingCommand('/bug\tx').command).toBeNull();
    expect(splitLeadingCommand(' /bug x').command).toBeNull();
  });

  it('claims the command once: a second /bug is prose', () => {
    expect(splitLeadingCommand('/bug /bug x')).toEqual({ command: 'bug', prose: '/bug x' });
  });

  it('writes nothing for no command', () => {
    expect(joinLeadingCommand(null, 'hello')).toBe('hello');
    expect(joinLeadingCommand('bug', '')).toBe('/bug ');
    expect(joinLeadingCommand('bug', 'the chart is blank')).toBe('/bug the chart is blank');
  });

  // References ride at the end of the message and are taken off before the
  // command is read, so the two splits compose without either disturbing the other.
  it('composes with references', () => {
    for (const body of BODIES) {
      const text = joinComposerText(body, [
        { kind: 'skill', value: 'my skill', label: undefined, start: 0, end: 0, raw: '' },
      ]);
      const split = splitComposerText(text);
      expect(split.body, JSON.stringify(body)).toBe(body);
      expect(split.refs).toHaveLength(1);
    }
    const { body } = splitComposerText(`/bug  ${refTag('skill', 'my skill')}`);
    expect(splitLeadingCommand(body)).toEqual({ command: 'bug', prose: '' });
  });
});

describe('chipCommandOfInsert', () => {
  it('recognises exactly the insert a picked row makes', () => {
    expect(chipCommandOfInsert('/bug ')).toBe('bug');
    expect(chipCommandOfInsert('/bug')).toBeNull();
    expect(chipCommandOfInsert('/bug x')).toBeNull();
    expect(chipCommandOfInsert('/compact')).toBeNull();
    expect(chipCommandOfInsert(refTag('skill', 'bug'))).toBeNull();
  });
});

describe('isChipCommand', () => {
  it('names the registry, case-sensitively as the daemon does', () => {
    expect(isChipCommand('bug')).toBe(true);
    expect(isChipCommand('Bug')).toBe(false);
    expect(isChipCommand('/bug')).toBe(false);
    expect(isChipCommand('compact')).toBe(false);
    expect(isChipCommand('toString')).toBe(false);
  });

  it('gives every chip command a label and a placeholder', () => {
    for (const def of Object.values(CHIP_COMMANDS)) {
      expect(def.label.trim()).not.toBe('');
      expect(def.placeholder.trim()).not.toBe('');
      expect(def.label).not.toMatch(/[—–]/);
    }
  });
});

describe('splitSentCommand', () => {
  it('claims the command a sent message starts with, as the daemon splits it', () => {
    expect(splitSentCommand('/bug')).toEqual({ command: 'bug', rest: '' });
    expect(splitSentCommand('/bug x')).toEqual({ command: 'bug', rest: ' x' });
    expect(splitSentCommand('/bug\nx')).toEqual({ command: 'bug', rest: '\nx' });
    expect(splitSentCommand('/bug\tx')).toEqual({ command: 'bug', rest: '\tx' });
  });

  it('leaves everything else as text', () => {
    for (const text of ['', 'hi', '/bugs', '/bugs x', '/Bug x', 'x /bug y', ' /bug x']) {
      expect(splitSentCommand(text), JSON.stringify(text)).toEqual({ command: null, rest: text });
    }
  });

  it('keeps every character after the command word', () => {
    for (const text of BODIES) {
      const { command, rest } = splitSentCommand(text);
      expect((command ? `/${command}` : '') + rest).toBe(text);
    }
  });
});

describe('composerTextOfSent', () => {
  it('gives a bare sent /bug its space back, so the composer draws the chip', () => {
    expect(composerTextOfSent('/bug')).toBe('/bug ');
    expect(splitLeadingCommand(composerTextOfSent('/bug'))).toEqual({ command: 'bug', prose: '' });
  });

  it('leaves every other text exactly as it was', () => {
    for (const text of BODIES.filter((body) => body !== '/bug')) {
      expect(composerTextOfSent(text), JSON.stringify(text)).toBe(text);
    }
    // Merged behind typed text, a bare `/bug` is no longer one.
    expect(composerTextOfSent('/bug\n\ntyped')).toBe('/bug\n\ntyped');
  });
});
