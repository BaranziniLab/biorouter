import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { ProviderCard } from './subcomponents/ProviderCard';
import ProviderConfigurationModal from './modal/ProviderConfiguationModal';
import {
  DeclarativeProviderConfig,
  getCustomProvider,
  ProviderDetails,
  updateCustomProvider,
  UpdateCustomProviderRequest,
} from '../../../api';
import { Plus } from '../../icons/app-icons';
import { ModalShell } from '../../ModalShell';
import { InfoTip } from '../../ui/info-tip';
import { SettingSection } from '../../ui/setting-row';
import CustomProviderForm from './modal/subcomponents/forms/CustomProviderForm';
import { SwitchModelModal } from '../models/subcomponents/SwitchModelModal';
import { holdChatModel } from '../models/pendingChatModel';
import type Model from '../models/modelInterface';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../ui/tabs';
import type { View } from '../../../utils/navigationUtils';
import type { SessionClassification } from '../../../api';
import { useConfig } from '../../ConfigContext';
import { userActionHeaders } from '../../../utils/userAction';
import { Note } from '../../ui/note';
import { Button } from '../../ui/button';
import { Skeleton } from '../../ui/skeleton';
import { llamaServerStore } from '../models/llamaServerStore';
import { catalogCopy, customProviderCopy, providerPageCopy } from './copy';
import './providers.css';
import {
  RETIRED_AZURE_ENDPOINT_NOTICE,
  isRetiredAzureDefaultEndpoint,
} from './azureRetiredEndpoint';
import {
  AI_AGENT_PROVIDER_IDS,
  getOrderedProviderGroups,
  type OrderedProviderGroup,
  type OrderedProviderSection,
  type ProviderGroupKey,
} from './providerOrdering';
import { NonPrivateModelDisclosureNote } from '../../privacy/NonPrivateModelDisclosureNote';
import { HostManagedModelNote } from '../../privacy/HostManagedModelNote';
import LlamaServerInlineCard from '../../onboarding/LlamaServerInlineCard';
import OllamaInlineCard from '../../onboarding/OllamaInlineCard';
import InstitutionalSetupCard, {
  INSTITUTIONAL_SETUP_FLAVOR,
} from '../../onboarding/InstitutionalSetupCard';
import CommercialSetupCard, {
  type DetectedProviderSetup,
} from '../../onboarding/CommercialSetupCard';
import {
  CodingAgentBody,
  CodingAgentLoadError,
  CodingAgentProvenance,
  StatusPill,
  pillFor,
  useCodingAgents,
} from '../../onboarding/codingAgentControls';
import type { CodingAgentAvailability } from '../../onboarding/codingAgentStatus';

export type ProviderCatalogMode = 'settings' | 'onboarding';

/**
 * The provider catalog: every provider the daemon serves, in three tabs that
 * carry §14.5's privacy taxonomy, and — in `onboarding` mode — the first-run
 * setup UI for each one, opened in place.
 *
 * ## One component, two modes
 *
 * The settings page and the first-run screen were two different lists of the
 * same providers, ordered differently, labelled differently, and one of them
 * had five bespoke cards for five providers and no way to reach the other
 * twenty-three. They are one surface now, and the mode changes only what a row
 * *does* when it opens: settings opens the configure modal; onboarding opens the
 * provider's own setup UI, which is the same component the cards used, rendered
 * without its card chrome (see `OnboardingCardShell`).
 *
 * ## What is deliberately NOT keyed on a provider name here
 *
 * The tab a provider lands in, the section inside it, and the institution a
 * private gateway is grouped under all come from the daemon — `metadata.tier`,
 * `metadata.runs_locally`, and the affiliation payload — via
 * `providerOrdering.ts`. **No institution is ever named by a literal in this
 * file**: "UCSF" is rendered from `affiliation.institutions[].display_name`, so
 * a second institution needs no edit here at all.
 *
 * The one thing that *is* name-keyed is which bespoke setup component a row
 * opens ({@link ONBOARDING_SETUP}), and that is a different kind of fact: a
 * hand-written setup form for one provider's own keys. `ProviderGuard` wired
 * exactly the same five components by name before this file existed. The
 * distinction worth holding on to is that a wrong entry there costs a setup
 * panel, while a wrong institution name would tell a user their data is covered
 * by an agreement it is not.
 */
