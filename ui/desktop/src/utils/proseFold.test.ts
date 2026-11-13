import { describe, expect, it } from 'vitest';
import {
  fitsInfoTip,
  foldForHelp,
  foldProse,
  sentenceCount,
  splitFirstSentence,
} from './proseFold';

describe('splitFirstSentence', () => {
  it('cuts after the first sentence and keeps every word', () => {
    const text =
      "Biorouter's backend is restarting or unreachable. Your chat is safe. Retry in a moment.";
    const { first, rest } = splitFirstSentence(text);
    expect(first).toBe("Biorouter's backend is restarting or unreachable.");
    expect(rest).toBe('Your chat is safe. Retry in a moment.');
    expect(`${first} ${rest}`).toBe(text);
  });

  it('never cuts at a stop with no space after it', () => {
    expect(splitFirstSentence('Every question must reach P(yes) 0.80. 1 of 3 used.')).toEqual({
      first: 'Every question must reach P(yes) 0.80.',
      rest: '1 of 3 used.',
    });
    expect(splitFirstSentence('Recorded in /home/me/.config/biorouter/x.yaml.').rest).toBe('');
  });

  it('skips a stop that ends an abbreviation or a lowercase word', () => {
    expect(splitFirstSentence('Use e.g. the default model. Then retry.')).toEqual({
      first: 'Use e.g. the default model.',
      rest: 'Then retry.',
    });
  });

  it('takes a closing quote with its stop', () => {
    expect(splitFirstSentence('It said “No.” Then it stopped working.').first).toBe(
      'It said “No.”'
    );
  });

  it('returns one sentence whole', () => {
    expect(splitFirstSentence('  One sentence only.  ')).toEqual({
      first: 'One sentence only.',
      rest: '',
    });
  });
});

describe('foldProse', () => {
  it('leaves text that fits on one line whole', () => {
    expect(foldProse('Short. Two sentences.')).toEqual({ lead: 'Short. Two sentences.', rest: '' });
  });

  it('folds longer text after its first sentence', () => {
    const text = `${'A long first sentence that keeps going'.repeat(2)}. And the advice after it.`;
    expect(foldProse(text)).toEqual({
      lead: `${'A long first sentence that keeps going'.repeat(2)}.`,
      rest: 'And the advice after it.',
    });
  });

  it('never cuts a single long sentence', () => {
    const text = 'word '.repeat(40).trim();
    expect(foldProse(text)).toEqual({ lead: text, rest: '' });
  });
});

describe('sentenceCount and fitsInfoTip', () => {
  it('counts sentences', () => {
    expect(sentenceCount('One. Two words here. Three of them now.')).toBe(2);
    expect(sentenceCount('First sentence here. Second sentence here. Third one here.')).toBe(3);
    expect(sentenceCount('')).toBe(0);
  });

  it('admits two short sentences and refuses a third or a long run', () => {
    expect(fitsInfoTip('Smaller local models often do this. A larger model may help.')).toBe(true);
    expect(
      fitsInfoTip('First sentence here. Second sentence here. Third sentence here as well.')
    ).toBe(false);
    expect(fitsInfoTip('x'.repeat(161))).toBe(false);
  });
});

describe('foldForHelp', () => {
  it('puts a short rest behind the info icon and a long one in details', () => {
    const lead = 'Nothing ran: this reply is a call to shell written as text, not a tool call.';
    const short = foldForHelp(
      `${lead} Smaller local models often do this. A larger model may help.`
    );
    expect(short).toEqual({
      lead,
      tip: 'Smaller local models often do this. A larger model may help.',
      details: '',
    });
    const more =
      'First extra sentence here. Second extra sentence here. Third extra sentence here.';
    const long = foldForHelp(`${lead} ${more}`);
    expect(long.tip).toBe('');
    expect(long.details).toBe(more);
  });

  it('folds nothing that fits one line', () => {
    expect(foldForHelp('Folder approval was refused.')).toEqual({
      lead: 'Folder approval was refused.',
      tip: '',
      details: '',
    });
  });
});
