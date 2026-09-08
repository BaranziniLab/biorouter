import React, { useCallback, useMemo, useState } from 'react';
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
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../../ui/dialog';
import CustomProviderForm from './modal/subcomponents/forms/CustomProviderForm';
import { SwitchModelModal } from '../models/subcomponents/SwitchModelModal';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../ui/tabs';
import type { View } from '../../../utils/navigationUtils';
import {
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
  INSTITUTIONAL_SETUP_PROVIDER_IDS,
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
}

/**
 * The provider ids that open a bespoke first-run setup panel, and which one.
 *
 * ⚠ Wiring, not taxonomy — see the class note above. `institutional` is handled
 * per *section* rather than per row, because that card configures a family of
 * gateways behind its own flavour toggle and would otherwise be rendered twice
 * inside one institution.
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
      data-testid="add-custom-provider-card"
      onClick={onClick}
      className="w-full flex items-center gap-3 py-3 px-4 rounded-container
        cursor-pointer tint-interactive
        transition-colors text-left"
    >
      <div className="w-8 h-8 rounded-element flex items-center justify-center flex-shrink-0">
        <Plus className="w-4 h-4 text-text-muted" />
      </div>
      <p className="text-label text-text-muted">Add Custom Provider</p>
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
}: ProviderCatalogProps) {
  const isOnboarding = mode === 'onboarding';

  const [configuringProvider, setConfiguringProvider] = useState<ProviderDetails | null>(null);
  const [showCustomProviderModal, setShowCustomProviderModal] = useState(false);
  const [showSwitchModelModal, setShowSwitchModelModal] = useState(false);
  const [switchModelProvider, setSwitchModelProvider] = useState<string | null>(null);
  const [switchModelInitial, setSwitchModelInitial] = useState<string | null>(null);
  const [editingProvider, setEditingProvider] = useState<{
    id: string;
    config: DeclarativeProviderConfig;
    isEditable: boolean;
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

  /**
   * ⚠ **Mounted once, at the catalog — never per row.** `GET
   * /coding_agents/status` spawns both vendor CLIs, so one probe per agent row
   * would fork four processes on every render of this panel. `useCodingAgents`
   * fetches on mount and on an explicit "Check again" only.
   */
  const agentControls = useCodingAgents(handleProviderReady);
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
  const activeTab = chosenTab ?? computedTab;

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
    setConfiguringProvider(null);
    refreshProviders?.();
  }, [refreshProviders]);

  const onProviderConfigured = useCallback(
    (provider: ProviderDetails) => {
      setConfiguringProvider(null);
      refreshProviders?.();
      setSwitchModelProvider(provider.name);
      setShowSwitchModelModal(true);
    },
    [refreshProviders]
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

    return (
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
          agent ? (
            <StatusPill tone={pillFor(agent.auth).tone}>{pillFor(agent.auth).label}</StatusPill>
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
    );
  };

  const renderSection = (group: OrderedProviderGroup, section: OrderedProviderSection) => {
    const isApiSection = group.key === 'commercial' && section.key === 'api';
    const isInstitutionSection =
      group.key === 'institutional' &&
      section.key !== 'unaffiliated' &&
      section.providers.length > 0;
    // Only offer the institutional setup form where it actually configures one of
    // the section's providers — derived from that card's own declaration, so a
    // second institution's gateways never get a form written for someone else's.
    const hostsInstitutionalSetup =
      isOnboarding &&
      isInstitutionSection &&
      section.providers.some((provider) =>
        INSTITUTIONAL_SETUP_PROVIDER_IDS.includes(provider.name)
      );

    return (
      <div key={section.key} data-testid={`catalog-section-${section.key}`}>
        {section.label && (
          <>
            <h3 className="text-caps text-text-muted mb-1">{section.label}</h3>
            {section.note && <p className="text-supporting text-text-muted mb-3">{section.note}</p>}
          </>
        )}
        {/*
          Issue #56, DR-17 requirement 3. The API-provider section — and only it —
          carries the standing one-line disclosure of what a model there can
          reach. The words come from the daemon, never from a literal here: a
          second copy of a sentence is a sentence that goes stale in one of its
          two homes and stays wrong.
        */}
        {isApiSection && (
          <NonPrivateModelDisclosureNote className="text-supporting text-text-muted mb-3" />
        )}
        {/*
          The fastest path for someone who already holds a key, at the TOP of the
          section it belongs to rather than at the bottom of the page: paste it
          and the provider is detected, with no need to know which of twenty-odd
          rows is theirs.
        */}
        {isOnboarding && isApiSection && (
          <div className="mb-4">
            <CommercialSetupCard
              chrome="bare"
              onSuccess={handleCommercialDetected}
              onStartTesting={onStartTesting}
            />
          </div>
        )}
        <div className="divide-y divide-border-subtle">
          {section.providers.map(renderRow)}
          {isApiSection && (
            <AddCustomProviderRow onClick={() => setShowCustomProviderModal(true)} />
          )}
        </div>
        {hostsInstitutionalSetup && (
          <div className="mt-4 border-t border-border-subtle pt-4">
            <InstitutionalSetupCard
              chrome="bare"
              onSuccess={handleProviderReady}
              onStartTesting={onStartTesting}
            />
          </div>
        )}
      </div>
    );
  };

  const renderPanel = (group: OrderedProviderGroup) => (
    <div className="space-y-6">
      <div>
        <h2 className="text-caps text-text-muted mb-1 flex items-center gap-2">
          <span className={`w-1.5 h-1.5 ${group.accentClassName} rounded-full flex-shrink-0`} />
          {group.label}
        </h2>
        <p className="text-supporting text-text-muted">{group.note}</p>
      </div>

      {group.key === 'commercial' && agentControls.loadError && !agentControls.agents && (
        <CodingAgentLoadError message={agentControls.loadError} controls={agentControls} />
      )}

      {group.sections.map((section) => renderSection(group, section))}

      {group.providers.length === 0 && group.key !== 'commercial' && (
        <p className="text-supporting text-text-muted" data-testid={`catalog-empty-${group.key}`}>
          Nothing here yet.
        </p>
      )}

      {/*
        Where an institution's gateway comes from, said once per catalog rather
        than implied by an empty tab. It is the honest answer to "why is my
        university not listed?", and it names the only two outcomes available:
        Biorouter recognises the endpoint, or it does not and the account is
        treated as public.
      */}
      {group.key === 'institutional' && (
        <div
          className="rounded-container border border-border-subtle px-4 py-3"
          data-testid="other-institutions-note"
        >
          <h3 className="text-caps text-text-muted mb-1">Other institutions</h3>
          <p className="text-supporting text-text-muted">
            An institution&apos;s gateway appears here once Biorouter recognises its endpoint as
            private. Anything else can still be added under Public → Add Custom Provider, and is
            treated as public: Biorouter cannot verify where that endpoint points, so it gets none
            of the private tier&apos;s protections.
          </p>
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
  const customModalTitle =
    (editingProvider ? (editable ? 'Edit' : 'Configure') : 'Add') + '  Provider';

  return (
    <>
      {/*
        SD-1, stated once at the top of the page rather than on twenty rows.
        Storing a provider's API key still works in a browser — a secret is not a
        capability key — but the step *after* it, choosing which model becomes
        the default, is refused. Saying so here means the user learns it before
        pasting a key rather than after.
      */}
      <HostManagedModelNote className="mb-6 rounded-container border border-border-subtle bg-background-default px-4 py-3 text-xs leading-relaxed text-text-muted" />

      <Tabs
        value={activeTab}
        onValueChange={(value) => setChosenTab(value as ProviderGroupKey)}
        className="flex flex-col"
      >
        {/* The same list, triggers and class Settings uses — one tab vocabulary
            across the app, not a second one that happens to look similar. */}
        <TabsList className="biorouter-settings-tabs justify-start w-fit border-b-0 mb-5">
          {groups.map((group) => (
            <TabsTrigger
              key={group.key}
              value={group.key}
              className="flex gap-2 text-label"
              data-testid={`catalog-tab-${group.key}`}
            >
              <span
                className={`w-1.5 h-1.5 ${group.accentClassName} rounded-full flex-shrink-0`}
                aria-hidden
              />
              {group.tabLabel}
            </TabsTrigger>
          ))}
        </TabsList>

        {groups.map((group) => (
          <TabsContent key={group.key} value={group.key} className="mt-0">
            {renderPanel(group)}
          </TabsContent>
        ))}
      </Tabs>

      <Dialog open={showCustomProviderModal} onOpenChange={handleCloseCustomModal}>
        <DialogContent aria-describedby={undefined} className="sm:max-w-[600px]">
          <DialogHeader>
            <DialogTitle>{customModalTitle}</DialogTitle>
          </DialogHeader>
          <CustomProviderForm
            initialData={initialData}
            isEditable={editable}
            onSubmit={editingProvider ? handleUpdateCustomProvider : handleCreateCustomProvider}
            onCancel={handleCloseCustomModal}
          />
        </DialogContent>
      </Dialog>
      {configuringProvider && (
        <ProviderConfigurationModal
          provider={configuringProvider}
          onClose={onCloseProviderConfig}
          onConfigured={onProviderConfigured}
        />
      )}
      {showSwitchModelModal && (
        <SwitchModelModal
          sessionId={null}
          onClose={() => setShowSwitchModelModal(false)}
          setView={handleSetView}
          onModelSelected={onModelSelected}
          initialProvider={switchModelProvider}
          initialModel={switchModelInitial}
          titleOverride="Choose Model"
        />
      )}
    </>
  );
}
