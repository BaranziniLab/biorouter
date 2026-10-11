import React, { useEffect, useMemo, useState, useCallback } from 'react';
import { Input } from '../../../../../ui/input';
import { SecretInput } from '../../../../../ui/secret-input';
import { useConfig } from '../../../../../ConfigContext';
import { ProviderDetails, ConfigKey } from '../../../../../../api';
import { Disclosure } from '../../../../../ui/disclosure';
import { Field } from '../../../../../ui/field';
import { configLabels } from '../../../../../../utils/configUtils';
import { isBrowserSurface } from '../../../../../../utils/surface';
import { HostManagedModelNote } from '../../../../../privacy/HostManagedModelNote';
import { isDestinationConfigKey } from '../../../../destinationConfigKeys';
import { sentenceCase } from '../../../../config/configLabels';
import { providerSetupCopy } from '../../../copy';
import { useHandFocusOnReady } from '../../../../models/dialogFocus';

type ValidationErrors = Record<string, string>;

type ConfigValue = string | { maskedValue: string };
export interface ConfigInput {
  serverValue?: ConfigValue;
  value?: string;
}

interface DefaultProviderSetupFormProps {
  configValues: Record<string, ConfigInput>;
  setConfigValues: React.Dispatch<React.SetStateAction<Record<string, ConfigInput>>>;
  provider: ProviderDetails;
  validationErrors: ValidationErrors;
}

// Frontend-side defaults per provider — ensures defaults show up immediately
// without requiring a backend recompile. The backend also declares these defaults
// in Rust (azure.rs, bedrock.rs) for CLI consistency.
//
// ⚠ An entry only reaches a key the provider DECLARES in `metadata.config_keys`:
// both readers below look a default up per declared parameter, so one for any
// other key is never read. `versa_azure` and `versa_bedrock` declare only their
// credentials, which is why neither has an entry.
//
// ⚠ A default here is a VALUE, not a hint: `loadConfigValues` fills the field
// with it and the submit handler saves it if the user leaves the field alone.
// So only a value that is right for every user belongs here. Anything that
// names one user's resource goes in PROVIDER_KEY_PLACEHOLDERS instead. The
// Azure endpoint used to default to UCSF's Versa gateway, which sent a
// non-UCSF user's Azure key and transcript to UCSF.
const PROVIDER_KEY_DEFAULTS: Record<string, Record<string, string>> = {
  azure_openai: {
    AZURE_OPENAI_API_VERSION: '2025-01-01-preview',
  },
  aws_bedrock: {
    AWS_REGION: 'us-west-2',
  },
};

// Placeholder text for a key the user must fill in with their own value. Shown
// in the empty field and never saved.
const PROVIDER_KEY_PLACEHOLDERS: Record<string, Record<string, string>> = {
  azure_openai: {
    AZURE_OPENAI_ENDPOINT: 'https://<your-resource>.openai.azure.com',
  },
  // A llama-server Biorouter does not manage, which stays private only on this
  // machine: `llamacpp`'s tier demotes a non-loopback host to Public. The
  // generic `https://api.example.com` suggested exactly the host that does it
  // (W2-PRV-13).
  llamacpp: {
    LLAMACPP_EXTERNAL_HOST: 'http://127.0.0.1:8080',
  },
};

/** The note every host-owned field points its `aria-describedby` at. */
const HOST_OWNED_NOTE_ID = 'provider-config-host-owned-note';

const envToPrettyName = (envVar: string) => {
  const wordReplacements: { [w: string]: string } = {
    Api: 'API',
    Aws: 'AWS',
    Gcp: 'GCP',
    Id: 'ID',
    Url: 'URL',
  };

  return envVar
    .toLowerCase()
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .map((word) => wordReplacements[word] || word)
    .join(' ')
    .trim();
};

