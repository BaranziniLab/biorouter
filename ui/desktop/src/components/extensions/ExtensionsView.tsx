import { View, ViewOptions } from '../../utils/navigationUtils';
import ExtensionsSection from '../settings/extensions/ExtensionsSection';
import { ExtensionConfig } from '../../api';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import { Plus, Search } from '../icons/app-icons';
import { useCallback, useEffect, useRef, useState } from 'react';
import kebabCase from 'lodash/kebabCase';
import ExtensionModal from '../settings/extensions/modal/ExtensionModal';
import {
  getDefaultFormData,
  ExtensionFormData,
  createExtensionConfig,
  nameToKey,
} from '../settings/extensions/utils';
import { activateExtensionDefault } from '../settings/extensions';
import { useConfig } from '../ConfigContext';
import { SearchView } from '../conversation/SearchView';
import { getSearchShortcutText } from '../../utils/keyboardShortcuts';
import { BrxtInstallModal } from '../BrxtInstallModal';
import BrowseExtensionsModal from '../baam/BrowseExtensionsModal';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader } from '../Layout/PageHeader';

export type ExtensionsViewOptions = {
  deepLinkConfig?: ExtensionConfig;
  showEnvVars?: boolean;
  brxtFilePath?: string;
};

export function getExtensionScrollBehavior(): 'auto' | 'smooth' {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
}

