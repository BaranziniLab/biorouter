import { useEffect, useRef, useState } from 'react';
import { ScrollArea } from '../ui/scroll-area';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../ui/tabs';
import { View, ViewOptions } from '../../utils/navigationUtils';
import ModelsSection from './models/ModelsSection';
import AppSettingsSection from './app/AppSettingsSection';
import ConfigSettings from './config/ConfigSettings';
import { ExtensionConfig } from '../../api';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import ChatSettingsSection from './chat/ChatSettingsSection';
import PrivacyPanel from './privacy/PrivacyPanel';
import { CONFIGURATION_ENABLED } from '../../updates';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader } from '../Layout/PageHeader';
import { prefersReducedMotion } from '../../styles/motion';
import { settingsShellCopy } from './app/copy';
import { resolveSettingsDeepLink, type SettingsTab } from './settingsSections';
import './settings.css';

export type SettingsViewOptions = {
  deepLinkConfig?: ExtensionConfig;
  showEnvVars?: boolean;
  section?: string;
};

/** How long a deep-linked section keeps its highlight (the `.br-highlight` wash's own length). */
const HIGHLIGHT_MS = 1600;
/** How long to wait for a deep-linked section that loads asynchronously before giving up. */
const FIND_SECTION_MS = 2000;

const TABS: SettingsTab[] = ['models', 'chat', 'app'];

function isSettingsTab(value: string): value is SettingsTab {
  return (TABS as string[]).includes(value);
}

