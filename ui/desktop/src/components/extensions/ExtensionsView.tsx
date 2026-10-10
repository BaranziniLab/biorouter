import { View, ViewOptions } from '../../utils/navigationUtils';
import ExtensionsSection, { listedExtensions } from '../settings/extensions/ExtensionsSection';
import { ExtensionConfig } from '../../api';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
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
import { BrxtInstallModal } from '../BrxtInstallModal';
import BrowseExtensionsModal from '../baam/BrowseExtensionsModal';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader } from '../Layout/PageHeader';
import { FilterInput } from '../ui/filter-input';
import { ScrollArea } from '../ui/scroll-area';
import { ExtensionLoadFailureNotice } from './ExtensionLoadFailureNotice';
import { AddExtensionMenu } from './AddExtensionMenu';
import { highlightRow } from './highlightRow';
import { EXTENSIONS_COPY } from './copy';
import { startNewSession } from '../../sessions';
import { getInitialWorkingDir } from '../../utils/workingDir';
import { toastError } from '../../toasts';
import { startChatFailureNotice } from '../../utils/startChatFailure';

export type ExtensionsViewOptions = {
  deepLinkConfig?: ExtensionConfig;
  showEnvVars?: boolean;
  brxtFilePath?: string;
};

export function getExtensionScrollBehavior(): 'auto' | 'smooth' {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth';
}

export default function ExtensionsView({
  setView,
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
  const cancelHighlightRef = useRef<(() => void) | null>(null);
  const { addExtension, getExtensions, extensionsList } = useConfig();

  // The band counts what the list lists, not what is configured: a shipped capability has no row
  // here (it lives under Settings, Chat, Capabilities), so counting it would say 13 over a list
  // of 7.
  const count = listedExtensions(extensionsList).length;

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
    cancelHighlightRef.current?.();
    cancelHighlightRef.current = null;

    scrollTimerRef.current = window.setTimeout(() => {
      scrollTimerRef.current = null;
      const element = document.getElementById(`extension-${kebabCase(extensionName)}`);
      if (element) {
        element.scrollIntoView({
          behavior: getExtensionScrollBehavior(),
          block: 'center',
        });
        // The wash `main.css` owns (`.br-highlight`), not an inline ring.
        cancelHighlightRef.current = highlightRow(element);
      }
    }, 200);
  }, []);

  useEffect(() => {
    return () => {
      if (scrollTimerRef.current !== null) window.clearTimeout(scrollTimerRef.current);
      cancelHighlightRef.current?.();
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
    <MainPanelLayout removeTopPadding>
      <div className="flex min-h-0 flex-1 flex-col">
        {/* The band (spec 3.11): the title, a count, the page's help, the filter and one Add
            menu. It sits OUTSIDE the reading column, so its hairline runs edge to edge, and
            OUTSIDE the scroller: the band IS the window's drag region on this route
            (`routeOwnsTopBand`), so one that scrolled away with the list would take the filter
            and the Add menu with it. Skills pins its band the same way.

            What it replaces: a 24px title, a two-line description with "Ctrl+F to search" in
            it, and a strip of three labelled buttons (about 228px above the first row). The
            description is the InfoTip; the shortcut is the filter's tooltip. */}
        <PageHeader
          title={EXTENSIONS_COPY.title}
          info={EXTENSIONS_COPY.info}
          adornment={count > 0 ? count : undefined}
          actions={
            <>
              <FilterInput value={searchTerm} onValueChange={setSearchTerm} />
              <AddExtensionMenu
                onBrowse={() => setIsBrowseModalOpen(true)}
                onFromFile={() => setIsBrxtModalOpen(true)}
                onCustom={() => setIsAddModalOpen(true)}
              />
            </>
          }
        />

        <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6">
          <ScrollArea className="h-full">
            <div className="relative py-4">
              {/* Above the list, because it is about an extension the list may not
                  contain. The toast that used to carry this news pointed here and
                  the page answered "No extensions yet" — the destination denied
                  the failure existed. */}
              <ExtensionLoadFailureNotice
                onAskBiorouter={(hints) =>
                  // The eighth surface that starts a chat, and it reports a failed
                  // start in the same words as the other seven — the census in
                  // `utils/startChatFailure.test.ts` is what says so. Without this
                  // the rejection was unhandled: the notice stayed on screen and
                  // nothing said the chat had not opened.
                  void startNewSession(getInitialWorkingDir(), hints, setView).catch((error) =>
                    toastError(startChatFailureNotice(error, { kept: false }))
                  )
                }
              />
              <ExtensionsSection
                key={refreshKey}
                deepLinkConfig={viewOptions.deepLinkConfig}
                showEnvVars={viewOptions.showEnvVars}
                searchTerm={searchTerm}
                onModalClose={(extensionName: string) => {
                  scrollToExtension(extensionName);
                }}
              />
            </div>
          </ScrollArea>
        </ReadableContent>
      </div>

      {/* Modal for adding a new extension */}
      {isAddModalOpen && (
        <ExtensionModal
          title={EXTENSIONS_COPY.customTitle}
          initialData={getDefaultFormData()}
          onClose={handleModalClose}
          onSubmit={handleAddExtension}
          submitLabel={EXTENSIONS_COPY.customSubmit}
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
           * Issue #116. The page owns the Browse modal but not the configure dialog (that
           * lives inside `ExtensionsSection`), so the strongest thing it can honestly do is
           * what the acceptance criterion allows as the alternative: close the marketplace
           * and take the person to the extension's row, scrolled to and highlighted, where
           * its ⋯ menu opens the configuration.
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
