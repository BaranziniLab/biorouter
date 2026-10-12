import { useEffect, useRef, useState } from 'react';
import { ModalShell } from '../../../ModalShell';
import DefaultProviderSetupForm, {
  ConfigInput,
  isHostOwnedProviderField,
  providerFieldName,
} from './subcomponents/forms/DefaultProviderSetupForm';
import { hostManagedDestinationRequired } from '../../../privacy/hostManagedModelCopy';
import ProviderSetupActions from './subcomponents/ProviderSetupActions';
import { SecureStorageNotice } from './subcomponents/SecureStorageNotice';
import { providerConfigSubmitHandler } from './subcomponents/handlers/DefaultSubmitHandler';
import { useConfig } from '../../../ConfigContext';
import { useModelAndProvider } from '../../../ModelAndProviderContext';
import { ProviderDetails, removeCustomProvider } from '../../../../api';
import { Button } from '../../../ui/button';
import { InfoTip } from '../../../ui/info-tip';
import { providerSetupCopy } from '../copy';
import { restOnDialog } from '../../models/dialogFocus';
import CodingAgentSetupRecovery from './CodingAgentSetupRecovery';
import {
  CODING_AGENT_ORDER,
  fetchCodingAgentStatus,
  type CodingAgentAvailability,
  type CodingAgentKind,
} from '../../../onboarding/codingAgentStatus';

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
  const [isSubmitting, setIsSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const [checkAttempt, setCheckAttempt] = useState(0);
  const [checkedAgents, setCheckedAgents] = useState<CodingAgentAvailability[] | undefined>();
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);
  const codingAgentKind = CODING_AGENT_ORDER.includes(provider.name as CodingAgentKind)
    ? (provider.name as CodingAgentKind)
    : null;

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
    ? providerSetupCopy.deleteTitle(provider.metadata.display_name)
    : providerSetupCopy.configure(provider.metadata.display_name);

  // A line under the title only for the delete states: they are consequences, said at the moment
  // of decision (principle 2), so they stay visible. A provider with a key field needs no line
  // ("Add your API keys…" restated the field).
  const descriptionText = showDeleteConfirmation
    ? isActiveProvider
      ? providerSetupCopy.inUse
      : providerSetupCopy.deleteConsequence
    : undefined;
  // Why a form has no key field is an explanation, so it is the title's InfoTip, not a line under
  // it: Configure Amazon Bedrock was the one dialog on the first-run path with a subtitle (vision
  // FR-R3-2). Llama Server and Ollama declare no secret and were once asked for "API key(s)"
  // (W2-PRV-13); a coding agent's form is a command path, not a key.
  const titleHelp = showDeleteConfirmation
    ? undefined
    : codingAgentKind
      ? providerSetupCopy.agentIntro(provider.metadata.display_name)
      : provider.metadata.config_keys.some((key) => key.secret)
        ? undefined
        : providerSetupCopy.noKey(provider.metadata.display_name);

  const handleSubmitForm = async (e?: React.SyntheticEvent) => {
    e?.preventDefault();
    if (submittingRef.current) return;

    setValidationErrors({});

    const parameters = provider.metadata.config_keys || [];
    const errors: Record<string, string> = {};

    parameters.forEach((parameter) => {
      if (
        parameter.required &&
        !configValues[parameter.name]?.value &&
        !configValues[parameter.name]?.serverValue
      ) {
        const name = providerFieldName(provider.name, parameter.name);
        // A browser cannot fill this one in (W2-PRV-2, round 4), so "is
        // required" would ask for something the field does not allow.
        errors[parameter.name] = isHostOwnedProviderField(parameter)
          ? hostManagedDestinationRequired(name)
          : `${name} is required`;
      }
    });

    if (Object.keys(errors).length > 0) {
      setValidationErrors(errors);
      return;
    }

    const toSubmit = Object.fromEntries(
      Object.entries(configValues)
        .map(([key, entry]) => {
          const parameter = parameters.find((item) => item.name === key);
          const value =
            entry.value ||
            (!parameter?.secret && typeof entry.serverValue === 'string' ? entry.serverValue : '');
          return [key, value];
        })
        .filter(([, value]) => !!value)
    );

    submittingRef.current = true;
    setIsSubmitting(true);
    try {
      await providerConfigSubmitHandler(upsert, provider, toSubmit);
      if (!mountedRef.current) return;
      // The provider check can succeed by listing static models without ever
      // authenticating the CLI. Model selection requires a separate live probe.
      if (codingAgentKind) {
        const status = await fetchCodingAgentStatus();
        if (!mountedRef.current) return;
        const agent = status.agents.find((item) => item.kind === codingAgentKind);
        if (!agent?.path || agent.auth.state !== 'signed_in_subscription') {
          setCheckedAgents(status.agents);
          setError(providerSetupCopy.completeCliSetup);
          setCheckAttempt((attempt) => attempt + 1);
          return;
        }
      }
      if (onConfigured) {
        onConfigured(provider);
      } else {
        onClose();
      }
    } catch (error) {
      if (!mountedRef.current) return;
      setCheckedAgents(undefined);
      setError(`${error}`);
      setCheckAttempt((attempt) => attempt + 1);
    } finally {
      submittingRef.current = false;
      if (mountedRef.current) setIsSubmitting(false);
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

  // The delete states say what they are in the title's ink (a status colour, never a glyph).
  const titleInk = showDeleteConfirmation
    ? isActiveProvider
      ? 'text-text-warning'
      : 'text-text-danger'
    : undefined;

  return (
    <>
      <ModalShell
        open={!!error}
        onOpenChange={(open) => !open && setError(null)}
        size="md"
        scrollBody
        title={
          codingAgentKind
            ? providerSetupCopy.setUp(provider.metadata.display_name)
            : providerSetupCopy.needsAttention
        }
        subtitle={
          codingAgentKind ? providerSetupCopy.agentCheckFailed : providerSetupCopy.checkFailed
        }
        footer={
          <Button variant="secondary" onClick={() => setError(null)}>
            {codingAgentKind ? providerSetupCopy.editCommandPath : providerSetupCopy.goBack}
          </Button>
        }
      >
        <div className="py-3">
          {codingAgentKind && error ? (
            <CodingAgentSetupRecovery
              key={checkAttempt}
              kind={codingAgentKind}
              onRetry={() => void handleSubmitForm()}
              isRetrying={isSubmitting}
              initialAgents={checkedAgents}
            />
          ) : (
            <>
              <p className="whitespace-pre-wrap break-words text-body">{error}</p>
              <p className="mt-3 text-secondary text-text-muted">{providerSetupCopy.tryAgain}</p>
            </>
          )}
        </div>
      </ModalShell>
      <ModalShell
        open={!error}
        onOpenChange={(open) => !open && onClose()}
        onOpenAutoFocus={restOnDialog}
        size="md"
        purpose="form"
        scrollBody
        title={headerText}
        titleClassName={titleInk}
        titleInfo={
          titleHelp ? (
            <InfoTip label={headerText} help={titleHelp} data-testid="provider-config-title-info" />
          ) : undefined
        }
        subtitle={descriptionText}
        footer={
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
            isSubmitting={isSubmitting}
          />
        }
      >
        {!showDeleteConfirmation && (
          <div className="flex flex-col gap-3 py-3">
            <DefaultProviderSetupForm
              configValues={configValues}
              setConfigValues={setConfigValues}
              provider={provider}
              validationErrors={validationErrors}
            />
            {provider.metadata.config_keys.some((parameter) => parameter.secret) && (
              <SecureStorageNotice />
            )}
          </div>
        )}
      </ModalShell>
    </>
  );
}
