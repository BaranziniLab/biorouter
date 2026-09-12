import { checkProvider } from '../../../../../../api';
import { coerceConfigKeyValue } from '../../../configKeyValue';

/**
 * Standalone function to submit provider configuration
 * Useful for components that don't want to use the hook
 *
 * Every value arriving here is a string — the setup form renders each key as a
 * text field (masked for a secret, but a string all the same). `/config/upsert`
 * writes what it is given verbatim, so a
 * string lands in `config.yaml` quoted (`LLAMACPP_PORT: '11543'`) and the
 * backend's typed `get_param::<usize>()` / `get_param::<bool>()` cannot read it
 * back, silently falling through to the default. `coerceConfigKeyValue` turns
 * the values of keys that *declare* a numeric or boolean default into real JSON
 * numbers/booleans; string keys are passed through untouched. See
 * `../../../configKeyValue.ts`.
 */
export const providerConfigSubmitHandler = async (
  upsertFn: (key: string, value: unknown, isSecret: boolean) => Promise<void>,
  provider: {
    name: string;
    metadata: {
      config_keys?: Array<{
        name: string;
        required?: boolean;
        default?: unknown;
        secret?: boolean;
      }>;
    };
  },
  configValues: Record<string, string>
) => {
  const parameters = provider.metadata.config_keys || [];

  const requiredParams = parameters.filter((param) => param.required);
  if (requiredParams.length === 0 && parameters.length > 0) {
    const allOptionalWithDefaults = parameters.every(
      (param) => !param.required && param.default !== undefined
    );
    if (allOptionalWithDefaults) {
      const promises: Promise<void>[] = [];

      for (const param of parameters) {
        if (param.default !== undefined) {
          const value =
            configValues[param.name] !== undefined ? configValues[param.name] : param.default;
          promises.push(
            upsertFn(param.name, coerceConfigKeyValue(param, value), param.secret === true)
          );
        }
      }

      await Promise.all(promises);
      return;
    }
  }

  const upsertPromises = parameters.map(
    async (parameter: {
      name: string;
      required?: boolean;
      default?: unknown;
      secret?: boolean;
    }) => {
      if (!configValues[parameter.name] && !parameter.required) {
        return;
      }

      const value =
        configValues[parameter.name] !== undefined
          ? configValues[parameter.name]
          : parameter.default;

      if (value === undefined || value === null) {
        return;
      }

      const configKey = `${parameter.name}`;
      const isSecret = parameter.secret === true;

      await upsertFn(configKey, coerceConfigKeyValue(parameter, value), isSecret);
    }
  );

  await Promise.all(upsertPromises);
  await checkProvider({
    body: { provider: provider.name },
    throwOnError: true,
  });
};
