import React, {
  createContext,
  useContext,
  useState,
  useEffect,
  useMemo,
  useCallback,
  useRef,
} from 'react';
import {
  readAllConfig,
  readConfig,
  removeConfig,
  upsertConfig,
  getExtensions as apiGetExtensions,
  addExtension as apiAddExtension,
  removeExtension as apiRemoveExtension,
  providers,
  getProviderModels as apiGetProviderModels,
} from '../api';
import { syncBundledExtensions } from './settings/extensions';
import { userActionHeaders } from '../utils/userAction';
import { newlyInstalledExtensions, subscribeToCatalog } from '../utils/catalogSubscription';
import type { CatalogDelta } from '../api';
import { toastService } from '../toasts';
import {
  isCapabilityDefaultEnabled,
  shouldDefaultEnableAgentDrafter,
  shouldDefaultEnableWorkspace,
  shouldDefaultEnablePromotedCapability,
} from './settings/capabilities/capabilities';
import {
  PRIVACY_TIERS_KEY,
  PRIVACY_TIERS_RECORD_KEY,
  privacyTiersEnabledFromConfig,
  privacyTiersRecordFromConfig,
  type PrivacyTiersRecord,
} from './settings/privacy/privacyTiers';
import { announceAppModelSelection } from '../utils/sessionBindingSync';
import type {
  ConfigResponse,
  UpsertConfigQuery,
  ConfigKeyQuery,
  ExtensionResponse,
  ProviderDetails,
  ExtensionQuery,
  ExtensionConfig,
} from '../api';

export type { ExtensionConfig } from '../api/types.gen';

// Define a local version that matches the structure of the imported one
export type FixedExtensionEntry = ExtensionConfig & {
  enabled: boolean;
};

interface ConfigContextType {
  config: ConfigResponse['config'];
  providersList: ProviderDetails[];
  extensionsList: FixedExtensionEntry[];
  extensionWarnings: string[];
  upsert: (key: string, value: unknown, is_secret: boolean, confirm?: string) => Promise<void>;
  /**
   * Re-read the whole config from the daemon.
   *
   * `config` is a cache, and `upsert` is not the only way its keys get written:
   * `setConfigProvider` writes BIOROUTER_PROVIDER/BIOROUTER_MODEL straight to
   * the API, and the CLI or another window can write any key at any time. Every
   * such write must be followed by this, or the cache goes on serving the
   * pre-write snapshot to every consumer — silently, and for as long as it
   * takes some unrelated `upsert` to refresh it (issue #52).
   *
   * Rejects if the re-read fails, leaving the cache exactly as it was — a
   * stale snapshot is bad, an erased one is worse. Callers that only wanted
   * the refresh as bookkeeping after a write of their own should catch and log
   * rather than report their write as failed.
   */
  refreshConfig: () => Promise<void>;
  read: (key: string, is_secret: boolean) => Promise<unknown>;
  remove: (key: string, is_secret: boolean) => Promise<void>;
  addExtension: (name: string, config: ExtensionConfig, enabled: boolean) => Promise<void>;
  toggleExtension: (name: string) => Promise<void>;
  removeExtension: (name: string) => Promise<void>;
  getProviders: (b: boolean) => Promise<ProviderDetails[]>;
  getExtensions: (b: boolean) => Promise<FixedExtensionEntry[]>;
  getProviderModels: (providerName: string) => Promise<string[]>;
  disableAllExtensions: () => Promise<void>;
  enableBotExtensions: (extensions: ExtensionConfig[]) => Promise<void>;
}

interface ConfigProviderProps {
  children: React.ReactNode;
}

export class MalformedConfigError extends Error {
  constructor() {
    super('Check contents of ~/.config/biorouter/config.yaml');
    this.name = 'MalformedConfigError';
    Object.setPrototypeOf(this, MalformedConfigError.prototype);
  }
}

const ConfigContext = createContext<ConfigContextType | undefined>(undefined);

