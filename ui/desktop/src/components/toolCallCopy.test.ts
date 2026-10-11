import { describe, expect, it } from 'vitest';
import {
  toolActionPhrase,
  toolDisplayName,
  toolNamePhrase,
  toolNameWords,
  toolRowLabel,
} from './toolCallCopy';

describe('toolRowLabel: the verb carries the state', () => {
  it.each([
    ['running', 'Reading package.json'],
    ['done', 'Read package.json'],
    ['failed', 'Failed to read package.json'],
    ['stopped', 'Stopped reading package.json'],
  ] as const)('conjugates a gerund summary while %s', (state, expected) => {
    expect(toolRowLabel('Reading package.json', state)).toBe(expected);
  });

  it('reads a shell row the way Codex does', () => {
    expect(toolRowLabel('Running npm test', 'running')).toBe('Running npm test');
    expect(toolRowLabel('Running npm test', 'done')).toBe('Ran npm test');
    expect(toolRowLabel('Running npm test', 'failed')).toBe('Failed to run npm test');
  });

  it('keeps everything after the verb byte for byte', () => {
    expect(toolRowLabel('Marking “Draft #2” complete', 'done')).toBe('Marked “Draft #2” complete');
    expect(toolRowLabel('Delegating: read the logs', 'done')).toBe('Delegated: read the logs');
    expect(toolRowLabel('Searching skills for rna-seq', 'failed')).toBe(
      'Failed to search skills for rna-seq'
    );
  });

  it('never prefixes a verb it cannot conjugate', () => {
    // A plan step's own words, or a tool's display name.
    expect(toolRowLabel('Read the manifest → List the files', 'done')).toBe(
      'Read the manifest → List the files'
    );
    expect(toolRowLabel('Lookup · Id: 7', 'running')).toBe('Lookup · Id: 7');
    expect(toolRowLabel('Lookup · Id: 7', 'failed')).toBe('Failed: Lookup · Id: 7');
    expect(toolRowLabel('Lookup · Id: 7', 'stopped')).toBe('Stopped: Lookup · Id: 7');
  });

  it('does not mistake a capitalized noun for a verb', () => {
    expect(toolRowLabel('Readings from the sensor', 'done')).toBe('Readings from the sensor');
  });
});

describe('tool names read in sentence case (V2-04)', () => {
  it('splits every identifier style into the same words', () => {
    expect(toolNameWords('installMarketplaceSkill')).toEqual(['install', 'marketplace', 'skill']);
    expect(toolNameWords('agent_drafter/build-app')).toEqual(['agent', 'drafter', 'build', 'app']);
    expect(toolNameWords('files.read')).toEqual(['files', 'read']);
  });

  it('keeps acronyms and product names in capitals and nothing else', () => {
    expect(toolNamePhrase('text_editor')).toBe('text editor');
    expect(toolNamePhrase('ucsf_omop_query')).toBe('UCSF OMOP query');
    expect(toolNamePhrase('agent_drafter_build_app')).toBe('Agent Drafter build app');
    expect(toolNamePhrase('autovisualiser_kb_sync')).toBe('Auto Visualiser KB sync');
    expect(toolDisplayName('developer__text_editor')).toBe('Text editor');
  });

  it.each([
    [['ingest', 'conversation'], 'Ingesting conversation'],
    [['cohort', 'lookup'], 'Looking up cohort'],
    [['workspace', 'watch'], 'Watching workspace'],
    [['get', 'page'], 'Getting page'],
    [['authenticate'], 'Authenticating'],
    // A noun that is also a verb only counts as the first word.
    [['smoke', 'test'], 'Running smoke test'],
    [['spreadsheet', 'summary'], 'Running spreadsheet summary'],
    [[], 'Running a tool'],
  ])('names the action of %j', (words, expected) => {
    expect(toolActionPhrase(words)).toBe(expected);
  });

  it('writes only gerunds that conjugate', () => {
    for (const words of [
      ['ingest', 'x'],
      ['cohort', 'lookup'],
      ['set', 'x'],
      ['show', 'x'],
    ]) {
      const phrase = toolActionPhrase(words);
      expect(toolRowLabel(phrase, 'done')).not.toBe(phrase);
      expect(toolRowLabel(phrase, 'failed')).toMatch(/^Failed to [a-z]/);
    }
  });
});
