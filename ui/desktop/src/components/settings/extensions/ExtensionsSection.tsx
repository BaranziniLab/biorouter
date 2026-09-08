import { useEffect, useState, useCallback, useMemo } from 'react';
import { Button } from '../../ui/button';
import { Plus, Search } from '../../icons/app-icons';
import { useConfig, FixedExtensionEntry } from '../../ConfigContext';
import ExtensionList from './subcomponents/ExtensionList';
import ExtensionModal from './modal/ExtensionModal';
import {
  createExtensionConfig,
  ExtensionFormData,
  extensionToFormData,
  getDefaultFormData,
  nameToKey,
} from './utils';

import { activateExtensionDefault, deleteExtension, toggleExtensionDefault } from './index';
import { isCapabilityExtension } from '../capabilities/capabilities';
import {
  CHATRECALL_KEY,
  markChatrecallSuggestionSeen,
  shouldSuggestChatrecall,
} from './chatrecallSuggestion';
import { toastService } from '../../../toasts';
import type { ExtensionConfig, ProviderTier } from '../../../api/types.gen';
import { BrxtInstallModal } from '../../BrxtInstallModal';
import BrowseExtensionsModal from '../../baam/BrowseExtensionsModal';
import { catalogFreshnessLine, loadRegistry, type RegistryLoad } from '../../baam/registry';

/** The global default provider, as the extension cards need to describe it. */
export interface DefaultProvider {
  /** Display name — the card names what it judged against. */
  name: string;
  tier: ProviderTier;
}

interface ExtensionSectionProps {
  deepLinkConfig?: ExtensionConfig;
  showEnvVars?: boolean;
  hideButtons?: boolean;
  disableConfiguration?: boolean;
  customToggle?: (extension: FixedExtensionEntry) => Promise<boolean | void>;
  selectedExtensions?: string[]; // Add controlled state
  onModalClose?: (extensionName: string) => void;
  searchTerm?: string;
}