export default function SettingsView({
  onClose,
  setView,
  viewOptions,
}: {
  onClose: () => void;
  setView: (view: View, viewOptions?: ViewOptions) => void;
  viewOptions: SettingsViewOptions;
}) {
  const initialTarget = resolveSettingsDeepLink(viewOptions.section);
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTarget?.tab ?? 'models');
  // The section a deep link asked for, waiting to be scrolled to once its tab has rendered.
  const [pendingSection, setPendingSection] = useState<string | null>(
    initialTarget?.sectionId ?? null
  );
  const tabsRef = useRef<HTMLDivElement>(null);
  // The tab on screen, readable inside effects that must not re-run when it changes.
  const activeTabRef = useRef(activeTab);
  activeTabRef.current = activeTab;

  const viewport = () =>
    tabsRef.current?.querySelector<HTMLElement>('[data-radix-scroll-area-viewport]') ?? null;

  /**
   * Show a tab. One scroller holds all three, so a change of tab would otherwise land mid-page
   * at the previous tab's offset (and a smooth scroll to a deep-linked section would glide there
   * from it), so every move to a DIFFERENT tab starts at the top.
   */
  const showTab = (tab: SettingsTab) => {
    if (tab !== activeTabRef.current) {
      const scroller = viewport();
      if (scroller) scroller.scrollTop = 0;
    }
    setActiveTab(tab);
  };

  const handleTabChange = (tab: string) => {
    if (!isSettingsTab(tab)) return;
    setPendingSection(null);
    showTab(tab);
  };

  // A deep link that arrives while Settings is already open (route state or `?section=`).
  useEffect(() => {
    const target = resolveSettingsDeepLink(viewOptions.section);
    if (!target) return;
    showTab(target.tab);
    setPendingSection(target.sectionId ?? null);
    // `showTab` only reads refs and a setter, so a change of `section` is the one trigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewOptions.section]);

  // The highlighted row and what clears it. Held outside the scroll effect: that effect re-runs
  // (and cleans up) the moment it clears `pendingSection`, which must not cancel the highlight's
  // own end.
  const highlight = useRef<{ target: HTMLElement; end: () => void } | null>(null);
  useEffect(
    () => () => {
      highlight.current?.end();
    },
    []
  );

  /** Wash `target` (`.br-highlight`) until its animation ends, with a timer for a renderer that
   *  never runs one (and for reduced motion's hold, which ends on its own schedule). */
  const highlightRow = (target: HTMLElement) => {
    highlight.current?.end();
    let timer = 0;
    const end = () => {
      window.clearTimeout(timer);
      target.removeEventListener('animationend', onAnimationEnd);
      target.classList.remove('br-highlight');
      if (highlight.current?.target === target) highlight.current = null;
    };
    // A child's animation bubbles here too, and must not end the wash early.
    const onAnimationEnd = (event: AnimationEvent) => {
      if (event.target === target) end();
    };
    target.classList.add('br-highlight');
    target.addEventListener('animationend', onAnimationEnd);
    timer = window.setTimeout(end, HIGHLIGHT_MS);
    highlight.current = { target, end };
  };

  // Scroll a deep-linked section into view and highlight its first row (the control the link
  // names: Approval mode, Tool call details, Privacy tiers, Version). Sections such as Privacy
  // and Usage load asynchronously, so the element is looked for on each frame for a short while.
  // A section that holds no row (a card, a list of its own) is washed whole.
  useEffect(() => {
    if (!pendingSection) return;
    let frame = 0;
    const started = performance.now();
    const find = () => {
      const section = document.getElementById(pendingSection);
      if (section) {
        section.scrollIntoView({
          behavior: prefersReducedMotion() ? 'auto' : 'smooth',
          block: 'start',
        });
        highlightRow(section.querySelector<HTMLElement>('.biorouter-settings-row') ?? section);
        setPendingSection(null);
        return;
      }
      if (performance.now() - started < FIND_SECTION_MS) {
        frame = window.requestAnimationFrame(find);
      } else {
        setPendingSection(null);
      }
    };
    frame = window.requestAnimationFrame(find);
    return () => window.cancelAnimationFrame(frame);
    // `highlightRow` reads only refs; a new pending section or tab is the trigger.
  }, [pendingSection, activeTab]);

  useEffect(() => {
    // ⚠ An Escape something else already handled is not ours. Radix's dialogs,
    // menus and selects dismiss on a CAPTURE-phase listener and mark the event
    // with preventDefault(), not stopPropagation(), so this bubble-phase
    // listener still hears it: closing the Usage report with Escape also left
    // Settings for Home. `sidebar.tsx` and the Crew `DetailsPane` guard the
    // same way.
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing) {
        return;
      }
      onClose();
    };

    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose]);

  return (
    // `removeTopPadding`: the band starts at y=0 and replaces the 32px drag strip (spec 3.10).
    <MainPanelLayout removeTopPadding>
      {/* The page is one band and one reading column (spec §3.13, principle 1). The band holds
          the title and the three tabs, text only, and has no description: the tabs switch which
          rows you are looking at, and the rows explain themselves. The body is the CHAT measure
          (760px), the edge Home, the transcript and the composer share: Settings is a column of
          labelled rows, and width beyond the measure only separates each control from the
          label it names. The tab names are pinned by daemon copy ("Settings > Models",
          "Settings > App > Privacy"); never rename them. */}
      <Tabs
        ref={tabsRef}
        value={activeTab}
        onValueChange={handleTabChange}
        className="flex min-h-0 flex-1 flex-col"
      >
        <PageHeader
          title={settingsShellCopy.title}
          tabs={
            // `border-b-0`: the band's own hairline is the one rule; the active underline
            // lands on it.
            <TabsList
              aria-label={settingsShellCopy.tabsLabel}
              className="biorouter-settings-tabs border-b-0"
            >
              <TabsTrigger value="models" data-testid="settings-models-tab">
                {settingsShellCopy.tabs.models}
              </TabsTrigger>
              <TabsTrigger value="chat" data-testid="settings-chat-tab">
                {settingsShellCopy.tabs.chat}
              </TabsTrigger>
              <TabsTrigger value="app" data-testid="settings-app-tab">
                {settingsShellCopy.tabs.app}
              </TabsTrigger>
            </TabsList>
          }
        />

        {/* `biorouter-scroll-fade-top` fades the clipped top edge only while the scroller is
            scrolled, so a row cut off at the band's hairline does not read as a second rule.
            Guarded at the source by `styles/settingsScrollFade.test.ts`, because jsdom computes
            no `mask-image`. */}
        <ScrollArea className="biorouter-scroll-fade-top flex-1" paddingX={1}>
          <ReadableContent size="chat" className="br-rows-flat px-6 py-5">
            {/* `tabIndex={-1}` on all three panels (vision V4-05): Radix makes a panel a Tab stop,
                and a panel has no focus treatment by design, so Tab from the tab list landed on an
                invisible stop before the first control. Every Settings panel opens with a control,
                so one Tab now goes from the tab list straight to it. */}
            <TabsContent value="models" className="mt-0" tabIndex={-1}>
              <ModelsSection setView={setView} />
            </TabsContent>
            <TabsContent value="chat" className="mt-0" tabIndex={-1}>
              <ChatSettingsSection />
            </TabsContent>
            <TabsContent value="app" className="mt-0" tabIndex={-1}>
              {/* Order is the operator's, and it is not arbitrary: Configuration, then
                  Privacy, then everything AppSettingsSection owns (General, Appearance,
                  Usage, About, Danger zone).

                  Privacy used to be a fourth tab. Four tabs for what is really two audiences
                  (what the model does, how the app behaves) made Privacy feel like a separate
                  product rather than a property of this install; it sits with Configuration
                  now because that is what it is. */}
              {/* The ONE carrier of the tail spacer: a bare wrapper between two sections
                  would stop `.biorouter-settings-section + .biorouter-settings-section` from
                  firing, so the sections below are siblings inside it. */}
              <div className="pb-8">
                {CONFIGURATION_ENABLED && <ConfigSettings />}
                <PrivacyPanel />
                <AppSettingsSection />
              </div>
            </TabsContent>
          </ReadableContent>
        </ScrollArea>
      </Tabs>
    </MainPanelLayout>
  );
}
