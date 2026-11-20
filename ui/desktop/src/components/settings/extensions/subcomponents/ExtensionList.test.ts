import { describe, expect, it } from 'vitest';
import { formatExtensionName } from './ExtensionList';

describe('formatExtensionName', () => {
  it('formats ordinary extension identifiers', () => {
    expect(formatExtensionName('code_execution')).toBe('Code Execution');
    expect(formatExtensionName('auto-visualiser')).toBe('Auto Visualiser');
  });

  it('uses the proper display name for Chat Recall', () => {
    expect(formatExtensionName('chatrecall')).toBe('Chat Recall');
    expect(formatExtensionName('Chat Recall')).toBe('Chat Recall');
  });
});
