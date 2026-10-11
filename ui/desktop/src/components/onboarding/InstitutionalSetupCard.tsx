import { useState } from 'react';
import { useConfig } from '../ConfigContext';
import { checkProvider } from '../../api';
import { checkCandidateCredentials } from '../settings/providers/modal/subcomponents/handlers/DefaultSubmitHandler';
import { Button } from '../ui/button';
import { ArrowRight } from '../icons/app-icons';
import { Disclosure } from '../ui/disclosure';
import { Field } from '../ui/field';
import { Input } from '../ui/input';
import { Note } from '../ui/note';
import OnboardingCardShell, { type OnboardingCardChrome } from './OnboardingCardShell';
import { SegmentedControl } from '../ui/segmented-control';
import { Spinner } from '../ui/spinner';

interface InstitutionalSetupCardProps {
  onSuccess: (provider: string) => void;
  onStartTesting?: () => void;
  /** See `OnboardingCardShell`. Defaults to the standalone card. */
  chrome?: OnboardingCardChrome;
}

type VersaFlavor = 'azure' | 'bedrock';

// Versa Bedrock's own keys, the only ones `versa_bedrock.rs` reads. This card
// used to write `AWS_ENDPOINT_URL_BEDROCK` and `AWS_REGION`, which belong to the
// PUBLIC Amazon Bedrock card: the write marked that card Configured and replaced
// its region, and the public provider took UCSF's gateway as its endpoint.
const VERSA_BEDROCK_DEFAULTS = {
  VERSA_BEDROCK_ENDPOINT: 'https://unified-api.ucsf.edu/general/awsai',
  VERSA_BEDROCK_REGION: 'us-west-2',
};

// ⚠ No deployment, deliberately. `versa_azure` posts each model to its own
// deployment (`VERSA_AZURE_DEPLOYMENTS` in versa_azure.rs), so the model a chat
// selects chooses it. A `VERSA_AZURE_DEPLOYMENT_NAME` that names a catalog
// deployment is ignored, and any other value pins EVERY model to that one
// deployment while the chat still shows the model it selected. This card used
// to write the shipped default on every connect (ignored) and offer a box for
// any other value (that trap, in a first-run form). An operator who needs a
// deployment the catalog does not list yet sets the key in config instead.
const VERSA_AZURE_DEFAULTS = {
  VERSA_AZURE_ENDPOINT: 'https://unified-api.ucsf.edu/general',
  VERSA_AZURE_API_VERSION: '2025-01-01-preview',
};

const TABS: { id: VersaFlavor; label: string }[] = [
  { id: 'azure', label: 'Azure OpenAI (GPT)' },
  { id: 'bedrock', label: 'Bedrock (Claude)' },
];

/**
 * The providers this card configures — one per flavour of its own toggle, and
 * the `BIOROUTER_PROVIDER` value each branch of `handleSubmit` writes.
 *
 * ⚠ Exported so the provider catalog can ask *this card* which rows it covers,
 * rather than keeping a second table mapping an institution to a setup form.
 * The catalog groups by institution from the daemon's affiliation payload; an
 * institution whose gateways are not in this list simply gets no bespoke form,
 * which is the correct outcome — a form written for UCSF's Versa endpoints must
 * not be offered under another institution's heading.
 */
export const INSTITUTIONAL_SETUP_PROVIDER_IDS: readonly string[] = ['versa_azure', 'versa_bedrock'];

