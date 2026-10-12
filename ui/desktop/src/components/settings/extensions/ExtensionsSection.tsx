import { useEffect, useState, useCallback, useMemo, useRef } from 'react';
import kebabCase from 'lodash/kebabCase';
import { useConfig, FixedExtensionEntry } from '../../ConfigContext';
import ExtensionList from './subcomponents/ExtensionList';
import ExtensionModal from './modal/ExtensionModal';
import { createExtensionConfig, ExtensionFormData, extensionToFormData, nameToKey } from './utils';

import { activateExtensionDefault, deleteExtension, toggleExtensionDefault } from './index';
import { isCapabilityExtension } from '../capabilities/capabilities';
import {
  CHATRECALL_KEY,
  markChatrecallSuggestionSeen,
  shouldSuggestChatrecall,
} from './chatrecallSuggestion';
import { toastService } from '../../../toasts';
import type { ExtensionConfig, ProviderTier } from '../../../api/types.gen';
import { catalogFreshnessLine, loadRegistry, type RegistryLoad } from '../../baam/registry';
import { EXTENSION_MODAL_COPY, EXTENSION_ROW_COPY } from './copy';
import { EXTENSIONS_COPY } from '../../extensions/copy';

/**
 * The extensions this page lists: everything configured except the shipped capabilities, which
 * are managed in Settings → Chat → Capabilities. The page's band counts the same list.
 */
export function listedExtensions(list: FixedExtensionEntry[]): FixedExtensionEntry[] {
  return list.filter((ext) => !isCapabilityExtension(ext));
}

/** The global default provider, as the extension cards need to describe it. */
export interface DefaultProvider {
  /** Display name — the card names what it judged against. */
  name: string;
  tier: ProviderTier;
}

interface ExtensionSectionProps {
  deepLinkConfig?: ExtensionConfig;
  showEnvVars?: boolean;
  onModalClose?: (extensionName: string) => void;
  searchTerm?: string;
}

/**
 * The extension list and the dialogs that edit one.
 *
 * The page's three ways to add an extension (marketplace, a file, a custom one) belong to the
 * page's band (`extensions/ExtensionsView.tsx`), which is the only mount; this section used to
 * carry a button strip and two modals of its own behind a `hideButtons` prop nobody left off,
 * and they are gone along with the props only they read.
 */
export default function ExtensionsSection({
  deepLinkConfig,
  showEnvVars,
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
  // The row whose menu opened the dialog, so closing it can go back there.
  const openedFromRow = useRef<string | null>(null);

  /**
   * Where focus goes when the configure dialog closes. The shared dialog gives focus back to the
   * control that was focused when it opened, and for a row's menu that control is a menu item that
   * is gone by then, so a keyboard user landed on the page and began again at the top of a long
   * list. The row's `⋯` button is what they used; the switch is the fallback. A row that is gone
   * (the extension was removed or renamed) leaves the default alone.
   */
  const returnFocusToRow = useCallback((event: Event) => {
    const name = openedFromRow.current;
    openedFromRow.current = null;
    if (!name) return;
    const row = document.getElementById(`extension-${kebabCase(name)}`);
    const target =
      row?.querySelector<HTMLElement>('[data-row-action]') ??
      row?.querySelector<HTMLElement>('[role="switch"]');
    if (!target) return;
    event.preventDefault();
    target.focus();
  }, []);
  // True when the dialog was opened from the row's Remove… item, so it starts at the removal
  // step and Cancel closes it instead of dropping the person into a form they never asked for.
  const [startAtRemoval, setStartAtRemoval] = useState(false);
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

    return listedExtensions(extensionsList).sort((a, b) => {
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
    });
  }, [extensionsList]);

  const fetchExtensions = useCallback(async () => {
    await getExtensions(true); // Force refresh - this will update the context
  }, [getExtensions]);

  const handleExtensionToggle = async (extensionConfig: FixedExtensionEntry) => {
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
          title: EXTENSION_ROW_COPY.chatrecallToastTitle,
          msg: EXTENSION_ROW_COPY.chatrecallToastMessage,
        },
        // No auto-close: this is a once-per-install prompt whose flag is burned
        // as soon as it is shown, so the shared 3s default would let decision
        // 14's only suggestion expire long before its message can be read. It
        // stays until the user dismisses it. (A toggle raises no toast of its
        // own any more: the switch is the confirmation, owner decision 6.9.)
        { autoClose: false }
      );
      markChatrecallSuggestionSeen();
    }

    return true;
  };

  const handleConfigureClick = (extension: FixedExtensionEntry) => {
    openedFromRow.current = extension.name;
    setStartAtRemoval(false);
    setSelectedExtension(extension);
    setIsModalOpen(true);
  };

  const handleRemoveClick = (extension: FixedExtensionEntry) => {
    openedFromRow.current = extension.name;
    setStartAtRemoval(true);
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
    // The row is about to go; there is nothing to return to.
    openedFromRow.current = null;
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
    setStartAtRemoval(false);
    setSelectedExtension(null);

    // Clear any navigation state that might be cached
    if (window.history.state?.deepLinkConfig) {
      window.history.replaceState({}, '', window.location.hash);
    }
  };

  return (
    <section id="extensions">
      <ExtensionList
        extensions={extensions}
        onToggle={handleExtensionToggle}
        onConfigure={handleConfigureClick}
        onRemove={handleRemoveClick}
        searchTerm={searchTerm}
        defaultProvider={defaultProvider}
        catalog={catalog}
        // §10.2's staleness line. The provenance under every row is only as good as the
        // catalogue it was read from, so how old that catalogue is belongs on the same screen,
        // once, in the first heading's help.
        catalogFreshness={catalog ? catalogFreshnessLine(catalog) : null}
      />

      {/* Modal for updating an existing extension */}
      {isModalOpen && selectedExtension && (
        <ExtensionModal
          title={EXTENSION_MODAL_COPY.updateTitle}
          initialData={extensionToFormData(selectedExtension)}
          onClose={handleModalClose}
          onSubmit={handleUpdateExtension}
          onDelete={handleDeleteExtension}
          submitLabel={EXTENSION_MODAL_COPY.updateSubmit}
          modalType={'edit'}
          startAtRemoval={startAtRemoval}
          onCloseAutoFocus={returnFocusToRow}
        />
      )}

      {/* Modal for adding extension from deeplink*/}
      {deepLinkConfigStateVar && showEnvVarsStateVar && (
        <ExtensionModal
          title={EXTENSIONS_COPY.customTitle}
          initialData={extensionToFormData({
            ...deepLinkConfig,
            enabled: true,
          } as FixedExtensionEntry)}
          onClose={handleModalClose}
          onSubmit={handleAddExtension}
          submitLabel={EXTENSIONS_COPY.customSubmit}
          modalType={'add'}
        />
      )}
    </section>
  );
}
