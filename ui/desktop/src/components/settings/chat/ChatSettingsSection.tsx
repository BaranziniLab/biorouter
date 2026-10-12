import { ContextsSection } from '../contexts/ContextsSection';
import { ModeSection } from '../mode/ModeSection';
import { ToolCallDetailsRow } from '../response_styles/ResponseStylesSection';
import { CapabilitiesSection } from '../capabilities/CapabilitiesSection';
import { BrsdkSection } from '../brsdk/BrsdkSection';
import { ProjectHintsRow } from './BioRouterHintsSection';
import { SpellcheckRow } from './SpellcheckToggle';
import MemorySection from '../memory/MemorySection';
import { SettingSection } from '../../ui/setting-row';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { chatSectionsCopy, displayCopy, projectCopy } from './copy';

/**
 * Settings > Chat (spec §3.13): Approvals, Display, Capabilities, Memory, Contexts, App SDK,
 * Project. One `SettingSection` each; a section's explanation is its header's InfoTip, never a
 * paragraph under it.
 *
 * Capabilities, Contexts and App SDK rows belong to WS-SETTINGS-B's components, which return
 * fragments of `SettingRow`s; this file owns their section headers (and the deep-link ids).
 * Memory renders its own section.
 */
export default function ChatSettingsSection() {
  return (
    <div className="pb-8">
      <ModeSection />

      <SettingSection id={SETTINGS_SECTION_IDS.display} title={displayCopy.section}>
        <ToolCallDetailsRow />
        <SpellcheckRow />
      </SettingSection>

      <SettingSection
        id={SETTINGS_SECTION_IDS.capabilities}
        title={chatSectionsCopy.capabilities}
        help={chatSectionsCopy.capabilitiesHelp}
      >
        <CapabilitiesSection />
      </SettingSection>

      {/* Directly under Capabilities, which owns the switch that turns memory
          on and off: the store and its own toggle belong together. */}
      <MemorySection />

      {/*
        ⚠ Below Memory, not above it. The brief asked for "directly beneath
        Capabilities and directly above App SDK", and those two are not the same
        slot: MemorySection already sits between them behind a load-bearing
        comment pinning it under Capabilities, because Capabilities owns the
        switch that turns memory on and off. Splitting that pair to satisfy the
        letter of the request would break the reason it exists.
      */}
      <SettingSection
        id={SETTINGS_SECTION_IDS.contexts}
        title={chatSectionsCopy.contexts}
        help={chatSectionsCopy.contextsHelp}
      >
        <ContextsSection />
      </SettingSection>

      <SettingSection
        id={SETTINGS_SECTION_IDS.appSdk}
        title={chatSectionsCopy.appSdk}
        help={chatSectionsCopy.appSdkHelp}
      >
        <BrsdkSection />
      </SettingSection>

      <SettingSection id={SETTINGS_SECTION_IDS.project} title={projectCopy.section}>
        <ProjectHintsRow />
      </SettingSection>
    </div>
  );
}