interface ProviderCatalogProps {
  providers: ProviderDetails[];
  mode: ProviderCatalogMode;
  refreshProviders?: () => void;
  setView?: (view: View) => void;
  /** A model was chosen in the Choose Model modal — onboarding is finished. */
  onModelSelected?: (model?: string) => void;
  /**
   * A local provider configured itself *and* picked its own model, so there is
   * no model step to run. Llama Server and Ollama both do this.
   */
  onLocalComplete?: () => void;
  /** A setup flow started a network round trip that must not be interrupted. */
  onStartTesting?: () => void;
  /** Detection succeeded; the host persists the key and opens model selection. */
  onCommercialSuccess?: (setup: DetectedProviderSetup) => void | Promise<void>;
  /**
   * `BIOROUTER_PROVIDER`. Drives the default tab — see {@link defaultCatalogTab}.
   * `undefined` means "not read yet"; `null`/`''` means "nothing configured".
   */
  configuredProvider?: string | null;
  /** A route hint (`?tab=public`), which outranks every computed default. */
  initialTab?: string | null;
  /**
   * W2-PRV-5. The chat whose model picker opened this catalog ("Use other
   * provider"), and its tier. The model step that follows a setup then
   * switches THAT chat, with the chat's own scope and pre-flight, instead of
   * the model every new chat starts on.
   */
  chatSessionId?: string | null;
  chatPrivacyTier?: SessionClassification;
  /**
   * T3-SH-2. The tab of a chat not sent yet whose model picker opened this
   * catalog. It has no session for {@link chatSessionId} to name, so the model
   * step holds its pick for that chat (`holdChatModel`), with "for this chat"
   * and "Also use for new chats", instead of setting the model every new chat
   * starts on. Ignored when {@link chatSessionId} is set.
   */
  heldChatTabId?: string | null;
  /**
   * The element the tab list is drawn into: the page band's tab slot (`ProviderSettingsPage`),
   * so Local, Institutional and Public sit on the title's line like Settings' three tabs do.
   * Omitted, the list is drawn at the top of the catalog (first run's `ProviderGuard`, which has
   * its own welcome header). `null` means the slot is not mounted yet: nothing is drawn, and the
   * tabs appear the moment it is.
   */
  tabsHost?: HTMLElement | null;
  /**
   * The provider list is still being fetched (`providers` is not the answer yet). The catalog
   * draws its quiet skeleton instead of a tab that would say "Nothing here yet." about providers
   * it has not been told about (first run, vision FR-R1-1).
   */
  loading?: boolean;
  /**
   * Draw nothing but the skeleton until the opening tab is known, then open on it at rest and
   * keep it: the provider list, `configuredProvider` (`undefined` = not read yet) and, when it can
   * decide the tab (no route hint, no bound provider), the coding-agent probe, waited for at most
   * {@link AGENT_PROBE_WAIT_MS}. Without it the first run painted Local, then switched itself to
   * Public a second later when the probe answered, under the pointer (FR-R1-1).
   */
  holdUntilSettled?: boolean;
}

/**
 * How long a held catalog waits for the coding-agent probe before it opens without it. The probe
 * spawns two vendor CLIs and answered in about 1.4 s in the first-run review; a hung CLI must not
 * keep the first screen a skeleton. After this the probe can no longer move the tab.
 */
export const AGENT_PROBE_WAIT_MS = 2500;

/**
 * The provider ids that open a bespoke first-run setup panel, and which one.
 *
 * ⚠ Wiring, not taxonomy — see the class note above. The institutional gateways
 * open their form through `INSTITUTIONAL_SETUP_FLAVOR` (the form's own table of the
 * rows it sets up), and only while they are not set up: a configured gateway's row
 * is chosen on a click, as any configured row is.
 */
const ONBOARDING_SETUP: Record<string, 'llamacpp' | 'ollama'> = {
  llamacpp: 'llamacpp',
  ollama: 'ollama',
};

/** The tab a `?tab=` hint names, or `null` when it names nothing we have. */
export function tabFromHint(hint: string | null | undefined): ProviderGroupKey | null {
  switch ((hint ?? '').trim().toLowerCase()) {
    case 'local':
      return 'local';
    case 'institutional':
      return 'institutional';
    // Both spellings: the group's key is `commercial`, the tab's label is
    // "Public", and a CTA writing either should land in the same place.
    case 'public':
    case 'commercial':
      return 'commercial';
    default:
      return null;
  }
}

