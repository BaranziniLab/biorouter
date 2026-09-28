import { checkProvider, type CheckProviderRequest } from '../../../../../../api';
import { userActionHeaders } from '../../../../../../utils/userAction';
import { isBrowserSurface } from '../../../../../../utils/surface';
import { coerceConfigKeyValue } from '../../../configKeyValue';
import { isDestinationConfigKey } from '../../../../destinationConfigKeys';

/**
 * W2-PRV-2 — `/config/check_provider`'s live, pre-save form.
 *
 * `candidate` holds the values about to be saved; the daemon builds the provider
 * with them as task-local overrides and, with `live`, makes one authenticated
 * call (listing models). Nothing is written by the check, so a key the provider
 * rejects never replaces the working one: the save below happens only after it
 * passes. A provider with no secret or no live listing passes as before.
 *
 * Typed here until the generated client carries the two fields.
 */
type LiveCheckRequest = CheckProviderRequest & {
  live: boolean;
  candidate: Record<string, string>;
};

/** Check `values` for `providerName` before any of them is saved. Throws the refusal. */
export async function checkCandidateCredentials(
  providerName: string,
  values: Record<string, string>
): Promise<void> {
  const body: LiveCheckRequest = { provider: providerName, live: true, candidate: values };
  await checkProvider({
    body,
    // The daemon refuses a live or candidate check that cannot prove a person
    // asked, because it can send a credential to a host the candidate names.
    headers: await userActionHeaders(),
    throwOnError: true,
  });
}

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
  const declared = provider.metadata.config_keys || [];
  // W2-PRV-2, round 4. In a browser served by `biorouter serve`, a setting that
  // decides where this provider sends its requests and key belongs to the
  // computer running Biorouter: the daemon refuses to change one without a
  // proof of a person, which a browser can never send. The form shows such a
  // field and does not let it be edited, so writing it here could only re-save
  // what is there or meet that refusal. It is left out; the provider keeps what
  // the host set, or its built-in default.
  const hostOwned = (name: string) => isBrowserSurface() && isDestinationConfigKey(name);
  const parameters = declared.filter(
    (parameter) => parameter.secret === true || !hostOwned(parameter.name)
  );

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

  // What this save will write, decided once, so the check below is of exactly
  // these values.
  const writes = parameters.flatMap((parameter) => {
    if (!configValues[parameter.name] && !parameter.required) {
      return [];
    }
    const value =
      configValues[parameter.name] !== undefined ? configValues[parameter.name] : parameter.default;
    if (value === undefined || value === null) {
      return [];
    }
    return [{ parameter, value }];
  });

  // W2-PRV-2: checked BEFORE anything is saved, whenever the provider holds a
  // credential: a save that carries a new key, and (in the app) one that moves a
  // host or endpoint the saved key will be sent to. A browser served by
  // `biorouter serve` cannot prove a person, so there the daemon checks only
  // with a key typed into the form, and only a save carrying the key is
  // checked. A provider with no credential is saved and built as before.
  const carriesKey = writes.some(({ parameter }) => parameter.secret === true);
  const holdsKey = parameters.some((parameter) => parameter.secret === true);
  if (writes.length > 0 && (carriesKey || (holdsKey && !isBrowserSurface()))) {
    await checkCandidateCredentials(
      provider.name,
      Object.fromEntries(writes.map(({ parameter, value }) => [parameter.name, String(value)]))
    );
  }

  // Settings before credentials. Where no person can be proven (`biorouter
  // serve`), the daemon refuses to move a host or endpoint, since the saved key
  // goes wherever it points; writing the key first would leave a new key saved
  // beside the old host. A refused setting stops the save before any
  // credential is written.
  const save = (batch: typeof writes) =>
    Promise.all(
      batch.map(({ parameter, value }) =>
        upsertFn(parameter.name, coerceConfigKeyValue(parameter, value), parameter.secret === true)
      )
    );
  await save(writes.filter(({ parameter }) => parameter.secret !== true));
  await save(writes.filter(({ parameter }) => parameter.secret === true));
  await checkProvider({
    body: { provider: provider.name },
    throwOnError: true,
  });
};
