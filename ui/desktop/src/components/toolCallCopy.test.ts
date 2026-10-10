import { describe, expect, it } from 'vitest';
import { toolRowLabel } from './toolCallCopy';

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
