import { useEffect, useRef, useState } from 'react';
import { detectProvider, getDetectableProviders } from '../../api';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Note } from '../ui/note';
import { AlertTriangle, Check } from '../icons/app-icons';
import { Field } from '../ui/field';
import { Spinner } from '../ui/spinner';
import { commercialCopy } from './copy';

interface CommercialSetupCardProps {
  onSuccess: (setup: DetectedProviderSetup) => void | Promise<void>;
  onStartTesting?: () => void;
}

export interface DetectedProviderSetup {
  provider: string;
  model: string;
  models: string[];
  apiKey: string;
  apiKeyConfigKey: string;
  extraConfig: Record<string, string>;
}

/**
 * Persist a detected provider: the secret, then any non-secret endpoint config,
 * then the provider selection itself.
 *
 * ⚠ **The ORDER is the contract, and it is asserted by `ProviderGuard.test.tsx`.**
 * The endpoint config is written before `BIOROUTER_PROVIDER` so the saved
 * provider targets the same endpoint detection validated against — a regional
 * host written *after* the selection leaves a window in which the bound provider
 * points somewhere the key was never checked against.
 *
 * Exported because two hosts reach the detection card — the first-run guard and
 * the `welcome` route — and a second copy of this sequence is a second place for
 * that ordering to be got wrong.
 */
export async function persistDetectedProviderSetup(
  upsert: (key: string, value: unknown, isSecret: boolean) => Promise<unknown>,
  { provider, apiKey, apiKeyConfigKey, extraConfig }: DetectedProviderSetup
): Promise<void> {
  await upsert(apiKeyConfigKey, apiKey, true);
  for (const [key, value] of Object.entries(extraConfig)) {
    await upsert(key, value, false);
  }
  await upsert('BIOROUTER_PROVIDER', provider, false);
}

interface DetectionResult {
  provider: string;
  model: string;
  totalModels: number;
}

// Fallback list shown if the backend list can't be fetched. The authoritative
// list comes from GET /config/detectable-providers (single source of truth in
// the Rust `auto_detect` module).
const FALLBACK_PROVIDERS = ['OpenAI', 'Anthropic', 'Google', 'Groq', 'xAI', 'z.ai', 'Xiaomi MiMo'];

// Map a backend failure reason code to actionable copy. The no-match case names what IS
// supported, in the note's own detail: it used to be a hand-drawn "·" list under the note.
function messageForReason(
  reason: string | null | undefined,
  supportedLabel: string
): { title: string; detail: string } {
  switch (reason) {
    case 'timeout':
      return commercialCopy.errors.timeout;
    case 'network':
      return commercialCopy.errors.network;
    case 'configuration':
      return commercialCopy.errors.configuration;
    case 'invalid_key':
      return commercialCopy.errors.invalidKey;
    case 'no_match':
    default:
      return commercialCopy.errors.noMatch(supportedLabel);
  }
}

export default function CommercialSetupCard({
  onSuccess,
  onStartTesting,
}: CommercialSetupCardProps) {
  const [apiKey, setApiKey] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [result, setResult] = useState<DetectionResult | null>(null);
  const [errorReason, setErrorReason] = useState<string | null>(null);
  const [supported, setSupported] = useState<string[]>(FALLBACK_PROVIDERS);
  const inputRef = useRef<HTMLInputElement>(null);

  // Pull the supported-provider list from the backend so this UI never drifts
  // from what detection actually supports.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const response = await getDetectableProviders();
        const names = response.data?.providers?.map((p) => p.display_name).filter(Boolean);
        if (!cancelled && names && names.length > 0) {
          setSupported(names);
        }
      } catch {
        // keep the fallback list
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const testApiKey = async () => {
    const actualValue = (inputRef.current?.value || apiKey).trim();
    if (!actualValue) return;

    onStartTesting?.();
    setIsLoading(true);
    setResult(null);
    setErrorReason(null);

    try {
      const response = await detectProvider({ body: { api_key: actualValue } });
      const data = response.data;

      if (data && data.provider_name) {
        const model = data.default_model ?? data.models?.[0] ?? '';
        const models = data.models ?? [];
        const extraConfig = data.extra_config ?? {};
        const apiKeyConfigKey =
          data.api_key_config_key ?? `${data.provider_name.toUpperCase()}_API_KEY`;
        setResult({
          provider: data.provider_name,
          model,
          totalModels: models.length,
        });
        try {
          await onSuccess({
            provider: data.provider_name,
            model,
            models,
            apiKey: actualValue,
            apiKeyConfigKey,
            extraConfig,
          });
        } catch {
          setResult(null);
          setErrorReason('configuration');
        }
      } else {
        setErrorReason(data?.reason ?? 'no_match');
      }
    } catch {
      // Transport-level failure (server unreachable, etc.)
      setErrorReason('network');
    } finally {
      setIsLoading(false);
    }
  };

  const hasInput = apiKey.trim().length > 0;
  const canSubmit = hasInput && !isLoading;
  const supportedLabel = supported.join(', ');
  const failure = errorReason ? messageForReason(errorReason, supportedLabel) : null;

  return (
    <div className="min-w-0">
      {/* A labelled field, the Institutional form's recipe: "API key" with what is detected in its
          InfoTip (FR-S-5). It was an unlabelled 36px box whose placeholder was its only words, and
          the supported list appeared only after a failure. */}
      <div className="space-y-3">
        <Field
          id="commercial-provider-api-key"
          label={commercialCopy.apiKey}
          info={commercialCopy.help(supportedLabel)}
        >
          <Input
            ref={inputRef}
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            disabled={isLoading}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && canSubmit) testApiKey();
            }}
          />
        </Field>
        {/* Secondary until there is a key to detect: an empty field's disabled coral button was a
            second coral fill beside an open agent's "Use" (principle 7, FR-R1-11). */}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant={hasInput ? 'default' : 'secondary'}
            onClick={testApiKey}
            disabled={!canSubmit}
            data-testid="commercial-detect"
          >
            {isLoading ? (
              <>
                <Spinner size={14} />
                {commercialCopy.detecting}
              </>
            ) : (
              commercialCopy.detect
            )}
          </Button>
        </div>

        {result && (
          <Note tone="success" role="status" icon={Check}>
            <div className="text-label">{commercialCopy.detected(result.provider)}</div>
            <div className="mt-0.5">
              {commercialCopy.detectedDetail(result.model, result.totalModels)}
            </div>
          </Note>
        )}

        {failure && (
          <Note tone="danger" role="alert" icon={AlertTriangle}>
            <div className="text-label">{failure.title}</div>
            <div className="mt-0.5">{failure.detail}</div>
          </Note>
        )}
      </div>
    </div>
  );
}