/**
 * Which tab opens — computed from state, never hardcoded.
 *
 * The rule, in order, and every clause is a habit the catalog is trying to
 * respect rather than a preference of its own:
 *
 * 1. **A route hint wins outright**, even when it names an empty tab: a CTA that
 *    says "set up an institutional model" has to land there and show that there
 *    is nothing yet, not silently go somewhere else.
 * 2. **The tab holding the provider you already use.** Someone who opens the
 *    catalog while bound to Versa is nearly always going there.
 * 3. **Public, when a coding-agent CLI reports `signed_in_subscription`** — the
 *    machine already has a plan ready to use and the cheapest path is one click,
 *    with no key to paste and nothing to download.
 * 4. **Local otherwise**, which is where the product ranks Local everywhere else.
 * 5. **…unless that tab is empty**, in which case the first tab with anything in
 *    it. Landing on an empty panel is worse than landing on the "wrong" one, and
 *    this is reachable: a daemon serving no local providers at all sends a user
 *    who has configured nothing to an empty Local tab under rule 4.
 */
export function defaultCatalogTab({
  groups,
  configuredProvider,
  hasSubscriptionReadyAgent,
  routeHint,
}: {
  groups: OrderedProviderGroup[];
  configuredProvider?: string | null;
  hasSubscriptionReadyAgent: boolean;
  routeHint?: string | null;
}): ProviderGroupKey {
  const hinted = tabFromHint(routeHint);
  if (hinted) return hinted;

  const chosen = (() => {
    if (configuredProvider) {
      const owning = groups.find((group) =>
        group.providers.some((provider) => provider.name === configuredProvider)
      );
      if (owning) return owning.key;
    }
    if (hasSubscriptionReadyAgent) return 'commercial';
    return 'local';
  })();

  const isEmpty = (key: ProviderGroupKey) =>
    (groups.find((group) => group.key === key)?.providers.length ?? 0) === 0;
  if (!isEmpty(chosen)) return chosen;
  return groups.find((group) => group.providers.length > 0)?.key ?? chosen;
}

const AddCustomProviderRow = React.memo(function AddCustomProviderRow({
  onClick,
}: {
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-testid="add-custom-provider-card"
      onClick={onClick}
      className="biorouter-settings-row br-setting-row w-full cursor-pointer text-left"
    >
      <Plus className="size-4 shrink-0 text-text-muted" aria-hidden />
      <span className="text-label text-text-muted">{catalogCopy.addCustom}</span>
    </button>
  );
});