export default function ExtensionsView({
  viewOptions,
}: {
  onClose: () => void;
  setView: (view: View, viewOptions?: ViewOptions) => void;
  viewOptions: ExtensionsViewOptions;
}) {
  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [isBrxtModalOpen, setIsBrxtModalOpen] = useState(false);
  const [isBrowseModalOpen, setIsBrowseModalOpen] = useState(false);
  const [installedExtNames, setInstalledExtNames] = useState<Set<string>>(new Set());
  // Issue #116. The names as configured, not lowercased: `scrollToExtension`
  // resolves a DOM id from `kebabCase(name)`, so a lowercased name would miss
  // the card for every extension whose name is not already lowercase.
  const [installedExtNamesRaw, setInstalledExtNamesRaw] = useState<string[]>([]);
  const [brxtPreloadedPath, setBrxtPreloadedPath] = useState<string | undefined>(undefined);
  const [refreshKey, setRefreshKey] = useState(0);
  const [searchTerm, setSearchTerm] = useState('');
  const scrollTimerRef = useRef<number | null>(null);
  const highlightTimerRef = useRef<number | null>(null);
  const highlightedElementRef = useRef<HTMLElement | null>(null);
  const { addExtension, getExtensions } = useConfig();

  // Track configured extension names so Browse can flag what's already installed.
  useEffect(() => {
    let cancelled = false;
    getExtensions(false)
      .then((exts) => {
        if (cancelled) return;
        setInstalledExtNames(new Set(exts.map((e) => e.name.toLowerCase())));
        setInstalledExtNamesRaw(exts.map((e) => e.name));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [getExtensions, refreshKey, isBrowseModalOpen]);

  // Only trigger refresh when deep link config changes AND we don't need to show env vars
  useEffect(() => {
    if (viewOptions.deepLinkConfig && !viewOptions.showEnvVars) {
      setRefreshKey((prevKey) => prevKey + 1);
    }
  }, [viewOptions.deepLinkConfig, viewOptions.showEnvVars]);

  const scrollToExtension = useCallback((extensionName: string) => {
    if (scrollTimerRef.current !== null) window.clearTimeout(scrollTimerRef.current);
    if (highlightTimerRef.current !== null) window.clearTimeout(highlightTimerRef.current);
    if (highlightedElementRef.current) highlightedElementRef.current.style.boxShadow = '';

    scrollTimerRef.current = window.setTimeout(() => {
      scrollTimerRef.current = null;
      const element = document.getElementById(`extension-${kebabCase(extensionName)}`);
      if (element) {
        element.scrollIntoView({
          behavior: getExtensionScrollBehavior(),
          block: 'center',
        });
        highlightedElementRef.current = element;
        element.style.boxShadow =
          '0 0 0 2px color-mix(in srgb, var(--color-block-teal) 45%, transparent)';
        highlightTimerRef.current = window.setTimeout(() => {
          highlightTimerRef.current = null;
          element.style.boxShadow = '';
          if (highlightedElementRef.current === element) highlightedElementRef.current = null;
        }, 2000);
      }
    }, 200);
  }, []);

  useEffect(() => {
    return () => {
      if (scrollTimerRef.current !== null) window.clearTimeout(scrollTimerRef.current);
      if (highlightTimerRef.current !== null) window.clearTimeout(highlightTimerRef.current);
      if (highlightedElementRef.current) highlightedElementRef.current.style.boxShadow = '';
    };
  }, []);

  // Scroll to extension whenever extensionId is provided (after refresh)
  useEffect(() => {
    if (viewOptions.deepLinkConfig?.name && refreshKey > 0) {
      scrollToExtension(viewOptions.deepLinkConfig?.name);
    }
  }, [refreshKey, scrollToExtension, viewOptions.deepLinkConfig?.name]);

  // Open BrxtInstallModal automatically when a file path is pre-loaded via IPC
  useEffect(() => {
    if (viewOptions.brxtFilePath) {
      setBrxtPreloadedPath(viewOptions.brxtFilePath);
      setIsBrxtModalOpen(true);
    }
  }, [viewOptions.brxtFilePath]);

  const handleModalClose = () => {
    setIsAddModalOpen(false);
  };

  const handleAddExtension = async (formData: ExtensionFormData) => {
    // Close the modal immediately
    handleModalClose();

    const extensionConfig = createExtensionConfig(formData);

    try {
      await activateExtensionDefault({
        addToConfig: addExtension,
        extensionConfig: extensionConfig,
      });
      // Trigger a refresh of the extensions list
      setRefreshKey((prevKey) => prevKey + 1);
    } catch (error) {
      console.error('Failed to activate extension:', error);
      setRefreshKey((prevKey) => prevKey + 1);
    }
  };

  return (
    <MainPanelLayout>
      <div
        className="flex flex-col min-w-0 flex-1 overflow-y-auto relative"
        data-search-scroll-area
      >
        {/* This view is one of the four `PageHeader`'s docblock names as the
            shape the rest of the app was matched to — title, description, then
            a button strip on its own line — so it now mounts that header rather
            than restating it. What the shared component supplies and this call
            site must NOT restate: the full-bleed hairline, the `pt-12 pb-6`
            column, `text-title`/`text-secondary`, and `.biorouter-settings-control-strip`
            for the actions.

            The three Buttons carry variant and nothing else. `className="flex
            items-center gap-2"` used to sit on all three, and it is not
            harmless duplication of the cva base: a bare `flex` FLIPS the base's
            own `inline-flex` through tailwind-merge (vocabulary V7), and the
            `gap-2` and the icons' `h-4 w-4` are already what `buttonVariants`
            emits.

            The reading column is the chat measure, matching the body below it:
            the hairline is full-bleed, so a header and a body on two different
            measures show the step along the edge they share. */}
        <PageHeader
          title="Extensions"
          description={
            <>
              MCP extensions expand Biorouter's capabilities with Prompts, Resources, and Tools.
              Enabled extensions apply to all new chats. {getSearchShortcutText()} to search.
            </>
          }
          actions={
            <>
              <Button variant="default" onClick={() => setIsBrxtModalOpen(true)}>
                <Plus />
                Add Extension
              </Button>
              <Button variant="outline" onClick={() => setIsBrowseModalOpen(true)}>
                <Search />
                Browse Extensions
              </Button>
              <Button variant="outline" onClick={() => setIsAddModalOpen(true)}>
                <Plus />
                Add Custom Extension
              </Button>
            </>
          }
        />

        <ReadableContent size="chat" className="px-6 pt-6 pb-8">
          <SearchView onSearch={(term) => setSearchTerm(term)} placeholder="Search extensions...">
            <ExtensionsSection
              key={refreshKey}
              deepLinkConfig={viewOptions.deepLinkConfig}
              showEnvVars={viewOptions.showEnvVars}
              hideButtons={true}
              searchTerm={searchTerm}
              onModalClose={(extensionName: string) => {
                scrollToExtension(extensionName);
              }}
            />
          </SearchView>
        </ReadableContent>
      </div>

      {/* Modal for adding a new extension */}
      {isAddModalOpen && (
        <ExtensionModal
          title="Add custom extension"
          initialData={getDefaultFormData()}
          onClose={handleModalClose}
          onSubmit={handleAddExtension}
          submitLabel="Add Extension"
          modalType={'add'}
        />
      )}
      {isBrxtModalOpen && (
        <BrxtInstallModal
          onClose={() => {
            setIsBrxtModalOpen(false);
            setBrxtPreloadedPath(undefined);
          }}
          onInstalled={() => {
            setRefreshKey((prev) => prev + 1);
          }}
          preloadedFilePath={brxtPreloadedPath}
        />
      )}
      {isBrowseModalOpen && (
        <BrowseExtensionsModal
          onClose={() => setIsBrowseModalOpen(false)}
          onInstalled={() => setRefreshKey((prev) => prev + 1)}
          installedNames={installedExtNames}
          /**
           * Issue #116. This page owns the Browse modal but not the Settings
           * card's configuration modal — that lives inside `ExtensionsSection`
           * below — so the strongest thing it can honestly do is what the
           * acceptance criterion allows as the alternative: close the
           * marketplace and take the user to the extension's Settings entry,
           * scrolled to and highlighted, where the gear opens its credentials.
           * (`ExtensionsSection`'s own Browse modal, on the Settings route,
           * opens that configuration directly.)
           */
          onConfigureInstalled={(ext) => {
            const key = nameToKey(ext.extension_name ?? ext.name);
            const match =
              installedExtNamesRaw.find((name) => nameToKey(name) === key) ??
              installedExtNamesRaw.find((name) => nameToKey(name) === nameToKey(ext.id));
            setIsBrowseModalOpen(false);
            if (match) scrollToExtension(match);
          }}
        />
      )}
    </MainPanelLayout>
  );
}
