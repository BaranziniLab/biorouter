import { describe, expect, it } from 'vitest';
import { folderName } from './historyRow';
import { messageCount } from './copy';

describe('folderName', () => {
  it('names the last folder of a path', () => {
    expect(folderName('/Users/wgu/data')).toBe('data');
    expect(folderName('/Users/wgu/data/')).toBe('data');
    expect(folderName('C:\\Users\\wgu\\proj')).toBe('proj');
  });

  it('reads the root as itself and nothing as nothing', () => {
    expect(folderName('/')).toBe('/');
    expect(folderName('')).toBe('');
    expect(folderName(undefined)).toBe('');
  });
});

describe('messageCount', () => {
  it('agrees in number', () => {
    expect(messageCount(1)).toBe('1 message');
    expect(messageCount(0)).toBe('0 messages');
    expect(messageCount(12)).toBe('12 messages');
  });
});
