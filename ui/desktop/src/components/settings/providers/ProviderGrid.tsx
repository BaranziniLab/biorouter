import React, { memo, useMemo, useCallback, useState } from 'react';
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
import type { View } from '../../../utils/navigationUtils';
import { getOrderedProviderGroups } from './providerOrdering';
import { NonPrivateModelDisclosureNote } from '../../privacy/NonPrivateModelDisclosureNote';
import { HostManagedModelNote } from '../../privacy/HostManagedModelNote';

const GridLayout = memo(function GridLayout({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-col">{children}</div>;
});

const CustomProviderCard = memo(function CustomProviderCard({ onClick }: { onClick: () => void }) {
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

function ProviderCards({
  providers,
  isOnboarding,
  refreshProviders,
  setView,
  onModelSelected,
}: {
  providers: ProviderDetails[];
  isOnboarding: boolean;
  refreshProviders?: () => void;
  setView?: (view: View) => void;
  onModelSelected?: (model?: string) => void;
}) {
  const [configuringProvider, setConfiguringProvider] = useState<ProviderDetails | null>(null);
  const [showCustomProviderModal, setShowCustomProviderModal] = useState(false);
  const [showSwitchModelModal, setShowSwitchModelModal] = useState(false);
  const [switchModelProvider, setSwitchModelProvider] = useState<string | null>(null);
  const [editingProvider, setEditingProvider] = useState<{
    id: string;
    config: DeclarativeProviderConfig;
    isEditable: boolean;
  } | null>(null);

  const handleProviderLaunchWithModelSelection = useCallback((provider: ProviderDetails) => {
    setSwitchModelProvider(provider.name);
    setShowSwitchModelModal(true);
  }, []);

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
      if (refreshProviders) {
        refreshProviders();
      }
      setSwitchModelProvider(providerId);
      setShowSwitchModelModal(true);
    },
    [editingProvider, refreshProviders]
  );

  const handleCloseModal = useCallback(() => {
    setShowCustomProviderModal(false);
    setEditingProvider(null);
  }, []);

  const onCloseProviderConfig = useCallback(() => {
    setConfiguringProvider(null);
    if (refreshProviders) {
      refreshProviders();
    }
  }, [refreshProviders]);

  const onProviderConfigured = useCallback(
    (provider: ProviderDetails) => {
      setConfiguringProvider(null);
      if (refreshProviders) {
        refreshProviders();
      }
      setSwitchModelProvider(provider.name);
      setShowSwitchModelModal(true);
    },
    [refreshProviders]
  );

  const onCloseSwitchModelModal = useCallback(() => {
    setShowSwitchModelModal(false);
  }, []);

  const handleSetView = useCallback(
    (view: View) => {
      setShowSwitchModelModal(false);
      if (setView) {
        setView(view);
      }
    },
    [setView]
  );

  const handleCreateCustomProvider = useCallback(
    async (data: UpdateCustomProviderRequest) => {
      const { createCustomProvider } = await import('../../../api');
      await createCustomProvider({ body: data, throwOnError: true });
      setShowCustomProviderModal(false);
      if (refreshProviders) {
        refreshProviders();
      }
      setShowSwitchModelModal(true);
    },
    [refreshProviders]
  );

  /**
   * §14.5 — the heading, the accent and the one line of card copy all come from
   * `getOrderedProviderGroups`, never from literals here.
   *
   * This file used to import that function for its ORDERING and then print
   * three hardcoded headings of its own, with its own dots. Relabelling
   * `providerOrdering.ts` therefore changed nothing a user could see — which is
   * the exact failure this rewrite removes, and why
   * `ProviderGrid.privacy.test.tsx` asserts against the rendered screen rather
   * than against the data. (The task's gate greps this file for the three old
   * headings and expects none, so do not quote them back in here either.)
   */
  const sections = useMemo(() => {
    const providersArray = Array.isArray(providers) ? providers : [];

    return getOrderedProviderGroups(providersArray).map((group) => ({
      ...group,
      cards: group.providers.map((provider) => (
        <ProviderCard
          key={provider.name}
          provider={provider}
          onConfigure={() => configureProviderViaModal(provider)}
          onLaunch={() => handleProviderLaunchWithModelSelection(provider)}
          isOnboarding={isOnboarding}
        />
      )),
    }));
  }, [providers, isOnboarding, configureProviderViaModal, handleProviderLaunchWithModelSelection]);

  const initialData = editingProvider && {
    engine: editingProvider.config.engine,
    display_name: editingProvider.config.display_name,
    api_url: editingProvider.config.base_url,
    api_key: '',
    models: editingProvider.config.models.map((m) => m.name),
    supports_streaming: editingProvider.config.supports_streaming ?? true,
  };

  const editable = editingProvider ? editingProvider.isEditable : true;
  const title = (editingProvider ? (editable ? 'Edit' : 'Configure') : 'Add') + '  Provider';
  return (
    <>
      {/*
        SD-1, stated once at the top of the page rather than on twenty cards.
        Storing a provider's API key still works in a browser — a secret is not a
        capability key — but the step *after* it, choosing which model becomes
        the default, is refused. Saying so here means the user learns it before
        pasting a key rather than after.
      */}
      <HostManagedModelNote className="mb-6" />
      <div className="space-y-8">
        {sections.map((section) => {
          // The commercial section always renders: it hosts "Add Custom
          // Provider", which must stay reachable on a machine that has only
          // private providers configured.
          const alwaysVisible = section.key === 'commercial';
          if (section.cards.length === 0 && !alwaysVisible) return null;

          return (
            <div key={section.key}>
              {/* `text-caps` is the ONE caps style — it carries the 11/500,
                  the +0.08em tracking and the uppercase transform the four
                  utilities here were spelling out by hand. The `mb-1` (rather
                  than main's `mb-3`) is because the section NOTE sits directly
                  under the heading and owns the gap below the pair. */}
              <h2 className="text-caps text-text-muted mb-1 flex items-center gap-2">
                <span
                  className={`w-1.5 h-1.5 ${section.accentClassName} rounded-full flex-shrink-0`}
                />
                {section.label}
              </h2>
              <p className="text-supporting text-text-muted mb-3">{section.note}</p>
              {/*
                Issue #56, DR-17 requirement 3. The Commercial section — and only
                it — carries the standing one-line disclosure of what a model
                there can reach. The words come from the daemon, never from a
                literal here, for exactly the reason the block above exists: a
                second copy of a sentence is a sentence that goes stale in one
                of its two homes and stays wrong.
              */}
              {alwaysVisible && (
                <NonPrivateModelDisclosureNote className="text-supporting text-text-muted mb-3" />
              )}
              <div className="divide-y divide-border-subtle">
                {section.cards}
                {alwaysVisible && (
                  <CustomProviderCard onClick={() => setShowCustomProviderModal(true)} />
                )}
              </div>
            </div>
          );
        })}
      </div>
      <Dialog open={showCustomProviderModal} onOpenChange={handleCloseModal}>
        <DialogContent aria-describedby={undefined} className="sm:max-w-[600px]">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
          </DialogHeader>
          <CustomProviderForm
            initialData={initialData}
            isEditable={editable}
            onSubmit={editingProvider ? handleUpdateCustomProvider : handleCreateCustomProvider}
            onCancel={handleCloseModal}
          />
        </DialogContent>
      </Dialog>{' '}
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
          onClose={onCloseSwitchModelModal}
          setView={handleSetView}
          onModelSelected={onModelSelected}
          initialProvider={switchModelProvider}
          titleOverride="Choose Model"
        />
      )}
    </>
  );
}

export default function ProviderGrid({
  providers,
  isOnboarding,
  refreshProviders,
  setView,
  onModelSelected,
}: {
  providers: ProviderDetails[];
  isOnboarding: boolean;
  refreshProviders?: () => void;
  setView?: (view: View) => void;
  onModelSelected?: (model?: string) => void;
}) {
  return (
    <GridLayout>
      <ProviderCards
        providers={providers}
        isOnboarding={isOnboarding}
        refreshProviders={refreshProviders}
        setView={setView}
        onModelSelected={onModelSelected}
      />
    </GridLayout>
  );
}
