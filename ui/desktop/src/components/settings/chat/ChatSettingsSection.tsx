import { ContextsSection } from '../contexts/ContextsSection';
import { ModeSection } from '../mode/ModeSection';
import { ResponseStylesSection } from '../response_styles/ResponseStylesSection';
import { CapabilitiesSection } from '../capabilities/CapabilitiesSection';
import { BrsdkSection } from '../brsdk/BrsdkSection';
import { BioRouterHintsSection } from './BioRouterHintsSection';
import { SpellcheckToggle } from './SpellcheckToggle';
import MemorySection from '../memory/MemorySection';

export default function ChatSettingsSection() {
  return (
    <div className="pb-8">
      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted mb-1">Mode</h2>
          <p className="text-supporting text-text-muted">
            Configure how Biorouter interacts with tools and extensions
          </p>
        </div>
        {/* No `.biorouter-settings-list` wrapper here: `ModeSection` IS the
            list, because `role="radiogroup"` has to sit on the element that
            contains the radios. Every other section below contributes a
            fragment of rows to the list this file provides — they have no
            semantics of their own to declare, and rows that are direct children
            are what make `:last-child` select the real last row. */}
        <ModeSection />
      </div>

      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted mb-1">Response styles</h2>
          <p className="text-supporting text-text-muted">
            Choose how Biorouter should format and style its responses
          </p>
        </div>
        <div className="biorouter-settings-list">
          <ResponseStylesSection />
        </div>
      </div>

      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted mb-1">Capabilities</h2>
          <p className="text-supporting text-text-muted">
            Choose which built-in abilities new chats start with. Existing chats keep their current
            capabilities.
          </p>
        </div>
        <div className="biorouter-settings-list">
          <CapabilitiesSection />
        </div>
      </div>

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
      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted mb-1">Contexts</h2>
          <p className="text-supporting text-text-muted">
            Skills that ship with Biorouter. They load into every chat unless you turn one off.
          </p>
        </div>
        <div className="biorouter-settings-list">
          <ContextsSection />
        </div>
      </div>

      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted mb-1">App SDK</h2>
          <p className="text-supporting text-text-muted">
            Opt-in safety frameworks for Agent-Drafter apps. All are off by default and apply only
            to Agent-Drafter apps, never to normal chat.
          </p>
        </div>
        <div className="biorouter-settings-list">
          <BrsdkSection />
        </div>
      </div>

      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted">Editor</h2>
        </div>
        <div className="biorouter-settings-list">
          <SpellcheckToggle />
        </div>
      </div>

      <div className="biorouter-settings-section">
        <div className="biorouter-settings-section-header">
          <h2 className="text-caps text-text-muted">Project</h2>
        </div>
        <div className="biorouter-settings-list">
          <BioRouterHintsSection />
        </div>
      </div>
    </div>
  );
}