export default function ProviderCatalog({
  providers,
  mode,
  refreshProviders,
  setView,
  onModelSelected,
  onLocalComplete,
  onStartTesting,
  onCommercialSuccess,
  configuredProvider,
  initialTab,
  chatSessionId = null,
  chatPrivacyTier,
  heldChatTabId = null,
  tabsHost,
  loading = false,
  holdUntilSettled = false,
}: ProviderCatalogProps) {
  const isOnboarding = mode === 'onboarding';
  const { read } = useConfig();
  // T3-SH-2: a chat not sent yet, named by its tab, gets the model step's pick.
  const unsentChatTabId = !chatSessionId && heldChatTabId ? heldChatTabId : undefined;
  const holdForUnsentChat = useCallback(
    (model: Model) => {
      if (unsentChatTabId) holdChatModel(unsentChatTabId, model);
    },
    [unsentChatTabId]
  );

  // W2-PRV-1. A configured Azure OpenAI whose saved endpoint is the one older
  // versions filled in by mistake (UCSF's gateway) says so on its row. Read only
  // for a configured row, and never rewritten from here.
  const azureConfigured = providers.some(
    (provider) => provider.name === 'azure_openai' && provider.is_configured
  );
  const [azureOnRetiredEndpoint, setAzureOnRetiredEndpoint] = useState(false);
  useEffect(() => {
    if (!azureConfigured) {
      setAzureOnRetiredEndpoint(false);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const endpoint = await read('AZURE_OPENAI_ENDPOINT', false);
        if (!cancelled) setAzureOnRetiredEndpoint(isRetiredAzureDefaultEndpoint(endpoint));
      } catch {
        if (!cancelled) setAzureOnRetiredEndpoint(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [azureConfigured, read, providers]);

  const [configuringProvider, setConfiguringProvider] = useState<ProviderDetails | null>(null);
  const [showCustomProviderModal, setShowCustomProviderModal] = useState(false);
  const [showSwitchModelModal, setShowSwitchModelModal] = useState(false);
  const [switchModelProvider, setSwitchModelProvider] = useState<string | null>(null);
  const [switchModelInitial, setSwitchModelInitial] = useState<string | null>(null);
  const [editingProvider, setEditingProvider] = useState<{
    id: string;
    config: DeclarativeProviderConfig;
    isEditable: boolean;
    /** Set up already, so a blank key keeps the saved one (W2-PRV-13). */
    isConfigured: boolean;
  } | null>(null);

  const handleProviderReady = useCallback((providerId: string, model?: string | null) => {
    setSwitchModelProvider(providerId);
    setSwitchModelInitial(model ?? null);
    setShowSwitchModelModal(true);
  }, []);

  /**
   * The detection card validated a key and the host has persisted it; the only
   * step left is choosing which of that provider's models becomes the default.
   *
   * ⚠ The write is the HOST's, not this component's — `ProviderGuard` and the
   * `welcome` route both call `persistDetectedProviderSetup`, and the ordering
   * inside it is what `ProviderGuard.test.tsx` pins. What the catalog owns is
   * only what happens next on screen.
   */
  const handleCommercialDetected = useCallback(
    async (setup: DetectedProviderSetup) => {
      await onCommercialSuccess?.(setup);
      refreshProviders?.();
      handleProviderReady(setup.provider, setup.model || null);
    },
    [handleProviderReady, onCommercialSuccess, refreshProviders]
  );

  const handleAgentConnected = useCallback(
    (providerId: string) => {
      // "Use <agent>" has just saved the agent's command key, which is what
      // makes the daemon report it configured. The picker that opens next reads
      // the list for itself; the row behind it must not keep the old answer.
      refreshProviders?.();
      handleProviderReady(providerId);
    },
    [handleProviderReady, refreshProviders]
  );

  /**
   * ⚠ **Mounted once, at the catalog — never per row.** `GET
   * /coding_agents/status` spawns both vendor CLIs, so one probe per agent row
   * would fork four processes on every render of this panel. `useCodingAgents`
   * fetches on mount and on an explicit "Check again" only.
   *
   * A re-check also re-reads the provider list. The row's "Configured" check is
   * `is_configured`, which the daemon only grants a coding agent whose CLI
   * resolves, so installing (or removing) a CLI and pressing "Check again" moves
   * the check along with the pill — F6 was those two disagreeing on one line.
   */
  const agentControls = useCodingAgents(handleAgentConnected, refreshProviders);
  const recheckAgents = agentControls.refresh;

  /**
   * Re-read everything a change to one provider's setup can have changed.
   *
   * ⚠ **For a coding agent that is two answers, not one.** Its only config key
   * names the CLI, so saving it moves the status pill as well as
   * `is_configured`. Re-reading the list alone is what left the row reading
   * "Not installed" beside a fresh "✓ Configured" once `CODEX_COMMAND` was
   * corrected in the configure form — F6's contradiction, reached from the other
   * side, measured in the running app. The explicit re-check re-reads the list
   * itself (`onRechecked`), so for an agent it is the one call rather than both.
   */
  const refreshAfterSetupChange = useCallback(
    (providerName: string) => {
      if (AI_AGENT_PROVIDER_IDS.includes(providerName)) {
        void recheckAgents(false);
      } else {
        refreshProviders?.();
      }
    },
    [recheckAgents, refreshProviders]
  );
  const agentsByProviderId = useMemo(() => {
    const map = new Map<string, CodingAgentAvailability>();
    for (const agent of agentControls.agents ?? []) map.set(agent.providerId, agent);
    return map;
  }, [agentControls.agents]);
  const hasSubscriptionReadyAgent = (agentControls.agents ?? []).some(
    (agent) => agent.auth.state === 'signed_in_subscription'
  );

  const groups = useMemo(
    () => getOrderedProviderGroups(Array.isArray(providers) ? providers : []),
    [providers]
  );

  /**
   * The default is computed once and then owned by the user.
   *
   * ⚠ Lazy `useState`, not a `useEffect` that re-selects: the agent probe
   * resolves a beat after mount, and re-running the rule then would yank a user
   * who had already clicked a tab onto a different one. The initial value is
   * therefore computed *without* the probe, and the one thing the probe is
   * allowed to do is settle the very first choice, before any interaction —
   * which `hasChosen` below gates.
   */
  const [chosenTab, setChosenTab] = useState<ProviderGroupKey | null>(null);
  const computedTab = defaultCatalogTab({
    groups,
    configuredProvider,
    hasSubscriptionReadyAgent,
    routeHint: initialTab,
  });

  /**
   * A held catalog (`holdUntilSettled`) opens once, at rest, on the tab its inputs settle on.
   *
   * The probe only gets a say when it could change the answer: a route hint or a bound provider
   * outranks it (rules 1 and 2 of {@link defaultCatalogTab}), so with either of those the catalog
   * does not wait for two CLIs to start. Once settled the tab is frozen in `settledTab`, so a late
   * probe (one that outlived {@link AGENT_PROBE_WAIT_MS}) or a refreshed provider list never moves
   * it; the person's own click (`chosenTab`) still does.
   */
  const [probeWaitOver, setProbeWaitOver] = useState(false);
  useEffect(() => {
    if (!holdUntilSettled) return;
    const timer = window.setTimeout(() => setProbeWaitOver(true), AGENT_PROBE_WAIT_MS);
    return () => window.clearTimeout(timer);
  }, [holdUntilSettled]);
  const ownsBoundProvider =
    !!configuredProvider &&
    groups.some((group) =>
      group.providers.some((provider) => provider.name === configuredProvider)
    );
  const probeCanDecide = tabFromHint(initialTab) === null && !ownsBoundProvider;
  const settling =
    loading ||
    (holdUntilSettled &&
      (configuredProvider === undefined ||
        (probeCanDecide && agentControls.isChecking && !probeWaitOver)));
  const [settledTab, setSettledTab] = useState<ProviderGroupKey | null>(null);
  useEffect(() => {
    if (holdUntilSettled && !settling && settledTab === null) setSettledTab(computedTab);
  }, [computedTab, holdUntilSettled, settledTab, settling]);
  const activeTab = chosenTab ?? settledTab ?? computedTab;

  // First run opens the Llama Server row by default, and that panel starts on "Checking Llama
  // Server…" until its status arrives. Asked here, while the catalog is still a skeleton, the
  // status is usually in by the time the panel mounts, so the panel opens on its content. A read
  // (`GET /llamacpp/status`), never a start: nothing is downloaded or run.
  const hasLlamaServer =
    Array.isArray(providers) && providers.some((provider) => provider.name === 'llamacpp');
  useEffect(() => {
    if (!isOnboarding || !hasLlamaServer) return;
    if (llamaServerStore.getSnapshot().status !== null) return;
    llamaServerStore.refresh().catch(() => {
      // The panel asks again when it mounts and says so if it fails.
    });
  }, [hasLlamaServer, isOnboarding]);

  const [openRow, setOpenRow] = useState<string | null>(() => {
    // The recommended local provider opens by default in onboarding: it is the
    // zero-setup path, and an accordion that starts entirely closed reads as an
    // empty screen on the one page a first-run user has no context for.
    return mode === 'onboarding' ? 'llamacpp' : null;
  });
  const toggleRow = useCallback(
    (name: string) => setOpenRow((current) => (current === name ? null : name)),
    []
  );

  const openModal = useCallback(
    (provider: ProviderDetails) => setConfiguringProvider(provider),
    []
  );

  const configureProviderViaModal = useCallback(
    async (provider: ProviderDetails) => {
      if (provider.provider_type === 'Custom' || provider.provider_type === 'Declarative') {
        const result = await getCustomProvider({ path: { id: provider.name }, throwOnError: true });

        if (result.data) {
          setEditingProvider({
            id: provider.name,
            config: result.data.config,
            isEditable: result.data.is_editable,
            isConfigured: provider.is_configured,
          });
          setShowCustomProviderModal(true);
        }
      } else {
        openModal(provider);
      }
    },
    [openModal]
  );

  const handleUpdateCustomProvider = useCallback(
    async (data: UpdateCustomProviderRequest) => {
      if (!editingProvider) return;

      await updateCustomProvider({
        path: { id: editingProvider.id },
        body: data,
        // An update that moves the provider's URL keeps its saved key, so the
        // daemon asks for the proof that a person made it.
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const providerId = editingProvider.id;
      setShowCustomProviderModal(false);
      setEditingProvider(null);
      refreshProviders?.();
      setSwitchModelProvider(providerId);
      setShowSwitchModelModal(true);
    },
    [editingProvider, refreshProviders]
  );

  const handleCreateCustomProvider = useCallback(
    async (data: UpdateCustomProviderRequest) => {
      const { createCustomProvider } = await import('../../../api');
      await createCustomProvider({ body: data, throwOnError: true });
      setShowCustomProviderModal(false);
      refreshProviders?.();
      setShowSwitchModelModal(true);
    },
    [refreshProviders]
  );

  const handleCloseCustomModal = useCallback(() => {
    setShowCustomProviderModal(false);
    setEditingProvider(null);
  }, []);

  const onCloseProviderConfig = useCallback(() => {
    const closed = configuringProvider?.name;
    setConfiguringProvider(null);
    // Closing is not proof nothing changed: a save whose provider check failed
    // has still written the key before the error dialog appeared.
    if (closed) refreshAfterSetupChange(closed);
    else refreshProviders?.();
  }, [configuringProvider, refreshAfterSetupChange, refreshProviders]);

  const onProviderConfigured = useCallback(
    (provider: ProviderDetails) => {
      setConfiguringProvider(null);
      refreshAfterSetupChange(provider.name);
      setSwitchModelProvider(provider.name);
      setShowSwitchModelModal(true);
    },
    [refreshAfterSetupChange]
  );

  const handleSetView = useCallback(
    (view: View) => {
      setShowSwitchModelModal(false);
      setView?.(view);
    },
    [setView]
  );

  const handleLocalComplete = useCallback(() => {
    refreshProviders?.();
    onLocalComplete?.();
  }, [onLocalComplete, refreshProviders]);

  /** The first-run setup panel a row opens, or `null` when it has none. */
  const setupPanelFor = (provider: ProviderDetails): React.ReactNode => {
    if (!isOnboarding) return null;
    // The row is the selector (FR-R1-6): an institutional gateway that is not set up opens its own
    // form in place, for its own gateway, instead of a dialog beside a second form below the rows.
    const gateway = INSTITUTIONAL_SETUP_FLAVOR[provider.name];
    if (gateway && !provider.is_configured) {
      return (
        <InstitutionalSetupCard
          chrome="bare"
          flavor={gateway}
          onSuccess={handleProviderReady}
          onStartTesting={onStartTesting}
        />
      );
    }
    switch (ONBOARDING_SETUP[provider.name]) {
      case 'llamacpp':
        return <LlamaServerInlineCard chrome="bare" onSuccess={handleLocalComplete} />;
      case 'ollama':
        return <OllamaInlineCard chrome="bare" onSuccess={handleLocalComplete} />;
      default:
        return null;
    }
  };

  const renderRow = (provider: ProviderDetails) => {
    const agent = agentsByProviderId.get(provider.name);
    const setupPanel = setupPanelFor(provider);
    // A coding-agent row is an accordion on BOTH surfaces: its content is a live
    // status and the guidance for it, which a modal cannot carry and which a
    // settings user needs exactly as much as a first-run user does.
    const expandable = agent !== undefined || setupPanel !== null;
    const retiredEndpointNotice =
      provider.name === 'azure_openai' && azureOnRetiredEndpoint ? (
        <Note
          tone="warning"
          role="status"
          testId="azure-retired-endpoint-notice"
          className="mx-3 mb-2"
          action={
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void configureProviderViaModal(provider)}
            >
              {catalogCopy.configure}
            </Button>
          }
        >
          {RETIRED_AZURE_ENDPOINT_NOTICE}
        </Note>
      ) : null;

    // One status per row (principle 4, V4-26). A configured agent that is ready is said by
    // "Configured" alone: its pill ("Ready · signed in on your subscription") would be the same
    // news twice. Every state that needs attention keeps its pill, and so does a ready agent that
    // has not been set up yet, because the pill is what invites the "Use" in its panel.
    const agentPill = agent ? pillFor(agent.auth) : null;
    const showAgentPill =
      agentPill !== null && !(provider.is_configured && agentPill.tone === 'success');

    return (
      <React.Fragment key={provider.name}>
        <ProviderCard
          key={provider.name}
          provider={provider}
          onConfigure={() => void configureProviderViaModal(provider)}
          onLaunch={() => {
            setSwitchModelProvider(provider.name);
            setShowSwitchModelModal(true);
          }}
          isOnboarding={isOnboarding}
          expandable={expandable}
          expanded={expandable && openRow === provider.name}
          onToggle={() => toggleRow(provider.name)}
          statusSlot={
            showAgentPill ? (
              <StatusPill tone={agentPill.tone}>{agentPill.label}</StatusPill>
            ) : undefined
          }
        >
          {agent ? (
            <div className="space-y-2">
              <CodingAgentProvenance agent={agent} />
              <CodingAgentBody agent={agent} controls={agentControls} />
            </div>
          ) : (
            setupPanel
          )}
        </ProviderCard>
        {retiredEndpointNotice}
      </React.Fragment>
    );
  };

  const renderSection = (group: OrderedProviderGroup, section: OrderedProviderSection) => {
    const isApiSection = group.key === 'commercial' && section.key === 'api';

    const rows = (
      <>
        {/*
          Issue #56, DR-17 requirement 3. The API-provider section — and only it —
          carries the standing one-line disclosure of what a model there can
          reach. The words come from the daemon, never from a literal here: a
          second copy of a sentence is a sentence that goes stale in one of its
          two homes and stays wrong.
        */}
        {isApiSection && (
          <NonPrivateModelDisclosureNote className="px-3 pb-2 text-supporting text-text-muted" />
        )}
        {/*
          The fastest path for someone who already holds a key, at the TOP of the
          section it belongs to rather than at the bottom of the page: paste it
          and the provider is detected, with no need to know which of twenty-odd
          rows is theirs.
        */}
        {isOnboarding && isApiSection && (
          <div className="br-provider-inset mb-4">
            <CommercialSetupCard
              chrome="bare"
              onSuccess={handleCommercialDetected}
              onStartTesting={onStartTesting}
            />
          </div>
        )}
        {section.providers.map(renderRow)}
        {isApiSection && <AddCustomProviderRow onClick={() => setShowCustomProviderModal(true)} />}
      </>
    );

    return (
      <div key={section.key} data-testid={`catalog-section-${section.key}`}>
        {section.label ? (
          <SettingSection title={section.label} help={section.help}>
            {rows}
          </SettingSection>
        ) : (
          <div className="biorouter-settings-list">{rows}</div>
        )}
      </div>
    );
  };

  const renderPanel = (group: OrderedProviderGroup) => (
    <div>
      {/* §14.5: why this tab has the tier it has. PRIVACY information, so it is a visible line,
          never an InfoTip, and it is the only line a panel opens with: the tab already names
          the group, and this line starts with the same words ("Private because…", "Public."). */}
      <p className="br-provider-tier-note text-supporting text-text-muted">{group.note}</p>

      {group.key === 'commercial' && agentControls.loadError && !agentControls.agents && (
        <div className="br-provider-inset mb-4">
          <CodingAgentLoadError message={agentControls.loadError} controls={agentControls} />
        </div>
      )}

      {group.sections.map((section) => renderSection(group, section))}

      {group.providers.length === 0 && group.key !== 'commercial' && (
        <p
          className="px-3 text-supporting text-text-muted"
          data-testid={`catalog-empty-${group.key}`}
        >
          {catalogCopy.emptyTab}
        </p>
      )}

      {/*
        Where an institution's gateway comes from, said once per catalog rather
        than implied by an empty tab. It is the honest answer to "why is my
        university not listed?", and it names the only two outcomes available:
        Biorouter recognises the endpoint, or it does not and the account is
        treated as public. A quiet line with the answer one hover away; the tab's own note
        already says the public half in full.
      */}
      {group.key === 'institutional' && (
        <div className="br-provider-other-institutions" data-testid="other-institutions-note">
          <span className="text-secondary text-text-muted">{catalogCopy.otherInstitutions}</span>
          <InfoTip label={catalogCopy.otherInstitutions} help={catalogCopy.otherInstitutionsHelp} />
        </div>
      )}
    </div>
  );

  const initialData = editingProvider && {
    engine: editingProvider.config.engine,
    display_name: editingProvider.config.display_name,
    api_url: editingProvider.config.base_url,
    api_key: '',
    models: editingProvider.config.models.map((m) => m.name),
    supports_streaming: editingProvider.config.supports_streaming ?? true,
  };
  const editable = editingProvider ? editingProvider.isEditable : true;
  // Named, and honest about first-time setup (W2-PRV-13): it read
  // "Configure  Provider" for every built-in declarative provider, set up or not.
  const editingName = editingProvider?.config.display_name || 'provider';
  const customModalTitle = !editingProvider
    ? customProviderCopy.add
    : !editingProvider.isConfigured
      ? customProviderCopy.setUp(editingName)
      : editable
        ? customProviderCopy.edit(editingName)
        : customProviderCopy.configure(editingName);

  const tabsList = (
    <TabsList
      aria-label={catalogCopy.tabsLabel}
      className={tabsHost === undefined ? 'mb-4' : 'br-provider-band-tabs border-b-0'}
    >
      {groups.map((group) => (
        <TabsTrigger
          key={group.key}
          value={group.key}
          className="gap-2"
          data-testid={`catalog-tab-${group.key}`}
        >
          <span
            className={`size-1.5 ${group.accentClassName} flex-shrink-0 rounded-full`}
            aria-hidden
          />
          {group.tabLabel}
        </TabsTrigger>
      ))}
    </TabsList>
  );

  return (
    <>
      {/*
        SD-1, stated once at the top of the page rather than on twenty rows.
        Storing a provider's API key still works in a browser — a secret is not a
        capability key — but the step *after* it, choosing which model becomes
        the default, is refused. Saying so here means the user learns it before
        pasting a key rather than after.
      */}
      <HostManagedModelNote className="mb-6" />

      {settling ? (
        // The quiet skeleton at row height: the tier line, then three rows. Nothing in it can be
        // read as an answer (no "Nothing here yet.", no tab), so nothing on the first screen is
        // contradicted a second later (FR-R1-1).
        <div
          role="status"
          aria-label={providerPageCopy.loading}
          className="br-provider-skeleton"
          data-testid="provider-catalog-loading"
        >
          <Skeleton className="h-4 w-64" />
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-full" />
        </div>
      ) : (
        <Tabs
          value={activeTab}
          onValueChange={(value) => setChosenTab(value as ProviderGroupKey)}
          className="br-provider-catalog br-rows-flat flex flex-col"
        >
          {/* The shared underline tabs. The dot is the tier (§14.5), so it stays: it is the only
            colour that says private from public at a glance. In the page band they sit on the
            title's line, as Settings' do, and the band's own hairline is the one rule. */}
          {tabsHost === undefined ? tabsList : tabsHost ? createPortal(tabsList, tabsHost) : null}

          {/* Radix makes every panel a Tab stop, and this one draws no focus edge, so Tab from
            the tab list would land on a panel that shows nothing. A panel with rows hands that
            stop to its first control (V4-05, request WS-PRIMITIVES-3); one with no control to
            hand it to (an empty institutional tab) keeps it, so a keyboard user can still reach
            the line that explains why. */}
          {groups.map((group) => (
            <TabsContent
              key={group.key}
              value={group.key}
              className="mt-0"
              {...(group.providers.length > 0 ? { tabIndex: -1 } : {})}
            >
              {renderPanel(group)}
            </TabsContent>
          ))}
        </Tabs>
      )}

      <ModalShell
        open={showCustomProviderModal}
        onOpenChange={(open) => {
          if (!open) handleCloseCustomModal();
        }}
        size="md"
        purpose="form"
        scrollBody
        title={customModalTitle}
      >
        <CustomProviderForm
          initialData={initialData}
          isEditable={editable}
          hasSavedKey={editingProvider?.isConfigured ?? true}
          onSubmit={editingProvider ? handleUpdateCustomProvider : handleCreateCustomProvider}
          onCancel={handleCloseCustomModal}
        />
      </ModalShell>
      {configuringProvider && (
        <ProviderConfigurationModal
          provider={configuringProvider}
          onClose={onCloseProviderConfig}
          onConfigured={onProviderConfigured}
        />
      )}
      {showSwitchModelModal && (
        <SwitchModelModal
          sessionId={chatSessionId}
          privacyTier={chatPrivacyTier}
          onChooseForUnsentChat={unsentChatTabId ? holdForUnsentChat : undefined}
          unsentChatTabId={unsentChatTabId}
          onClose={() => setShowSwitchModelModal(false)}
          setView={handleSetView}
          onModelSelected={onModelSelected}
          initialProvider={switchModelProvider}
          initialModel={switchModelInitial}
          titleOverride={catalogCopy.chooseModel}
        />
      )}
    </>
  );
}
