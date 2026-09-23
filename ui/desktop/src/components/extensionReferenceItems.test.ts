import { describe, expect, it } from 'vitest';
import { extensionReferenceItems } from './extensionReferenceItems';
import { getMentionInsertText, mentionReference } from './MentionPopover';
import { findRefTags } from '../utils/resourceRefs';
import { CAPABILITIES } from './settings/capabilities/capabilities';

describe('extension command catalog', () => {
  it('uses current capability labels with stable backend identities', () => {
    const items = extensionReferenceItems([]);
    expect(items).toHaveLength(CAPABILITIES.length);
    for (const capability of CAPABILITIES) {
      const item = items.find((entry) => entry.name === `ext:${capability.label}`)!;
      expect(mentionReference(item)).toMatchObject({
        kind: 'extension',
        value: capability.key,
        label: capability.label,
      });
      expect(findRefTags(getMentionInsertText(item))[0].value).toBe(capability.key);
    }
    expect(items.some((item) => item.name === 'ext:computercontroller')).toBe(false);
  });

  it('deduplicates active builtins and platform capabilities without losing custom names', () => {
    const items = extensionReferenceItems([
      { type: 'builtin', name: 'computercontroller', description: 'Old description' },
      { type: 'platform', name: 'Chat Recall', description: '' },
      { type: 'platform', name: 'Extension Manager', description: '' },
      {
        type: 'stdio',
        name: 'my_tool-v2',
        description: 'Current session tool',
        cmd: 'fixture',
        args: [],
      },
    ]);
    expect(items).toHaveLength(CAPABILITIES.length + 1);
    expect(items.find((item) => item.name === 'ext:my_tool-v2')).toMatchObject({
      relativePath: 'my_tool-v2',
      builtIn: false,
    });
    expect(items.filter((item) => item.relativePath === 'computercontroller')).toHaveLength(1);
  });

  it('does not offer custom names that resolve to a different bundled target', () => {
    const items = extensionReferenceItems([
      { type: 'stdio', name: 'Developer', description: 'Custom', cmd: 'fixture', args: [] },
      { type: 'stdio', name: 'Biorouter Copilot', description: 'Custom', cmd: 'fixture', args: [] },
      { type: 'stdio', name: 'Workspace Control', description: 'Custom', cmd: 'fixture', args: [] },
      { type: 'stdio', name: 'Biorouter.COPILOT', description: 'Custom', cmd: 'fixture', args: [] },
      { type: 'stdio', name: 'Web & Documents', description: 'Custom', cmd: 'fixture', args: [] },
    ]);
    expect(items).toHaveLength(CAPABILITIES.length);
    expect(items.every((item) => item.builtIn)).toBe(true);
  });
});
