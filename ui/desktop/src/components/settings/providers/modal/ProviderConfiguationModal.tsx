import { useState } from 'react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../../ui/dialog';
import DefaultProviderSetupForm, {
  ConfigInput,
} from './subcomponents/forms/DefaultProviderSetupForm';
import ProviderSetupActions from './subcomponents/ProviderSetupActions';
import ProviderLogo from './subcomponents/ProviderLogo';
import { SecureStorageNotice } from './subcomponents/SecureStorageNotice';
import { providerConfigSubmitHandler } from './subcomponents/handlers/DefaultSubmitHandler';
import { useConfig } from '../../../ConfigContext';
import { useModelAndProvider } from '../../../ModelAndProviderContext';
import { AlertTriangle } from '../../../icons/app-icons';
import { ProviderDetails, removeCustomProvider } from '../../../../api';
import { Button } from '../../../../components/ui/button';

interface ProviderConfigurationModalProps {
  provider: ProviderDetails;
  onClose: () => void;
  onConfigured?: (provider: ProviderDetails) => void;
}

export default function ProviderConfigurationModal({
  provider,
  onClose,
  onConfigured,
}: ProviderConfigurationModalProps) {
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});
  const { upsert, remove } = useConfig();
  const { getCurrentModelAndProvider } = useModelAndProvider();
  const [configValues, setConfigValues] = useState<Record<string, ConfigInput>>({});
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(false);
  const [isActiveProvider, setIsActiveProvider] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const requiredParameters = provider.metadata.config_keys.filter(
    (param) => param.required === true
  );

  const isConfigured = provider.is_configured;
  // Something is SAVED for this provider even when it cannot run — a coding
  // agent whose command key names a CLI that is not installed is served
  // `is_configured: false` with a reason. Removing that saved key is still the
  // user's to do; keying "Remove" on `is_configured` alone would strand it.
  const hasSavedSetup = isConfigured || Boolean(provider.unavailable_reason);
  const headerText = showDeleteConfirmation
    ? `Delete configuration for ${provider.metadata.display_name}`
    : `Configure ${provider.metadata.display_name}`;

  const descriptionText = showDeleteConfirmation
    ? isActiveProvider
      ? `This provider is in use. Switch to a different model first.`
      : 'This will permanently delete the current provider configuration.'
    : `Add your API key(s) for this provider to integrate into Biorouter`;

  const handleSubmitForm = async (e: React.FormEvent) => {
    e.preventDefault();

    setValidationErrors({});

    const parameters = provider.metadata.config_keys || [];
    const errors: Record<string, string> = {};

    parameters.forEach((parameter) => {
      if (
        parameter.required &&
        !configValues[parameter.name]?.value &&
        !configValues[parameter.name]?.serverValue
      ) {
        errors[parameter.name] = `${parameter.name} is required`;
      }
    });

    if (Object.keys(errors).length > 0) {
      setValidationErrors(errors);
      return;
    }

    const toSubmit = Object.fromEntries(
      Object.entries(configValues)
        .filter(([_k, entry]) => !!entry.value)
        .map(([k, entry]) => [k, entry.value || ''])
    );

    try {
      await providerConfigSubmitHandler(upsert, provider, toSubmit);
      if (onConfigured) {
        onConfigured(provider);
      } else {
        onClose();
      }
    } catch (error) {
      setError(`${error}`);
    }
  };

  const handleCancel = () => {
    onClose();
  };

  const handleDelete = async () => {
    try {
      const providerModel = await getCurrentModelAndProvider();
      if (provider.name === providerModel.provider) {
        setIsActiveProvider(true);
        setShowDeleteConfirmation(true);
        return;
      }
    } catch (error) {
      console.error('Failed to check current provider:', error);
    }

    setIsActiveProvider(false);
    setShowDeleteConfirmation(true);
  };

  const handleConfirmDelete = async () => {
    if (isActiveProvider) {
      return;
    }

    const isCustomProvider = provider.provider_type === 'Custom';

    if (isCustomProvider) {
      await removeCustomProvider({
        path: { id: provider.name },
      });
    } else {
      const params = provider.metadata.config_keys;
      for (const param of params) {
        await remove(param.name, param.secret);
      }
    }

    onClose();
  };

  const getModalIcon = () => {
    if (showDeleteConfirmation) {
      return (
        <AlertTriangle
          className={isActiveProvider ? 'text-text-warning' : 'text-text-danger'}
          size={24}
        />
      );
    }
    return <ProviderLogo providerName={provider.name} />;
  };

  return (
    <>
      <Dialog open={!!error} onOpenChange={(open) => !open && setError(null)}>
        <DialogContent className="sm:max-w-[600px] max-h-[90vh] overflow-y-auto">
          <DialogTitle className="flex items-center gap-2">Error</DialogTitle>
          <DialogDescription className="text-inherit text-base">
            There was an error checking this provider configuration.
          </DialogDescription>
          <pre className="ml-2">{error}</pre>
          <div>Check your configuration again to use this provider.</div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setError(null)}>
              Go Back
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={!error} onOpenChange={(open) => !open && onClose()}>
        <DialogContent className="sm:max-w-[600px] max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              {getModalIcon()}
              {headerText}
            </DialogTitle>
            <DialogDescription>{descriptionText}</DialogDescription>
          </DialogHeader>

          {!showDeleteConfirmation && (
            <div className="py-2">
              <DefaultProviderSetupForm
                configValues={configValues}
                setConfigValues={setConfigValues}
                provider={provider}
                validationErrors={validationErrors}
              />
              {requiredParameters.length > 0 &&
                provider.metadata.config_keys &&
                provider.metadata.config_keys.length > 0 && <SecureStorageNotice />}
            </div>
          )}

          <DialogFooter className="border-t border-border-subtle pt-4 mt-2">
            <ProviderSetupActions
              requiredParameters={requiredParameters}
              onCancel={handleCancel}
              onSubmit={handleSubmitForm}
              onDelete={handleDelete}
              showDeleteConfirmation={showDeleteConfirmation}
              onConfirmDelete={handleConfirmDelete}
              onCancelDelete={() => {
                setIsActiveProvider(false);
                setShowDeleteConfirmation(false);
              }}
              canDelete={hasSavedSetup && !isActiveProvider}
              providerName={provider.metadata.display_name}
              isActiveProvider={isActiveProvider}
            />
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