export default function ExtensionsSection({
  deepLinkConfig,
  showEnvVars,
  hideButtons,
  disableConfiguration,
  customToggle,
  selectedExtensions = [],
  onModalClose,
  searchTerm = '',
}: ExtensionSectionProps) {
  const { getExtensions, addExtension, removeExtension, extensionsList, read, getProviders } =
    useConfig();
  const [defaultProvider, setDefaultProvider] = useState<DefaultProvider | null>(null);

  /**
   * §14.5's third state, scoped to what Settings can honestly compute.
   *
   * ⚠ The design asks for a state "computed against the focused session". This
   * screen has no session: a grep for `session` across `SettingsView.tsx` and
   * this file returns nothing, and with tabs and splits there is no single
   * focused chat once the user has navigated away from one. Inventing one here
   * would be a fabricated answer to a real question.
   *
   * So Settings answers the question it *can* answer — will a newly created
   * chat be able to call this? — by resolving the tier of the global default
   * provider, and the card names that provider so the scope of the claim is
   * visible. The per-chat answer lives in the composer's extension selector,
   * which is already given the id of the chat it belongs to.
   *
   * (This file is greppped for that identifier and must keep returning zero
   * hits — a fabricated "focused session" here is the wrong implementation the
   * gate exists to catch, so do not name one even in a comment.)
   */
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const name = (await read('BIOROUTER_PROVIDER', false)) as string | null;
        if (!name) return;
        const providers = await getProviders(false);
        const match = providers.find((provider) => provider.name === name);
        if (cancelled || !match?.metadata.tier) return;
        setDefaultProvider({
          name: match.metadata.display_name || match.name,
          tier: match.metadata.tier,
        });
      } catch (error) {
        // A tier nobody could read judges nothing — the cards simply say less.
        console.warn('[ExtensionsSection] Failed to resolve the default provider tier:', error);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [read, getProviders]);
  /**
   * The marketplace catalogue, resolved once for the whole screen (issue #56
   * §13.5). Each card needs to say where it came from — marketplace or a file —
   * and that is a question only the catalogue answers; resolving it per row
   * would be one IPC round-trip per extension on a screen that routinely lists
   * twenty, which is exactly why `defaultProvider` above is passed through too.
   */
  const [catalog, setCatalog] = useState<RegistryLoad | null>(null);
  useEffect(() => {
    let cancelled = false;
    loadRegistry()
      .then((load) => {
        if (!cancelled) setCatalog(load);
      })
      // `loadRegistry` documents that it never rejects, and it is hardened so
      // that stays true against a hostile document — but this screen is not the
      // right place to find out it was wrong. Without a handler the failure is
      // an unhandled rejection and every §13.5 provenance line silently
      // vanishes, which looks exactly like a slow load and never resolves.
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const [selectedExtension, setSelectedExtension] = useState<FixedExtensionEntry | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [isBrxtModalOpen, setIsBrxtModalOpen] = useState(false);
  const [isBrowseModalOpen, setIsBrowseModalOpen] = useState(false);
  const [deepLinkConfigStateVar, setDeepLinkConfigStateVar] = useState<
    ExtensionConfig | undefined | null
  >(deepLinkConfig);
  const [showEnvVarsStateVar, setShowEnvVarsStateVar] = useState<boolean | undefined | null>(
    showEnvVars
  );

  useEffect(() => {
    setDeepLinkConfigStateVar(deepLinkConfig);
    setShowEnvVarsStateVar(showEnvVars);
  }, [deepLinkConfig, showEnvVars]);

  const extensions = useMemo(() => {
    if (extensionsList.length === 0) return [];

    return (
      [...extensionsList]
        // Shipped capabilities are managed in Settings → Chat → Capabilities.
        .filter((ext) => !isCapabilityExtension(ext))
        .sort((a, b) => {
          // First sort by builtin
          if (a.type === 'builtin' && b.type !== 'builtin') return -1;
          if (a.type !== 'builtin' && b.type === 'builtin') return 1;

          // Then sort by bundled (handle null/undefined cases)
          const aBundled = 'bundled' in a && a.bundled === true;
          const bBundled = 'bundled' in b && b.bundled === true;
          if (aBundled && !bBundled) return -1;
          if (!aBundled && bBundled) return 1;

          // Finally sort alphabetically within each group
          return a.name.localeCompare(b.name);
        })
        .map((ext) => ({
          ...ext,
          // Use selectedExtensions to determine enabled state in workflow editor
          enabled: disableConfiguration ? selectedExtensions.includes(ext.name) : ext.enabled,
        }))
    );
  }, [extensionsList, disableConfiguration, selectedExtensions]);

  const fetchExtensions = useCallback(async () => {
    await getExtensions(true); // Force refresh - this will update the context
  }, [getExtensions]);

  const handleExtensionToggle = async (extensionConfig: FixedExtensionEntry) => {
    if (customToggle) {
      await customToggle(extensionConfig);
      return true;
    }

    const toggleDirection = extensionConfig.enabled ? 'toggleOff' : 'toggleOn';

    await toggleExtensionDefault({
      toggle: toggleDirection,
      extensionConfig: extensionConfig,
      addToConfig: addExtension,
    });

    await fetchExtensions();

    if (
      shouldSuggestChatrecall(
        { name: extensionConfig.name, nowEnabled: !extensionConfig.enabled },
        {
          // Keyed, not name-matched: the entry the daemon sends is called
          // "Chat Recall", so `e.name === 'chatrecall'` never matched and this
          // read `false` even with chatrecall already on — which would have
          // suggested it to someone who has it.
          chatrecallEnabled:
            extensionsList.find((e) => nameToKey(e.name) === CHATRECALL_KEY)?.enabled ?? false,
        }
      )
    ) {
      // Show first, THEN burn the one-shot. `toastService.success` returns early
      // when the singleton is in silent mode (`toasts.tsx`, `if (this.silent) return;`),
      // and `silent` is sticky once set — `handleError`'s options flow into
      // `configure()`. Burning the flag first would spend decision 14's single
      // suggestion on a toast the user never saw.
      toastService.success(
        {
          title: 'Workspace Control enabled',
          msg: 'Chat Recall pairs with it: Workspace reads and steers live chats, Chat Recall searches past ones. Turn it on under Settings → Chat → Capabilities.',
        },
        // No auto-close: this is a once-per-install prompt whose flag is burned
        // as soon as it is shown, so the shared 3s default would let decision
        // 14's only suggestion expire long before its message can be read. It
        // stays until the user dismisses it — which is also what separates it
        // from `toggleExtensionDefault`'s own "Extension enabled for new chats"
        // toast, fired for the same click and gone after 3s.
        { autoClose: false }
      );
      markChatrecallSuggestionSeen();
    }

    return true;
  };

  const handleConfigureClick = (extension: FixedExtensionEntry) => {
    setSelectedExtension(extension);
    setIsModalOpen(true);
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
    } catch (error) {
      console.error('Failed to add extension:', error);
    } finally {
      await fetchExtensions();
      if (onModalClose) {
        setTimeout(() => {
          onModalClose(formData.name);
        }, 200);
      }
    }
  };

  const handleUpdateExtension = async (formData: ExtensionFormData) => {
    if (!selectedExtension) {
      console.error('No selected extension for update');
      return;
    }

    // Close the modal immediately
    handleModalClose();

    let extensionConfig: ExtensionConfig;
    if (selectedExtension.type === 'builtin') {
      // Built-in extensions only expose timeout for editing — preserve every other field.
      extensionConfig = {
        type: 'builtin',
        name: selectedExtension.name,
        description: selectedExtension.description,
        display_name: selectedExtension.display_name,
        bundled: selectedExtension.bundled,
        available_tools: selectedExtension.available_tools,
        timeout: formData.timeout,
      };
    } else {
      extensionConfig = createExtensionConfig(formData);
    }

    const originalName = selectedExtension.name;

    try {
      if (originalName !== extensionConfig.name) {
        await removeExtension(originalName);
      }
      await addExtension(extensionConfig.name, extensionConfig, formData.enabled);
    } catch (error) {
      console.error('Failed to update extension:', error);
    } finally {
      await fetchExtensions();
    }
  };

  const handleDeleteExtension = async (name: string) => {
    handleModalClose();

    // Detect .brxt-installed extensions by their --directory arg pointing to extensions/
    const config = extensionsList.find((e) => e.name === name);
    const isBrxtInstalled =
      config?.type === 'stdio' &&
      Array.isArray(config.args) &&
      config.args[0] === 'uv' &&
      config.args[1] === 'run' &&
      config.args[2] === '--directory' &&
      typeof config.args[3] === 'string' &&
      config.args[3].includes('biorouter/extensions/');

    try {
      if (isBrxtInstalled) {
        const uninstallResult = await window.electron.uninstallBrxtExtension(name);
        if ('error' in uninstallResult) {
          toastService.error({
            title: name,
            msg: `Failed to remove extension files: ${uninstallResult.error}`,
          });
          return;
        }
      }
      await deleteExtension({
        name,
        removeFromConfig: removeExtension,
        extensionConfig: config,
      });
      if (isBrxtInstalled) {
        toastService.success({ title: name, msg: 'Extension and its skills removed' });
      }
    } catch (error) {
      console.error('Failed to delete extension:', error);
    } finally {
      await fetchExtensions();
    }
  };

  const handleModalClose = () => {
    setDeepLinkConfigStateVar(null);
    setShowEnvVarsStateVar(null);

    setIsModalOpen(false);
    setIsAddModalOpen(false);
    setSelectedExtension(null);

    // Clear any navigation state that might be cached
    if (window.history.state?.deepLinkConfig) {
      window.history.replaceState({}, '', window.location.hash);
    }
  };

  return (
    <section id="extensions">
      <div className="">
        {/* §10.2's staleness line. The provenance under every card below is only
            as good as the catalogue it was read from, so how old that catalogue
            is belongs on the same screen, once. */}
        {catalog && catalogFreshnessLine(catalog) && (
          <p className="text-xs text-text-subtle mb-3">
            Marketplace catalogue · {catalogFreshnessLine(catalog)}
          </p>
        )}
        <ExtensionList
          extensions={extensions}
          onToggle={handleExtensionToggle}
          onConfigure={handleConfigureClick}
          disableConfiguration={disableConfiguration}
          searchTerm={searchTerm}
          defaultProvider={defaultProvider}
          catalog={catalog}
        />

        {!hideButtons && (
          <div className="flex gap-4 pt-4 w-full">
            <Button
              className="flex items-center gap-2 justify-center"
              variant="default"
              onClick={() => setIsBrxtModalOpen(true)}
            >
              <Plus className="h-4 w-4" />
              Add extension
            </Button>
            <Button
              className="flex items-center gap-2 justify-center"
              variant="outline"
              onClick={() => setIsBrowseModalOpen(true)}
            >
              <Search className="h-4 w-4" />
              Browse extensions
            </Button>
            <Button
              className="flex items-center gap-2 justify-center"
              variant="outline"
              onClick={() => setIsAddModalOpen(true)}
            >
              <Plus className="h-4 w-4" />
              Add custom extension
            </Button>
          </div>
        )}

        {/* Modal for updating an existing extension */}
        {isModalOpen && selectedExtension && (
          <ExtensionModal
            title="Update extension"
            initialData={extensionToFormData(selectedExtension)}
            onClose={handleModalClose}
            onSubmit={handleUpdateExtension}
            onDelete={handleDeleteExtension}
            submitLabel="Save changes"
            modalType={'edit'}
          />
        )}

        {/* Modal for adding a new extension */}
        {isAddModalOpen && (
          <ExtensionModal
            title="Add custom extension"
            initialData={getDefaultFormData()}
            onClose={handleModalClose}
            onSubmit={handleAddExtension}
            submitLabel="Add extension"
            modalType={'add'}
          />
        )}

        {/* Modal for adding extension from deeplink*/}
        {deepLinkConfigStateVar && showEnvVarsStateVar && (
          <ExtensionModal
            title="Add custom extension"
            initialData={extensionToFormData({
              ...deepLinkConfig,
              enabled: true,
            } as FixedExtensionEntry)}
            onClose={handleModalClose}
            onSubmit={handleAddExtension}
            submitLabel="Add extension"
            modalType={'add'}
          />
        )}
      </div>
      {isBrxtModalOpen && (
        <BrxtInstallModal onClose={() => setIsBrxtModalOpen(false)} onInstalled={fetchExtensions} />
      )}
      {isBrowseModalOpen && (
        <BrowseExtensionsModal
          onClose={() => setIsBrowseModalOpen(false)}
          onInstalled={fetchExtensions}
          installedNames={new Set(extensions.map((e) => e.name.toLowerCase()))}
          /* Issue #116. An installed marketplace row opens the very same
             configuration modal the card's gear opens — this screen owns both,
             so there is no reason to make the user find the card first. */
          onConfigureInstalled={(entry) => {
            const key = nameToKey(entry.extension_name ?? entry.name);
            const match =
              extensions.find((e) => nameToKey(e.name) === key) ??
              extensions.find((e) => nameToKey(e.name) === nameToKey(entry.id));
            if (!match) return;
            setIsBrowseModalOpen(false);
            handleConfigureClick(match);
          }}
        />
      )}
    </section>
  );
}