/**
 * A config key's name in words, for its label, its placeholder and its
 * "is required" error: the curated label in `configUtils` when there is one
 * ("Llama Server external host"), else the key's role, else the key itself
 * without the provider's prefix. The modal's required error used to print the
 * raw env var ("VERSA_BEDROCK_ACCESS_KEY_ID is required") and the placeholder
 * fallback was the env var with spaces (W2-PRV-13).
 *
 * Sentence case, like every other label in the app (V4-R3-2): the curated labels
 * are Title Case ("Anthropic API Key"), so the name goes through the one
 * `sentenceCase` the Configuration editor uses, which keeps acronyms and product
 * names ("Anthropic API key", "Azure OpenAI deployment name"). That is also what
 * makes the required error read "Anthropic API key is required", beside the custom
 * provider form's "API key is required".
 */
export function providerFieldName(providerName: string, parameterName: string): string {
  return sentenceCase(titleCaseFieldName(providerName, parameterName));
}

function titleCaseFieldName(providerName: string, parameterName: string): string {
  if (configLabels[parameterName]) return configLabels[parameterName];
  const name = parameterName.toLowerCase();
  if (name.includes('api_key')) return 'API Key';
  if (name.includes('api_url') || name.includes('host')) return 'API Host';
  if (name.includes('models')) return 'Models';

  let parameter_name = parameterName.toUpperCase();
  if (parameter_name.startsWith(providerName.toUpperCase().replace('-', '_'))) {
    parameter_name = parameter_name.slice(providerName.length + 1);
  }
  return envToPrettyName(parameter_name);
}

/**
 * W2-PRV-2, round 4. Is this field the host computer's in this renderer? In a
 * browser served by `biorouter serve`, a setting that decides where the
 * provider sends its requests and key cannot be changed: the daemon asks for a
 * proof of a person that a browser can never give. The field shows its value
 * and is not editable, and the submit handler leaves it out.
 */
export function isHostOwnedProviderField(parameter: { name: string; secret?: boolean }): boolean {
  return !parameter.secret && isBrowserSurface() && isDestinationConfigKey(parameter.name);
}

