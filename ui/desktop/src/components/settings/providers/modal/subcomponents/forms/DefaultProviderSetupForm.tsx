import React, { useEffect, useMemo, useState, useCallback } from 'react';
import { Input } from '../../../../../ui/input';
import { SecretInput } from '../../../../../ui/secret-input';
import { useConfig } from '../../../../../ConfigContext';
import { ProviderDetails, ConfigKey } from '../../../../../../api';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../../../../../ui/collapsible';

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
const PROVIDER_KEY_DEFAULTS: Record<string, Record<string, string>> = {
  azure_openai: {
    AZURE_OPENAI_ENDPOINT: 'https://unified-api.ucsf.edu/general',
    AZURE_OPENAI_API_VERSION: '2025-01-01-preview',
  },
  aws_bedrock: {
    AWS_REGION: 'us-west-2',
  },
  versa_azure: {
    AZURE_OPENAI_ENDPOINT: 'https://unified-api.ucsf.edu/general',
    AZURE_OPENAI_DEPLOYMENT_NAME: 'gpt-5.5-2026-04-24',
    AZURE_OPENAI_API_VERSION: '2025-01-01-preview',
  },
};

const envToPrettyName = (envVar: string) => {
  const wordReplacements: { [w: string]: string } = {
    Api: 'API',
    Aws: 'AWS',
    Gcp: 'GCP',
  };

  return envVar
    .toLowerCase()
    .split('_')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .map((word) => wordReplacements[word] || word)
    .join(' ')
    .trim();
};

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

  const getPlaceholder = (parameter: ConfigKey): string => {
    if (parameter.secret) {
      const serverValue = configValues[parameter.name]?.serverValue;
      if (typeof serverValue === 'object' && 'maskedValue' in serverValue) {
        return serverValue.maskedValue;
      }
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

    return parameter.name
      .replace(/_/g, ' ')
      .replace(/^./, (str) => str.toUpperCase())
      .trim();
  };

  /** The field's name in words — the label's text, and what a reveal toggle is called. */
  const getFieldName = (parameter: ConfigKey): string => {
    const name = parameter.name.toLowerCase();
    if (name.includes('api_key')) return 'API Key';
    if (name.includes('api_url') || name.includes('host')) return 'API Host';
    if (name.includes('models')) return 'Models';

    let parameter_name = parameter.name.toUpperCase();
    if (parameter_name.startsWith(provider.name.toUpperCase().replace('-', '_'))) {
      parameter_name = parameter_name.slice(provider.name.length + 1);
    }
    return envToPrettyName(parameter_name);
  };

  const getFieldLabel = (parameter: ConfigKey) => {
    const name = parameter.name.toLowerCase();
    // The recognised roles are labelled by the role alone; everything else also
    // names the config key it writes.
    if (['api_key', 'api_url', 'host', 'models'].some((role) => name.includes(role))) {
      return getFieldName(parameter);
    }

    return (
      <span>
        <span>{getFieldName(parameter)}</span>
        <span className="text-xs text-text-muted font-normal ml-1.5">({parameter.name})</span>
      </span>
    );
  };

  if (isLoading) {
    return <div className="text-center py-4">Loading configuration values...</div>;
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
      const fieldProps = {
        id: fieldId,
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
        className: `w-full h-9 px-3 rounded-element shadow-none text-sm ${
          validationErrors[parameter.name]
            ? 'border-2 border-border-danger'
            : 'border border-border-subtle hover:border-border-strong focus:border-border-strong'
        } bg-background-default placeholder:text-text-muted text-text-default`,
        required: parameter.required,
      };

      return (
        <div key={parameter.name}>
          <label htmlFor={fieldId} className="block text-sm font-medium text-text-default mb-1">
            {getFieldLabel(parameter)}
            {parameter.required && <span className="text-text-danger ml-1">*</span>}
          </label>
          {/* F5: a secret is masked, exactly as the custom-provider form masks
              its key — the same primitive, so the two cannot diverge again. A
              non-secret parameter (an endpoint, a region) stays readable. */}
          {parameter.secret ? (
            <SecretInput {...fieldProps} revealLabel={getFieldName(parameter)} />
          ) : (
            <Input {...fieldProps} type="text" />
          )}
          {validationErrors[parameter.name] && (
            <p className="text-text-danger text-sm mt-1">{validationErrors[parameter.name]}</p>
          )}
        </div>
      );
    });
  };

  let aboveFoldParameters = parameters.filter((p) => p.required);
  let belowFoldParameters = parameters.filter((p) => !p.required);
  if (aboveFoldParameters.length === 0) {
    aboveFoldParameters = belowFoldParameters;
    belowFoldParameters = [];
  }

  const expandCtaText = `${optionalExpanded ? 'Hide' : 'Show'} ${belowFoldParameters.length} options `;

  return (
    <div className="mt-4 space-y-4">
      {aboveFoldParameters.length === 0 && belowFoldParameters.length === 0 ? (
        <div className="text-center text-sm text-text-muted py-2">
          No configuration parameters for this provider.
        </div>
      ) : (
        <div>
          <div className="space-y-3">{renderParametersList(aboveFoldParameters)}</div>
          {belowFoldParameters.length > 0 && (
            <Collapsible
              open={optionalExpanded}
              onOpenChange={setOptionalExpanded}
              className="mt-4 border border-border-subtle rounded-container bg-background-muted"
            >
              <CollapsibleTrigger className="w-full flex items-center justify-between px-4 py-2.5 text-sm text-text-muted hover:text-text-default transition-colors duration-150">
                <span>{expandCtaText}</span>
                <span className="text-xs">{optionalExpanded ? '▲' : '▼'}</span>
              </CollapsibleTrigger>
              <CollapsibleContent className="px-4 pb-4 space-y-3">
                {renderParametersList(belowFoldParameters)}
              </CollapsibleContent>
            </Collapsible>
          )}
        </div>
      )}
    </div>
  );
}