/**
 * F3 — the two keys `/agent/start` binds a new chat to.
 *
 * A write of either changes what every window's composer must state, so it is
 * announced to all of them (`utils/sessionBindingSync`), each of which re-reads
 * the pair. `ModelAndProviderContext.changeModel` announces its own writes; the
 * ones caught HERE are those that never pass through it — the local and
 * coding-agent onboarding cards, Lead/Worker settings, Settings' reset — which
 * until now left even their own window's chip naming the previous model.
 */
const APP_MODEL_SELECTION_KEYS: ReadonlySet<string> = new Set([
  'BIOROUTER_PROVIDER',
  'BIOROUTER_MODEL',
]);

export const ConfigProvider: React.FC<ConfigProviderProps> = ({ children }) => {
  const [config, setConfig] = useState<ConfigResponse['config']>({});
  const [providersList, setProvidersList] = useState<ProviderDetails[]>([]);
  const [extensionsList, setExtensionsList] = useState<FixedExtensionEntry[]>([]);
  const [extensionWarnings, setExtensionWarnings] = useState<string[]>([]);
  const configReadTicket = useRef(0);
  const appliedConfigRead = useRef(0);

  /**
   * Re-read the whole config into the cache. The only place that writes it.
   *
   * Two things this must never do, both learned the hard way:
   *
   * 1. **Empty the cache because the read failed.** `readAllConfig` is
   *    non-throwing by default, so an HTTP 500 *resolves* with `data`
   *    undefined — indistinguishable, to a `response.data?.config || {}`, from
   *    a config that really is empty. The cache is the only copy the UI has;
   *    losing it makes a fully configured app look unconfigured. So: ask the
   *    client to throw, treat a missing body as a failure too, and on any
   *    failure leave the previous snapshot exactly where it was.
   * 2. **Let an older read win.** Reads overlap — the mount read, a refresh
   *    after an API-mediated provider write, an unrelated `upsert` — and they
   *    can complete in any order. A read issued *before* a write that lands
   *    *after* the refresh which followed it would republish the pre-write
   *    snapshot, which is issue #52 all over again as a race. Each read takes
   *    a ticket on the way out and applies only if nothing newer has already
   *    been applied.
   *
   *    Note the rule is "newer than what was applied", not "the newest issued":
   *    a *failed* newer read applies nothing, and must not therefore condemn an
   *    older successful one — that would leave the cache empty on a race whose
   *    only successful read had the answer in hand.
   *
   * Rejects when the config could not be re-read, so callers can decide what
   * that means to them (see `reloadConfigAfterWrite`).
   */
  const reloadConfig = useCallback(async () => {
    const ticket = ++configReadTicket.current;

    const response = await readAllConfig({ throwOnError: true });

    const nextConfig = response.data?.config;
    if (!nextConfig) {
      throw new Error('The config read returned no configuration');
    }

    if (ticket < appliedConfigRead.current) {
      // A read issued after this one has already published a newer snapshot.
      return;
    }

    appliedConfigRead.current = ticket;
    setConfig(nextConfig);
  }, []);

  /**
   * Refresh the cache after a write that already succeeded.
   *
   * The write is the caller's outcome; the re-read is bookkeeping. Failing an
   * upsert because a *cache* could not be re-read reports the wrong thing in
   * the more alarming direction, and worse, a caller writing several keys in
   * sequence (`ProviderGuard` writes an API key, then the provider) would stop
   * partway through and leave a half-configured provider behind. If the daemon
   * is genuinely unreachable, the write itself has already failed and said so.
   */
  const reloadConfigAfterWrite = useCallback(async () => {
    try {
      await reloadConfig();
    } catch (error) {
      console.error('Failed to refresh the cached config after a write:', error);
    }
  }, [reloadConfig]);

  const upsert = useCallback(
    async (key: string, value: unknown, isSecret: boolean = false, confirm?: string) => {
      const query: UpsertConfigQuery = {
        key: key,
        value: value,
        is_secret: isSecret,
        // Issue #56 Task 30: the typed confirmation Settings > Privacy sends
        // with a write to `BIOROUTER_PRIVACY_TIERS`. Absent for every other
        // caller, which is the whole point — the daemon refuses a bare upsert of
        // that key, so a model composing an ordinary config write cannot flip
        // the master switch.
        ...(confirm === undefined ? {} : { confirm }),
      };
      await upsertConfig({
        body: query,
        // P-05. WITHOUT this the generated client resolves on a 4xx/5xx — it
        // returns `{ error }` and `throwOnError` defaults to false — so every
        // caller of this hook reported a refused write as a successful one.
        // That is how "Turn off privacy tiers" would have gone from hanging to
        // *lying*: the daemon answers 403 with a reason, and the panel would
        // have flipped the switch and shown the feature as off while it was
        // still enforcing. A write that did not happen must not resolve.
        throwOnError: true,
        // Issue #56 DR-16: this is the GUI's ONLY path to `/config/upsert`, and
        // real settings screens write capability keys through it —
        // BIOROUTER_LEAD_MODEL / BIOROUTER_LEAD_PROVIDER from Lead/Worker
        // settings, OLLAMA_HOST and LLAMACPP_EXTERNAL_HOST from the provider
        // forms. Those are the user editing their own settings; the daemon
        // guards the same four keys against a model curling the route, and
        // without this header it could not tell the two apart.
        headers: await userActionHeaders(),
      });
      await reloadConfigAfterWrite();
      // After the write resolved — a refused one threw above and moved nothing.
      if (APP_MODEL_SELECTION_KEYS.has(key)) announceAppModelSelection();
    },
    [reloadConfigAfterWrite]
  );

  const read = useCallback(async (key: string, is_secret: boolean = false) => {
    const query: ConfigKeyQuery = { key: key, is_secret: is_secret };
    const response = await readConfig({
      body: query,
    });
    return response.data;
  }, []);

  const remove = useCallback(
    async (key: string, is_secret: boolean) => {
      const query: ConfigKeyQuery = { key: key, is_secret: is_secret };
      await removeConfig({
        body: query,
        // Issue #56 DR-16: the same guard as `upsert`, because a DELETE of a
        // capability key restores its default and `OLLAMA_HOST`'s default is
        // loopback — i.e. Private. This is the GUI's only path to
        // `/config/remove`, and clearing a provider's host from the provider
        // form comes through here.
        headers: await userActionHeaders(),
      });
      await reloadConfigAfterWrite();
      if (APP_MODEL_SELECTION_KEYS.has(key)) announceAppModelSelection();
    },
    [reloadConfigAfterWrite]
  );

  /**
   * The last extension list we successfully read, as a ref.
   *
   * ⚠ **`refreshExtensions` must not close over `extensionsList` as a
   * dependency**, and the reason is not tidiness — it is the runaway loop that
   * took the whole renderer down in 1.89.5.
   *
   * This function ends by calling `setExtensionsList` with an array parsed
   * fresh from the response body, so it is never reference-equal to the one in
   * state and React always commits the update. With `extensionsList` in the
   * dependency array, *every successful refresh minted a new
   * `refreshExtensions`* — and the catalogue subscription below listed that
   * function among its effect dependencies, so every refresh tore the
   * subscription down and started a new one from `since = 0`. A restart at zero
   * is answered by the daemon **immediately** rather than parked (the caller is
   * behind), which re-fired the handler, which refreshed again. Two correct
   * pieces, feeding each other, with a loopback round trip as the only brake.
   *
   * The fallback below is the only thing the old dependency bought, and a ref
   * serves it exactly as well while keeping this function's identity stable for
   * the life of the provider.
   */
  const extensionsListRef = useRef<FixedExtensionEntry[]>([]);
  useEffect(() => {
    extensionsListRef.current = extensionsList;
  }, [extensionsList]);

  const refreshExtensions = useCallback(async () => {
    const result = await apiGetExtensions();

    // ⚠ `result.response` is OPTIONAL in practice, whatever the generated types
    // say. On a network-level failure the client returns `response: undefined`
    // (`api/client/client.gen.ts` returns `response: undefined as any` from its
    // fetch catch), so an unguarded `.status` throws
    // `TypeError: Cannot read properties of undefined (reading 'status')` —
    // which is how a backend that merely could not be reached surfaced as an
    // unhandled rejection rather than as a handled, reported failure.
    if (result.response?.status === 422) {
      throw new MalformedConfigError();
    }

    if (result.error && !result.data) {
      console.log(result.error);
      return extensionsListRef.current;
    }

    if (!result.data) {
      // No body and no error: nothing was learned, so nothing is published. The
      // cache keeps what it had rather than being emptied by a failed read.
      return extensionsListRef.current;
    }

    const extensionResponse: ExtensionResponse = result.data;
    setExtensionsList(extensionResponse.extensions);
    setExtensionWarnings(extensionResponse.warnings || []);
    return extensionResponse.extensions;
  }, []);

  const addExtension = useCallback(
    async (name: string, config: ExtensionConfig, enabled: boolean) => {
      const query: ExtensionQuery = { name, config, enabled };
      await apiAddExtension({
        body: query,
      });
      await reloadConfigAfterWrite();
      // Refresh extensions list after successful addition
      await refreshExtensions();
    },
    [reloadConfigAfterWrite, refreshExtensions]
  );

  const removeExtension = useCallback(
    async (name: string) => {
      await apiRemoveExtension({ path: { name: name } });
      await reloadConfigAfterWrite();
      // Refresh extensions list after successful removal
      await refreshExtensions();
    },
    [reloadConfigAfterWrite, refreshExtensions]
  );

  const getExtensions = useCallback(
    async (forceRefresh = false): Promise<FixedExtensionEntry[]> => {
      if (forceRefresh || extensionsList.length === 0) {
        return await refreshExtensions();
      }
      return extensionsList;
    },
    [extensionsList, refreshExtensions]
  );

  const toggleExtension = useCallback(
    async (name: string) => {
      const exts = await getExtensions(true);
      const extension = exts.find((ext) => ext.name === name);

      if (extension) {
        await addExtension(name, extension, !extension.enabled);
      }
    },
    [addExtension, getExtensions]
  );

  const getProviders = useCallback(
    async (forceRefresh = false): Promise<ProviderDetails[]> => {
      if (forceRefresh || providersList.length === 0) {
        try {
          const response = await providers();
          const providersData = response.data || [];
          setProvidersList(providersData);
          return providersData;
        } catch (error) {
          console.error('Failed to fetch providers:', error);
          return [];
        }
      }
      return providersList;
    },
    [providersList]
  );

  const getProviderModels = useCallback(async (providerName: string): Promise<string[]> => {
    try {
      const response = await apiGetProviderModels({
        path: { name: providerName },
        throwOnError: true,
      });
      return response.data || [];
    } catch (error) {
      console.error(`Failed to fetch models for provider ${providerName}:`, error);
      return [];
    }
  }, []);

  /**
   * Issue #112. Follow the daemon's extension catalogue for the life of the app.
   *
   * ⚠ **The writes this context makes are not the only writes.** `addExtension`
   * and `removeExtension` below refresh the cache themselves, and for a while
   * that looked like enough — but `biorouter extension install` runs in a
   * different process, a deep link and a hand-edited `config.yaml` bypass the
   * renderer entirely, and an agent installs through the daemon. None of those
   * touch this provider, so the extension the user just installed had no row in
   * the composer's picker to toggle, and they were told to open a new chat.
   *
   * One long poll, invalidating one cache, feeding every surface that reads it.
   * The delta itself is deliberately NOT applied: it says something moved, and
   * this refetches. Applying a partial history and believing yourself current is
   * the same stale-inventory bug one layer down.
   */
  /**
   * The subscription's handler, held in a ref so the effect below can mount
   * ONCE and never re-subscribe.
   *
   * ⚠ This is belt-and-braces on top of the stable `refreshExtensions` above,
   * and it is worth having both. Listing callbacks as effect dependencies is
   * the idiomatic thing to do and reads as obviously correct; what makes it
   * unsafe *here* is that a re-subscribe is not a cheap no-op but a cursor
   * reset to zero, which the daemon answers instantly. Anything that costs a
   * restart must not be re-run because a function identity moved — so the
   * effect depends on nothing, and the handler is read fresh at call time.
   */
  const onCatalogChangeRef = useRef<(delta: CatalogDelta) => void>(() => {});

  useEffect(() => {
    onCatalogChangeRef.current = (delta: CatalogDelta) => {
      // ⚠ `.catch`, not `void`. These are fire-and-forget by intent, but
      // "nobody is waiting for the result" is not the same as "nobody handles a
      // failure": a bare `void` on a rejecting promise is an unhandled
      // rejection, and that is how a merely-unreachable daemon reached the log
      // as an uncaught TypeError instead of a console warning.
      refreshExtensions().catch((error: unknown) =>
        console.warn('Catalogue changed, but the extension list could not be refreshed:', error)
      );
      reloadConfigAfterWrite().catch((error: unknown) =>
        console.warn('Catalogue changed, but the config could not be re-read:', error)
      );

      // ⚠ OFFER, never attach. A running chat snapshots the extensions it
      // started with, and an install made somewhere else — another terminal,
      // another window — is not that chat's decision to have made. An agent
      // asked to install one *in* a chat attaches it itself, because there
      // the user did ask; here they did not, so this says the row is now
      // there and leaves the click to them.
      for (const extension of newlyInstalledExtensions(delta)) {
        toastService.success({
          title: extension.name,
          msg: 'Extension installed. Turn it on for this chat from the extensions menu below the composer.',
        });
      }
    };
  }, [refreshExtensions, reloadConfigAfterWrite]);

  useEffect(() => {
    return subscribeToCatalog({ onChange: (delta) => onCatalogChangeRef.current(delta) });
    // Mount-once, deliberately. See `onCatalogChangeRef` above: a re-subscribe
    // resets the cursor to zero, and a zero cursor is answered without parking.
  }, []);

  useEffect(() => {
    // Load all configuration data and providers on mount
    (async () => {
      // Load config through the same reader every later refresh uses, so there
      // is one set of rules about failures and ordering rather than two.
      try {
        await reloadConfig();
      } catch (error) {
        // The config is one of three independent loads here. Letting its
        // failure escape took providers and extensions down with it and left
        // an unhandled rejection behind; the cache simply stays empty until a
        // later refresh succeeds.
        console.error('Failed to load config:', error);
      }

      // Load providers
      try {
        const providersResponse = await providers();
        const providersData = providersResponse.data || [];
        setProvidersList(providersData);
      } catch (error) {
        console.error('Failed to load providers:', error);
        setProvidersList([]);
      }

      // Load extensions
      try {
        const extensionsResponse = await apiGetExtensions();
        let extensions = extensionsResponse.data?.extensions || [];

        // Always sync from bundled-extensions.json so new built-ins added across
        // versions get picked up automatically. syncBundledExtensions is idempotent —
        // it skips bundled extensions already present in the user's config.
        const addExtensionForSync = async (
          name: string,
          config: ExtensionConfig,
          enabled: boolean
        ) => {
          const query: ExtensionQuery = { name, config, enabled };
          await apiAddExtension({ body: query });
        };
        await syncBundledExtensions(extensions, addExtensionForSync);

        const capabilityMigrations = [
          {
            flag: 'biorouter.capabilities.defaultEnabled.v1',
            shouldEnable: (ext: FixedExtensionEntry) =>
              !ext.enabled && isCapabilityDefaultEnabled(ext),
          },
          {
            flag: 'biorouter.capabilities.promotedDefaults.v2',
            shouldEnable: shouldDefaultEnablePromotedCapability,
          },
          {
            flag: 'biorouter.capabilities.agentDrafterDefault.v3',
            shouldEnable: shouldDefaultEnableAgentDrafter,
          },
          {
            // #76. Required, not optional: the Rust `default_enabled` is only
            // consulted when config.yaml has no stored entry, and saving any
            // extension persists the whole injected map — so most installs
            // already carry `workspace: {enabled: false}` and would never see
            // the new default.
            flag: 'biorouter.capabilities.workspaceDefault.v4',
            shouldEnable: shouldDefaultEnableWorkspace,
          },
        ];

        for (const migration of capabilityMigrations) {
          if (localStorage.getItem(migration.flag)) continue;

          try {
            const current = (await apiGetExtensions()).data?.extensions || [];
            for (const ext of current) {
              if (!migration.shouldEnable(ext)) continue;

              const { enabled: _omit, ...cfg } = ext;
              await addExtensionForSync(ext.name, cfg as ExtensionConfig, true);
            }
          } catch (e) {
            console.error('Capability default-enable migration failed:', e);
          }
          localStorage.setItem(migration.flag, '1');
        }

        const refreshedResponse = await apiGetExtensions();
        extensions = refreshedResponse.data?.extensions || [];

        setExtensionsList(extensions);
        setExtensionWarnings(extensionsResponse.data?.warnings || []);
      } catch (error) {
        console.error('Failed to load extensions:', error);
      }
    })();
  }, [reloadConfig]);

  const contextValue = useMemo(() => {
    const disableAllExtensions = async () => {
      const currentExtensions = await getExtensions(true);
      for (const ext of currentExtensions) {
        if (ext.enabled) {
          await addExtension(ext.name, ext, false);
        }
      }
      await reloadConfigAfterWrite();
    };

    const enableBotExtensions = async (extensions: ExtensionConfig[]) => {
      for (const ext of extensions) {
        await addExtension(ext.name, ext, true);
      }
      await reloadConfigAfterWrite();
    };

    return {
      config,
      providersList,
      extensionsList,
      extensionWarnings,
      upsert,
      refreshConfig: reloadConfig,
      read,
      remove,
      addExtension,
      removeExtension,
      toggleExtension,
      getProviders,
      getExtensions,
      getProviderModels,
      disableAllExtensions,
      enableBotExtensions,
    };
  }, [
    config,
    providersList,
    extensionsList,
    extensionWarnings,
    upsert,
    read,
    remove,
    addExtension,
    removeExtension,
    toggleExtension,
    getProviders,
    getExtensions,
    getProviderModels,
    reloadConfig,
    reloadConfigAfterWrite,
  ]);

  return <ConfigContext.Provider value={contextValue}>{children}</ConfigContext.Provider>;
};

