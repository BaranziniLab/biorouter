/**
 * Settings sections a deep link can land on, and the keys callers pass.
 *
 * This is the WS-SETTINGS-A wave-0 contract (codex simplicity redesign, spec
 * §3.13 and §3.19). Every section listed here renders its root with
 * `id={SETTINGS_SECTION_IDS.<key>}`, normally through
 * `<SettingSection id={SETTINGS_SECTION_IDS.<key>} …>`. `SettingsView`
 * resolves `viewOptions.section` with {@link resolveSettingsDeepLink}, selects
 * the tab, then scrolls the section into view and highlights it.
 *
 * Owners of the sections that live outside WS-SETTINGS-A's files:
 * - `defaultModel`, `localModels`, `modelsDangerZone`: WS-SETTINGS-M
 *   (`settings/models/**`, `settings/reset_provider/**`).
 * - `memory`, `configuration`: WS-SETTINGS-B (`settings/memory/**`,
 *   `settings/config/**`).
 * - `usage`: WS-USAGE (`settings/usage/**`).
 * Every other id is rendered by WS-SETTINGS-A (`SettingsView`,
 * `settings/{app,chat,mode,response_styles,privacy}/**`).
 *
 * ⚠ The deep-link KEYS in {@link SETTINGS_DEEP_LINKS} are an API: callers pass
 * them in route state or the `?section=` query (`PrivacyTiersOffNote.tsx`,
 * `ConfigureProvidersRoute.tsx`, `IngestModelPicker.tsx`, Crew's
 * `AgentTaskPane.tsx`, the main process's `set-view` for `update` and
 * `models`). Never rename or drop one; add a key instead.
 */

export type SettingsTab = 'models' | 'chat' | 'app';

/** Tab order, left to right. The names are pinned by daemon copy (§3.13). */
export const SETTINGS_TABS: readonly SettingsTab[] = ['models', 'chat', 'app'];

/** Each section's DOM id and the tab that holds it. */
export const SETTINGS_SECTIONS = {
  // Models tab
  defaultModel: { id: 'settings-default-model', tab: 'models' },
  localModels: { id: 'settings-local-models', tab: 'models' },
  modelsDangerZone: { id: 'settings-models-danger-zone', tab: 'models' },
  // Chat tab
  approvals: { id: 'settings-approvals', tab: 'chat' },
  display: { id: 'settings-display', tab: 'chat' },
  capabilities: { id: 'settings-capabilities', tab: 'chat' },
  memory: { id: 'settings-memory', tab: 'chat' },
  contexts: { id: 'settings-contexts', tab: 'chat' },
  appSdk: { id: 'settings-app-sdk', tab: 'chat' },
  project: { id: 'settings-project', tab: 'chat' },
  // App tab
  configuration: { id: 'settings-configuration', tab: 'app' },
  privacy: { id: 'settings-privacy', tab: 'app' },
  general: { id: 'settings-general', tab: 'app' },
  appearance: { id: 'settings-appearance', tab: 'app' },
  usage: { id: 'settings-usage', tab: 'app' },
  about: { id: 'settings-about', tab: 'app' },
  appDangerZone: { id: 'settings-app-danger-zone', tab: 'app' },
} as const satisfies Record<string, { id: string; tab: SettingsTab }>;

export type SettingsSectionKey = keyof typeof SETTINGS_SECTIONS;

/** `SETTINGS_SECTION_IDS.privacy === 'settings-privacy'`, and so on. */
export const SETTINGS_SECTION_IDS = Object.fromEntries(
  Object.entries(SETTINGS_SECTIONS).map(([key, section]) => [key, section.id])
) as { readonly [K in SettingsSectionKey]: (typeof SETTINGS_SECTIONS)[K]['id'] };

export type SettingsSectionId = (typeof SETTINGS_SECTIONS)[SettingsSectionKey]['id'];

export interface SettingsDeepLinkTarget {
  tab: SettingsTab;
  /** Absent when the link lands on the top of the tab. */
  sectionId?: SettingsSectionId;
}

/**
 * The keys callers already pass. `modes` lands on Approvals (it used to be
 * called Mode) and `styles` on Display (it used to be Response styles).
 * `tools` predates both and lands on Capabilities, the built-in tools.
 */
export const SETTINGS_DEEP_LINKS: Readonly<Record<string, SettingsDeepLinkTarget>> = {
  update: { tab: 'app', sectionId: SETTINGS_SECTIONS.about.id },
  models: { tab: 'models' },
  modes: { tab: 'chat', sectionId: SETTINGS_SECTIONS.approvals.id },
  styles: { tab: 'chat', sectionId: SETTINGS_SECTIONS.display.id },
  tools: { tab: 'chat', sectionId: SETTINGS_SECTIONS.capabilities.id },
  app: { tab: 'app' },
  chat: { tab: 'chat' },
  // Privacy used to be a tab of its own. It is a section of App now, so an
  // old link still lands somewhere that exists.
  privacy: { tab: 'app', sectionId: SETTINGS_SECTIONS.privacy.id },
};

/**
 * Resolve a `section` key to the tab and section it names. A legacy key wins;
 * otherwise any {@link SettingsSectionKey} (`memory`, `usage`, …) works too.
 * Unknown keys return null, so the view stays on its current tab.
 */
export function resolveSettingsDeepLink(
  section: string | null | undefined
): SettingsDeepLinkTarget | null {
  if (!section) return null;
  if (Object.prototype.hasOwnProperty.call(SETTINGS_DEEP_LINKS, section)) {
    return SETTINGS_DEEP_LINKS[section];
  }
  if (Object.prototype.hasOwnProperty.call(SETTINGS_SECTIONS, section)) {
    const target = SETTINGS_SECTIONS[section as SettingsSectionKey];
    return { tab: target.tab, sectionId: target.id };
  }
  return null;
}
