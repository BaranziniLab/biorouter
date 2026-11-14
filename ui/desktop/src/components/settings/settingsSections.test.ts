import { describe, expect, it } from 'vitest';
import {
  SETTINGS_DEEP_LINKS,
  SETTINGS_SECTIONS,
  SETTINGS_SECTION_IDS,
  SETTINGS_TABS,
  resolveSettingsDeepLink,
} from './settingsSections';

describe('settings deep links', () => {
  /**
   * These keys have callers outside Settings (route state, the `?section=`
   * query from the main process). Dropping one silently strands its caller on
   * whatever tab was open, so the whole set is pinned.
   */
  it('keeps every key a caller already passes', () => {
    expect(Object.keys(SETTINGS_DEEP_LINKS).sort()).toEqual(
      ['app', 'chat', 'models', 'modes', 'privacy', 'styles', 'tools', 'update'].sort()
    );
  });

  it('lands each legacy key on the tab and section it names', () => {
    expect(resolveSettingsDeepLink('update')).toEqual({
      tab: 'app',
      sectionId: 'settings-about',
    });
    expect(resolveSettingsDeepLink('models')).toEqual({ tab: 'models' });
    expect(resolveSettingsDeepLink('modes')).toEqual({
      tab: 'chat',
      sectionId: 'settings-approvals',
    });
    expect(resolveSettingsDeepLink('styles')).toEqual({
      tab: 'chat',
      sectionId: 'settings-display',
    });
    expect(resolveSettingsDeepLink('tools')).toEqual({
      tab: 'chat',
      sectionId: 'settings-capabilities',
    });
    expect(resolveSettingsDeepLink('privacy')).toEqual({
      tab: 'app',
      sectionId: 'settings-privacy',
    });
    expect(resolveSettingsDeepLink('app')).toEqual({ tab: 'app' });
    expect(resolveSettingsDeepLink('chat')).toEqual({ tab: 'chat' });
  });

  it('accepts any section key as well', () => {
    expect(resolveSettingsDeepLink('memory')).toEqual({
      tab: 'chat',
      sectionId: 'settings-memory',
    });
    expect(resolveSettingsDeepLink('usage')).toEqual({ tab: 'app', sectionId: 'settings-usage' });
  });

  /**
   * DD-66: Speech models is a section of the Models tab, not a fourth tab, and the setup
   * popover's More in Settings (PR-19) lands on it by the section key `dictation`. A section key,
   * not a new deep link: the legacy set pinned above is unchanged.
   */
  it('lands `dictation` on the Speech models section of the Models tab, with no fourth tab', () => {
    expect(resolveSettingsDeepLink('dictation')).toEqual({
      tab: 'models',
      sectionId: 'settings-dictation',
    });
    expect(SETTINGS_TABS).toEqual(['models', 'chat', 'app']);
    expect(Object.prototype.hasOwnProperty.call(SETTINGS_DEEP_LINKS, 'dictation')).toBe(false);
    const modelsTab = Object.entries(SETTINGS_SECTIONS)
      .filter(([, section]) => section.tab === 'models')
      .map(([key]) => key);
    // Between the local model inventory and the danger zone, as the tab draws them.
    expect(modelsTab).toEqual(['defaultModel', 'localModels', 'dictation', 'modelsDangerZone']);
  });

  it('ignores unknown, empty and prototype keys', () => {
    expect(resolveSettingsDeepLink(undefined)).toBeNull();
    expect(resolveSettingsDeepLink('')).toBeNull();
    expect(resolveSettingsDeepLink('advanced')).toBeNull();
    expect(resolveSettingsDeepLink('toString')).toBeNull();
    expect(resolveSettingsDeepLink('__proto__')).toBeNull();
  });

  it('gives every section a unique id on a known tab', () => {
    const ids = Object.values(SETTINGS_SECTIONS).map((section) => section.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const section of Object.values(SETTINGS_SECTIONS)) {
      expect(SETTINGS_TABS).toContain(section.tab);
      expect(section.id).toMatch(/^settings-[a-z-]+$/);
    }
    expect(SETTINGS_SECTION_IDS.privacy).toBe('settings-privacy');
  });

  it('never points a deep link at a section on another tab', () => {
    for (const target of Object.values(SETTINGS_DEEP_LINKS)) {
      if (!target.sectionId) continue;
      const section = Object.values(SETTINGS_SECTIONS).find((s) => s.id === target.sectionId);
      expect(section?.tab).toBe(target.tab);
    }
  });
});