export const useConfig = () => {
  const context = useContext(ConfigContext);
  if (context === undefined) {
    throw new Error('useConfig must be used within a ConfigProvider');
  }
  return context;
};

/**
 * Whether the daemon is enforcing privacy tiers (issue #56, DR-15).
 *
 * Read off the config cache rather than fetched, because the caller is
 * `PrivacyBadge` and there are a dozen of those on a session list. The cache is
 * refreshed on mount and after every write, including Settings → Privacy's own,
 * so a flip repaints every badge in the app on the next render.
 *
 * ⚠ **Returns `true` — enforcing — outside a `ConfigProvider`, rather than
 * throwing.** Same shape as `useResolvedTheme`, and for a sharper reason: the
 * fallback decides what a badge SAYS. A hook that threw would make the badge
 * unmountable outside the provider; one that fell back to `false` would print
 * "enforcement off" on a machine that is enforcing, which is the same class of
 * false statement DR-15 rejects the unchanged pill for. The safe fallback is the
 * one that claims nothing.
 */
export function usePrivacyTiersEnabled(): boolean {
  const context = useContext(ConfigContext);
  if (context === undefined) return true;
  return privacyTiersEnabledFromConfig(context.config[PRIVACY_TIERS_KEY]);
}

/**
 * The daemon's report on the master switch's record — where it lives and which
 * door last wrote it (H3). Off the same cache snapshot as
 * {@link usePrivacyTiersEnabled}, so the two are one read and cannot describe
 * different moments; a Settings → Privacy flip refreshes both.
 *
 * `null` outside a `ConfigProvider` and when the daemon sent no report. It says
 * HOW the switch got its value and never WHETHER it is off — that is
 * {@link usePrivacyTiersEnabled}'s alone, so a missing report can only cost the
 * explanation, never the notice.
 */
export function usePrivacyTiersRecord(): PrivacyTiersRecord | null {
  const context = useContext(ConfigContext);
  if (context === undefined) return null;
  return privacyTiersRecordFromConfig(context.config[PRIVACY_TIERS_RECORD_KEY]);
}
