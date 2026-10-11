import { describe, expect, it } from 'vitest';
import { extensionDisplayName, formatExtensionName } from './extensionDisplayName';
import { getFriendlyTitle } from './subcomponents/ExtensionList';
import type { BaamRegistry } from '../../baam/registry';
import type { FixedExtensionEntry } from '../../ConfigContext';

/**
 * Principle 10: one extension, one name, on every surface. These pin the order the resolver
 * asks its sources in, because each rung is the fix for a surface that used to disagree:
 * the Extensions page said Cdwagent, the composer said CDWAgent.
 */
describe('extensionDisplayName', () => {
  it('names a key from the marketplace catalogue, spelling and capitals included', () => {
    expect(extensionDisplayName('cdwagent')).toBe('CDWAgent');
    expect(extensionDisplayName('spokeagent')).toBe('SPOKEAgent');
    expect(extensionDisplayName('codegraphagent')).toBe('CodeGraph Agent');
    expect(extensionDisplayName('playwrightagent')).toBe('Playwright Agent');
  });

  it('names a bundled capability from its own catalogue, not by its key', () => {
    expect(extensionDisplayName('computercontroller')).toBe('Biorouter Copilot');
    expect(extensionDisplayName('webdocuments')).toBe('Web & Documents');
    expect(extensionDisplayName('autovisualiser')).toBe('Auto Visualiser');
    // The underscore and the space spell one key.
    expect(extensionDisplayName('agent_drafter')).toBe('Agent Drafter');
    expect(extensionDisplayName('Agent Drafter')).toBe('Agent Drafter');
  });

  it('keeps the platform names', () => {
    expect(extensionDisplayName('chatrecall')).toBe('Chat Recall');
    expect(extensionDisplayName('Chat Recall')).toBe('Chat Recall');
  });

  /**
   * The daemon sends the workspace capability as `Workspace`; the daemon's own display name and
   * Settings > Chat > Capabilities call it Workspace Control, so a load toast, a History row and
   * a permission list must too (the request from WS-SETTINGS-A, vision V4-R2-3).
   */
  it('names the workspace capability Workspace Control, however the daemon spells it', () => {
    expect(extensionDisplayName('Workspace')).toBe('Workspace Control');
    expect(extensionDisplayName('workspace')).toBe('Workspace Control');
    expect(extensionDisplayName({ name: 'Workspace', type: 'platform' })).toBe('Workspace Control');
    expect(formatExtensionName('Workspace')).toBe('Workspace Control');
  });

  it('does not give a platform name to an extension the user installed', () => {
    // `workspace` is a likely name for somebody's own MCP server. It is theirs, not the capability.
    expect(extensionDisplayName({ name: 'workspace', type: 'stdio' })).toBe('Workspace');
    expect(extensionDisplayName({ name: 'chatrecall', type: 'streamable_http' })).toBe(
      'Chatrecall'
    );
  });

  it('falls back to the key read as words only when nothing knows it', () => {
    expect(extensionDisplayName('code_execution')).toBe('Code Execution');
    expect(extensionDisplayName('my-lab_tool')).toBe('My Lab Tool');
  });

  /**
   * V3-R2-6. A chat records the extensions it ran with, display names included, so one recorded
   * before the rename still said `Computer Controller` in History's tooltip while the Extensions
   * page, Settings and the pickers said `Biorouter Copilot`.
   */
  it('names a bundled entry from the catalogue, not from the name a chat stored for it', () => {
    expect(
      extensionDisplayName({
        name: 'computercontroller',
        type: 'builtin',
        display_name: 'Computer Controller',
      })
    ).toBe('Biorouter Copilot');
    expect(
      extensionDisplayName({ name: 'computercontroller', type: 'builtin', display_name: 'Desk' })
    ).toBe('Biorouter Copilot');
    // The catalogue knows it by id or by name, and a space or an underscore is the same key.
    expect(
      extensionDisplayName({
        name: 'Computer Controller',
        type: 'builtin',
        display_name: 'Computer Controller',
      })
    ).toBe('Biorouter Copilot');
  });

  it('keeps the stored name of a bundled entry the catalogue does not know', () => {
    expect(
      extensionDisplayName({ name: 'lab_notes', type: 'builtin', display_name: 'Lab notes' })
    ).toBe('Lab notes');
    // With no stored name either, the key is read as words, as before.
    expect(extensionDisplayName({ name: 'lab_notes', type: 'builtin' })).toBe('Lab Notes');
  });

  it('does not trust a display name on an installed entry to rename the marketplace’s', () => {
    // A display name on an installed entry is not a bundled capability's, so it is not trusted
    // to rename the marketplace's.
    expect(
      extensionDisplayName({ name: 'cdwagent', type: 'stdio', display_name: 'Something else' })
    ).toBe('CDWAgent');
  });

  it('never turns an installed extension into a bundled one, whatever it is called', () => {
    // A stdio entry that happens to be called `webdocuments` is the user's own, and is named as such.
    expect(extensionDisplayName({ name: 'webdocuments', type: 'stdio' })).toBe('Webdocuments');
    // An entry that says nothing about its type (a bare key) may be either.
    expect(extensionDisplayName({ name: 'webdocuments' })).toBe('Web & Documents');
  });

  it('does not ask the marketplace about a bundled entry', () => {
    const registry = {
      version: 2,
      source: 'test',
      extensions: [{ id: 'developer', name: 'Not The Developer Extension' }],
      skills: [],
    } as unknown as BaamRegistry;
    expect(extensionDisplayName({ name: 'developer', type: 'builtin' }, registry)).toBe(
      'Developer'
    );
  });

  it('prefers a catalogue the caller holds over the bundled snapshot', () => {
    const registry = {
      version: 2,
      source: 'test',
      extensions: [
        {
          id: 'cdwagent',
          name: 'CDW Agent',
          extension_name: 'cdwagent',
        },
      ],
      skills: [],
    } as unknown as BaamRegistry;
    expect(extensionDisplayName('cdwagent', registry)).toBe('CDW Agent');
  });

  it('is what the Extensions rows are titled with', () => {
    const installed = { name: 'cdwagent', type: 'stdio', enabled: true } as FixedExtensionEntry;
    expect(getFriendlyTitle(installed)).toBe('CDWAgent');
    const bundled = {
      name: 'computercontroller',
      type: 'builtin',
      display_name: 'Biorouter Copilot',
      enabled: true,
    } as FixedExtensionEntry;
    expect(getFriendlyTitle(bundled)).toBe('Biorouter Copilot');
  });
});

describe('formatExtensionName', () => {
  it('stays the same function the Extensions list exports', async () => {
    const list = await import('./subcomponents/ExtensionList');
    expect(list.formatExtensionName).toBe(formatExtensionName);
  });
});