export default function DefaultProviderSetupForm({
  configValues,
  setConfigValues,
  provider,
  validationErrors = {},
}: DefaultProviderSetupFormProps) {
  const parameters = useMemo(
    () => provider.metadata.config_keys || [],
    [provider.metadata.config_keys]
  );
  const [isLoading, setIsLoading] = useState(true);
  const [optionalExpanded, setOptionalExpanded] = useState(false);
  const { read } = useConfig();

  const loadConfigValues = useCallback(async () => {
    setIsLoading(true);
    try {
      const values: { [k: string]: ConfigInput } = {};

      const frontendDefaults = PROVIDER_KEY_DEFAULTS[provider.name] ?? {};

      for (const parameter of parameters) {
        const configKey = `${parameter.name}`;
        const configValue = (await read(configKey, parameter.secret || false)) as ConfigValue;

        if (configValue) {
          values[parameter.name] = { serverValue: configValue };
        } else {
          const defaultValue = parameter.default ?? frontendDefaults[parameter.name] ?? null;
          if (defaultValue !== null) {
            values[parameter.name] = { value: defaultValue };
          }
        }
      }

      setConfigValues((prev) => ({
        ...prev,
        ...values,
      }));
    } finally {
      setIsLoading(false);
    }
  }, [parameters, provider.name, read, setConfigValues]);

  useEffect(() => {
    loadConfigValues();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The fields exist only once the saved values are in, and the dialog around them is already open
  // by then, so Radix could not put focus on one: the Configure dialog rests it on its box and
  // this hands it to the first enabled field (see `models/dialogFocus.ts`).
  const fieldsRef = useHandFocusOnReady<HTMLDivElement>(
    !isLoading,
    'input:not([disabled]):not([type="hidden"])'
  );

  const getPlaceholder = (parameter: ConfigKey): string => {
    if (parameter.secret) {
      const serverValue = configValues[parameter.name]?.serverValue;
      if (typeof serverValue === 'object' && 'maskedValue' in serverValue) {
        return serverValue.maskedValue;
      }
    }

    const hint = (PROVIDER_KEY_PLACEHOLDERS[provider.name] ?? {})[parameter.name];
    if (hint) {
      return hint;
    }

    const defaultValue =
      parameter.default ?? (PROVIDER_KEY_DEFAULTS[provider.name] ?? {})[parameter.name] ?? null;
    if (defaultValue !== null) {
      return defaultValue;
    }

    const name = parameter.name.toLowerCase();
    if (name.includes('api_key')) return 'Your API key';
    if (name.includes('api_url') || name.includes('host')) return 'https://api.example.com';
    if (name.includes('models')) return 'model-a, model-b';

    return getFieldName(parameter);
  };

  /** The field's name in words — the label's text, and what a reveal toggle is called. */
  const getFieldName = (parameter: ConfigKey): string =>
    providerFieldName(provider.name, parameter.name);

  if (isLoading) {
    return (
      <div className="py-4 text-center text-secondary text-text-muted">
        {providerSetupCopy.loadingValues}
      </div>
    );
  }

  function getRenderValue(parameter: ConfigKey): string | undefined {
    if (parameter.secret) {
      return undefined;
    }

    const entry = configValues[parameter.name];
    return entry?.value || (entry?.serverValue as string) || '';
  }

  const renderParametersList = (parameters: ConfigKey[]) => {
    return parameters.map((parameter) => {
      const fieldId = `provider-config-${parameter.name}`;
      const hostOwned = isHostOwnedProviderField(parameter);
      const fieldProps = {
        value: getRenderValue(parameter),
        onChange: (e: React.ChangeEvent<HTMLInputElement>) => {
          setConfigValues((prev) => {
            const newValue = { ...(prev[parameter.name] || {}), value: e.target.value };
            return {
              ...prev,
              [parameter.name]: newValue,
            };
          });
        },
        placeholder: getPlaceholder(parameter),
        required: parameter.required,
        disabled: hostOwned,
        'aria-describedby': hostOwned ? HOST_OWNED_NOTE_ID : undefined,
      };

      return (
        // The config key a field writes is machine text, so it sits behind the label's InfoTip
        // rather than beside every label (principle 10); a field names itself in words.
        <Field
          key={parameter.name}
          id={fieldId}
          label={getFieldName(parameter)}
          info={`Saved as ${parameter.name}.`}
          error={validationErrors[parameter.name]}
        >
          {/* F5: a secret is masked, exactly as the custom-provider form masks
              its key — the same primitive, so the two cannot diverge again. A
              non-secret parameter (an endpoint, a region) stays readable. */}
          {parameter.secret ? (
            <SecretInput {...fieldProps} revealLabel={getFieldName(parameter)} />
          ) : (
            <Input {...fieldProps} type="text" />
          )}
        </Field>
      );
    });
  };

  const hasHostOwnedField = parameters.some(isHostOwnedProviderField);

  let aboveFoldParameters = parameters.filter((p) => p.required);
  let belowFoldParameters = parameters.filter((p) => !p.required);
  if (aboveFoldParameters.length === 0) {
    aboveFoldParameters = belowFoldParameters;
    belowFoldParameters = [];
  }

  const expandCtaText = `${optionalExpanded ? 'Hide' : 'Show'} ${belowFoldParameters.length} ${
    belowFoldParameters.length === 1 ? 'option' : 'options'
  }`;

  return (
    <div ref={fieldsRef} className="flex flex-col gap-4 py-1">
      {hasHostOwnedField && (
        <div id={HOST_OWNED_NOTE_ID}>
          <HostManagedModelNote topic="destination" testId="host-managed-destination-note" />
        </div>
      )}
      {aboveFoldParameters.length === 0 && belowFoldParameters.length === 0 ? (
        <div className="py-2 text-center text-secondary text-text-muted">
          {providerSetupCopy.noParameters}
        </div>
      ) : (
        <>
          {renderParametersList(aboveFoldParameters)}
          {belowFoldParameters.length > 0 && (
            <Disclosure
              label={expandCtaText}
              open={optionalExpanded}
              onOpenChange={setOptionalExpanded}
            >
              <div className="flex flex-col gap-4">{renderParametersList(belowFoldParameters)}</div>
            </Disclosure>
          )}
        </>
      )}
    </div>
  );
}
