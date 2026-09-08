import { ScrollArea } from '../ui/scroll-area';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../ui/tabs';
import { View, ViewOptions } from '../../utils/navigationUtils';
import ModelsSection from './models/ModelsSection';
import AppSettingsSection from './app/AppSettingsSection';
import { WorkspaceSettingsSection } from './app/WorkspaceSettingsSection';
import ConfigSettings from './config/ConfigSettings';
import { ExtensionConfig } from '../../api';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Brain, Monitor, MessageSquare } from '../icons/app-icons';
import { useState, useEffect } from 'react';
import ChatSettingsSection from './chat/ChatSettingsSection';
import PrivacyPanel from './privacy/PrivacyPanel';
import { CONFIGURATION_ENABLED } from '../../updates';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader } from '../Layout/PageHeader';

export type SettingsViewOptions = {
  deepLinkConfig?: ExtensionConfig;
  showEnvVars?: boolean;
  section?: string;
};

export default function SettingsView({
  onClose,
  setView,
  viewOptions,
}: {
  onClose: () => void;
  setView: (view: View, viewOptions?: ViewOptions) => void;
  viewOptions: SettingsViewOptions;
}) {
  const [activeTab, setActiveTab] = useState('models');

  const handleTabChange = (tab: string) => {
    setActiveTab(tab);
  };

  // Determine initial tab based on section prop
  useEffect(() => {
    if (viewOptions.section) {
      // Map section names to tab values
      const sectionToTab: Record<string, string> = {
        update: 'app',
        models: 'models',
        modes: 'chat',
        styles: 'chat',
        tools: 'chat',
        app: 'app',
        chat: 'chat',
        // Privacy is no longer a tab of its own — it is a section of App, so an
        // old deep link to `section: 'privacy'` must still arrive somewhere it
        // exists rather than selecting a tab that is gone.
        privacy: 'app',
      };

      const targetTab = sectionToTab[viewOptions.section];
      if (targetTab) {
        setActiveTab(targetTab);
      }
    }
  }, [viewOptions.section]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };

    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose]);

  return (
    <>
      <MainPanelLayout>
        <div className="flex-1 flex flex-col min-h-0">
          {/* ⚠ All THREE of this view's reading columns are `size="chat"`, and
              they move together or not at all. Settings is a column of labelled
              rows — a label on the left, a control on the right — and at the
              page measure a wider window bought margin between the two rather
              than content, dragging every control away from the thing it names.
              The chat measure is what Home already uses (SessionsInsights.tsx),
              so Settings, Home, the transcript and the composer are one edge.

              The header, the tab strip and the scrolling body are three
              separate boxes precisely because the hairline under the header is
              FULL-BLEED (§4.2; it used to sit on the ReadableContent itself, so
              it stopped at the reading column while every other view's ran edge
              to edge), so a size on one and not the others is a visible step in
              that shared left edge rather than a mistake in a single component.
              `PageHeader` owns the header's box and its hairline now, and
              `measures.test.ts` asserts at the source that neither of the two
              columns still written here is left without the size.

              Settings passes no `actions`: it has none, and its tab strip is
              NOT one — a tab switches which rows you are looking at, so it
              stays below the header where it is, rather than being folded into
              the action strip the other views use. */}
          <PageHeader
            title="Settings"
            description="Manage models, chat behavior, and app preferences"
          />

          <div className="flex-1 min-h-0 flex flex-col">
            <Tabs
              value={activeTab}
              onValueChange={handleTabChange}
              className="h-full flex flex-col"
            >
              <ReadableContent size="chat" className="px-6 pt-4">
                {/* §4.2 — one rule, not two. `TabsList` carries its own bottom
                    hairline, which landed a few pixels under the header's and
                    read as a double rule unique to Settings. */}
                <TabsList className="biorouter-settings-tabs justify-start w-fit border-b-0">
                  <TabsTrigger
                    value="models"
                    className="flex gap-2 text-label"
                    data-testid="settings-models-tab"
                  >
                    <Brain className="h-4 w-4" />
                    Models
                  </TabsTrigger>
                  <TabsTrigger
                    value="chat"
                    className="flex gap-2 text-label"
                    data-testid="settings-chat-tab"
                  >
                    <MessageSquare className="h-4 w-4" />
                    Chat
                  </TabsTrigger>
                  <TabsTrigger
                    value="app"
                    className="flex gap-2 text-label"
                    data-testid="settings-app-tab"
                  >
                    <Monitor className="h-4 w-4" />
                    App
                  </TabsTrigger>
                </TabsList>
              </ReadableContent>

              <ScrollArea className="flex-1" paddingX={1}>
                <ReadableContent size="chat" className="px-6 py-5">
                  <TabsContent value="models" className="mt-0">
                    <ModelsSection setView={setView} />
                  </TabsContent>
                  <TabsContent value="chat" className="mt-0">
                    <ChatSettingsSection />
                  </TabsContent>
                  <TabsContent value="app" className="mt-0">
                    {/* Order is the operator's, and it is not arbitrary:
                        Configuration, then Privacy, then Workspace, then
                        everything AppSettingsSection owns — which ends with
                        Updates, so Updates stays at the bottom of the page.

                        Privacy used to be a fourth tab. Four tabs for what is
                        really two audiences (what the model does, how the app
                        behaves) made Privacy feel like a separate product
                        rather than a property of this install; it sits with
                        Configuration now because that is what it is. */}
                    {/* The ONE carrier of the tail spacer. Each of the four
                        below used to bring its own `pb-8` wrapper, which put a
                        bare `<div>` between every pair of sections and stopped
                        `.biorouter-settings-section + .biorouter-settings-section`
                        from ever firing — so the 10px adjacency the stylesheet
                        declares was dead on this tab and the four blocks read as
                        four separate pages. */}
                    <div className="pb-8">
                      {CONFIGURATION_ENABLED && <ConfigSettings />}
                      <PrivacyPanel />
                      <WorkspaceSettingsSection />
                      <AppSettingsSection scrollToSection={viewOptions.section} />
                    </div>
                  </TabsContent>
                </ReadableContent>
              </ScrollArea>
            </Tabs>
          </div>
        </div>
      </MainPanelLayout>
    </>
  );
}