export default function InstitutionalSetupCard({
  onSuccess,
  onStartTesting,
  chrome = 'card',
}: InstitutionalSetupCardProps) {
  const { upsert } = useConfig();
  const [flavor, setFlavor] = useState<VersaFlavor>('azure');
  const [bedrockAccessKey, setBedrockAccessKey] = useState('');
  const [bedrockSecretKey, setBedrockSecretKey] = useState('');
  const [azureApiKey, setAzureApiKey] = useState('');
  const [bedrockEndpoint, setBedrockEndpoint] = useState(
    VERSA_BEDROCK_DEFAULTS.VERSA_BEDROCK_ENDPOINT
  );
  const [bedrockRegion, setBedrockRegion] = useState(VERSA_BEDROCK_DEFAULTS.VERSA_BEDROCK_REGION);
  const [azureEndpoint, setAzureEndpoint] = useState(VERSA_AZURE_DEFAULTS.VERSA_AZURE_ENDPOINT);
  const [azureApiVersion, setAzureApiVersion] = useState(
    VERSA_AZURE_DEFAULTS.VERSA_AZURE_API_VERSION
  );
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const canSubmit =
    !isLoading &&
    (flavor === 'bedrock'
      ? bedrockAccessKey.trim().length > 0 && bedrockSecretKey.trim().length > 0
      : azureApiKey.trim().length > 0);

  const handleSubmit = async () => {
    if (!canSubmit) return;
    onStartTesting?.();
    setIsLoading(true);
    setError(null);

    try {
      // W2-PRV-2: the credentials are checked as candidates BEFORE anything is
      // saved, so a pair or key the check refuses never replaces a working one.
      // (Versa has no live model listing, so today the check can only build the
      // provider with them; it is the order that the save now keeps.)
      if (flavor === 'bedrock') {
        await checkCandidateCredentials('versa_bedrock', {
          VERSA_BEDROCK_ACCESS_KEY_ID: bedrockAccessKey.trim(),
          VERSA_BEDROCK_SECRET_ACCESS_KEY: bedrockSecretKey.trim(),
        });
        // Settings before credentials: a daemon that cannot prove a person
        // refuses to move the endpoint, and a refusal then saves no key.
        await upsert('VERSA_BEDROCK_ENDPOINT', bedrockEndpoint.trim(), false);
        await upsert('VERSA_BEDROCK_REGION', bedrockRegion.trim(), false);
        await upsert('VERSA_BEDROCK_ACCESS_KEY_ID', bedrockAccessKey.trim(), true);
        await upsert('VERSA_BEDROCK_SECRET_ACCESS_KEY', bedrockSecretKey.trim(), true);
        await checkProvider({ body: { provider: 'versa_bedrock' }, throwOnError: true });
        await upsert('BIOROUTER_PROVIDER', 'versa_bedrock', false);
        onSuccess('versa_bedrock');
      } else {
        await checkCandidateCredentials('versa_azure', {
          VERSA_AZURE_API_KEY: azureApiKey.trim(),
        });
        await upsert('VERSA_AZURE_ENDPOINT', azureEndpoint.trim(), false);
        await upsert('VERSA_AZURE_API_VERSION', azureApiVersion.trim(), false);
        await upsert('VERSA_AZURE_API_KEY', azureApiKey.trim(), true);
        await checkProvider({ body: { provider: 'versa_azure' }, throwOnError: true });
        await upsert('BIOROUTER_PROVIDER', 'versa_azure', false);
        onSuccess('versa_azure');
      }
    } catch (e) {
      // The daemon's refusal arrives as its body text, not an `Error`, and it
      // is the sentence that says what is wrong.
      const said =
        e instanceof Error ? e.message : typeof e === 'string' && e.trim() ? e.trim() : null;
      setError(said ? `Could not connect: ${said}` : 'Could not connect with these credentials.');
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <OnboardingCardShell
      chrome={chrome}
      titleId="institutional-setup-title"
      category="institutional"
      label="Institutional · UCSF Versa API"
      title="UCSF-hosted models"
      description="Use UCSF-hosted models through the Versa unified API. Best for UCSF affiliates."
    >
      {/* Which gateway: one value of two, so the shared segmented control, not a tablist. */}
      <div className="mb-4">
        <SegmentedControl
          aria-label="UCSF-hosted provider"
          options={TABS.map((tab) => ({ value: tab.id, label: tab.label }))}
          value={flavor}
          onValueChange={(next) => {
            setFlavor(next);
            setError(null);
          }}
        />
      </div>

      {flavor === 'azure' ? (
        <Field id="versa-azure-api-key" label="API key">
          <Input
            type="password"
            value={azureApiKey}
            onChange={(e) => setAzureApiKey(e.target.value)}
            disabled={isLoading}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && canSubmit) handleSubmit();
            }}
          />
        </Field>
      ) : (
        <div className="flex flex-col gap-3">
          <Field id="versa-bedrock-access-key" label="Access key ID">
            <Input
              type="password"
              value={bedrockAccessKey}
              onChange={(e) => setBedrockAccessKey(e.target.value)}
              disabled={isLoading}
            />
          </Field>
          <Field id="versa-bedrock-secret-key" label="Secret access key">
            <Input
              type="password"
              value={bedrockSecretKey}
              onChange={(e) => setBedrockSecretKey(e.target.value)}
              disabled={isLoading}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && canSubmit) handleSubmit();
              }}
            />
          </Field>
        </div>
      )}

      {/* The two overrides most people never touch. The config key each one writes is machine
          text, so it sits behind the label's InfoTip rather than being the label. */}
      <Disclosure
        label="Advanced"
        summary={flavor === 'azure' ? 'Endpoint, API version' : 'Endpoint, region'}
        open={advancedOpen}
        onOpenChange={setAdvancedOpen}
        className="mt-3"
      >
        <div className="flex flex-col gap-3 pt-2">
          {flavor === 'azure' ? (
            <>
              <Field
                id="versa-azure-endpoint"
                label="Endpoint"
                info="Saved as VERSA_AZURE_ENDPOINT."
              >
                <Input
                  type="text"
                  value={azureEndpoint}
                  onChange={(e) => setAzureEndpoint(e.target.value)}
                  disabled={isLoading}
                />
              </Field>
              <Field
                id="versa-azure-api-version"
                label="API version"
                info="Saved as VERSA_AZURE_API_VERSION."
              >
                <Input
                  type="text"
                  value={azureApiVersion}
                  onChange={(e) => setAzureApiVersion(e.target.value)}
                  disabled={isLoading}
                />
              </Field>
            </>
          ) : (
            <>
              <Field
                id="versa-bedrock-endpoint"
                label="Endpoint"
                info="Saved as VERSA_BEDROCK_ENDPOINT."
              >
                <Input
                  type="text"
                  value={bedrockEndpoint}
                  onChange={(e) => setBedrockEndpoint(e.target.value)}
                  disabled={isLoading}
                />
              </Field>
              <Field id="versa-bedrock-region" label="Region" info="Saved as VERSA_BEDROCK_REGION.">
                <Input
                  type="text"
                  value={bedrockRegion}
                  onChange={(e) => setBedrockRegion(e.target.value)}
                  disabled={isLoading}
                />
              </Field>
            </>
          )}
        </div>
      </Disclosure>

      <div className="mt-5 flex flex-col items-stretch gap-2 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-4 sm:gap-y-3">
        <Button onClick={handleSubmit} disabled={!canSubmit} size="lg" className="w-full sm:w-auto">
          {isLoading ? (
            <Spinner />
          ) : (
            <>
              Connect to {flavor === 'azure' ? 'Versa Azure OpenAI' : 'Versa Bedrock'}
              <ArrowRight className="w-4 h-4" />
            </>
          )}
        </Button>
      </div>

      {error && (
        <Note tone="danger" role="alert" className="mt-3">
          {error}
        </Note>
      )}
    </OnboardingCardShell>
  );
}
