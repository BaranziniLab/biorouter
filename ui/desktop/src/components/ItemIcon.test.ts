import { describe, expect, it } from 'vitest';
import { Package, SquareSlash, Wrench } from './icons/app-icons';
import { ENTITY_ICONS } from './icons/entity-icons';
import { getItemIcon } from './ItemIcon';
import type { DisplayItem } from './MentionPopover';

const item = (itemType: DisplayItem['itemType'], name = 'x'): DisplayItem => ({
  name,
  extra: '',
  itemType,
  relativePath: name,
});

/** The mention popover draws the same glyph for an entity as every other
 * surface does (design.md §3.9: one glyph, one meaning). */
describe('getItemIcon', () => {
  it('draws each entity with its entity glyph', () => {
    expect(getItemIcon(item('Workflow')).Icon).toBe(ENTITY_ICONS.workflow);
    expect(getItemIcon(item('KnowledgeBase')).Icon).toBe(ENTITY_ICONS.knowledge);
    expect(getItemIcon(item('Skill')).Icon).toBe(ENTITY_ICONS.skill);
    expect(getItemIcon(item('Extension')).Icon).toBe(ENTITY_ICONS.extension);
    expect(getItemIcon(item('Directory')).Icon).toBe(ENTITY_ICONS.folder);
  });

  /** A built-in entry is a slash command, not a bolt (the bolt belongs to the
   * Skill glyph now) and not a shell. */
  it('draws a built-in command as the slash-command mark', () => {
    expect(getItemIcon(item('Builtin', '/compact')).Icon).toBe(SquareSlash);
  });

  /** The wrench means an unknown tool call; an installer is a package. */
  it('draws an installer as a package, never the wrench', () => {
    for (const name of ['setup.exe', 'Biorouter.dmg', 'biorouter_1.0_amd64.deb']) {
      expect(getItemIcon(item('File', name)).Icon, name).toBe(Package);
      expect(getItemIcon(item('File', name)).Icon, name).not.toBe(Wrench);
    }
  });

  it('keeps every glyph monochrome', () => {
    for (const type of ['Builtin', 'Workflow', 'Directory', 'File'] as const) {
      expect(getItemIcon(item(type, 'a.py')).color).toBe('currentColor');
    }
  });
});
