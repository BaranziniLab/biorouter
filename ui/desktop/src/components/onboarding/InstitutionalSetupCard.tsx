import { useState } from 'react';
import { useConfig } from '../ConfigContext';
import { checkProvider } from '../../api';
import { checkCandidateCredentials } from '../settings/providers/modal/subcomponents/handlers/DefaultSubmitHandler';
import { Button } from '../ui/button';
import { Disclosure } from '../ui/disclosure';
import { Field } from '../ui/field';
import { Input } from '../ui/input';
import { Note } from '../ui/note';
import OnboardingCardShell, { type OnboardingCardChrome } from './OnboardingCardShell';
import { Spinner } from '../ui/spinner';
import { institutionalCopy } from './copy';

interface InstitutionalSetupCardProps {
  /** Which gateway the row that opened this form is. */
  flavor: VersaFlavor;
  onSuccess: (provider: string) => void;
  onStartTesting?: () => void;
  /** See `OnboardingCardShell`. Defaults to the standalone card. */
  chrome?: OnboardingCardChrome;
}

export type VersaFlavor = 'azure' | 'bedrock';

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

/**
 * The rows this form sets up, and which gateway each one is: the `BIOROUTER_PROVIDER` value each
 * branch of `handleSubmit` writes.
 *
 * ⚠ Exported so the provider catalog can ask *this form* which rows open it, rather than keeping
 * a second table mapping an institution to a setup form. The catalog groups by institution from
 * the daemon's affiliation payload; a gateway not in this table simply opens no bespoke form, which
 * is the correct outcome: a form written for UCSF's Versa endpoints must not be offered for another
 * institution's gateway.
 *
 * The ROW is the selector (vision FR-R1-6 / FR-S-6). The Institutional tab used to list "Versa
 * API Azure" and "Versa API Bedrock" as rows that opened the Configure dialog, and under them a
 * second, unheaded form whose segmented control chose between the same two: two paths, on two
 * surfaces, for one setup. Each row now opens this form in place, for its own gateway.
 */
export const INSTITUTIONAL_SETUP_FLAVOR: Readonly<Record<string, VersaFlavor>> = {
  versa_azure: 'azure',
  versa_bedrock: 'bedrock',
};
export const INSTITUTIONAL_SETUP_PROVIDER_IDS: readonly string[] = Object.keys(
  INSTITUTIONAL_SETUP_FLAVOR
);

export default function InstitutionalSetupCard({
  flavor,
  onSuccess,
  onStartTesting,
  chrome = 'card',
}: InstitutionalSetupCardProps) {
  const { upsert } = useConfig();
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
      setError(said ? institutionalCopy.refused(said) : institutionalCopy.refusedPlain);
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
      {flavor === 'azure' ? (
        <Field id="versa-azure-api-key" label={institutionalCopy.apiKey}>
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
          <Field id="versa-bedrock-access-key" label={institutionalCopy.accessKeyId}>
            <Input
              type="password"
              value={bedrockAccessKey}
              onChange={(e) => setBedrockAccessKey(e.target.value)}
              disabled={isLoading}
            />
          </Field>
          <Field id="versa-bedrock-secret-key" label={institutionalCopy.secretAccessKey}>
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
        label={institutionalCopy.advanced}
        summary={
          flavor === 'azure' ? institutionalCopy.azureSummary : institutionalCopy.bedrockSummary
        }
        open={advancedOpen}
        onOpenChange={setAdvancedOpen}
        className="mt-3"
      >
        <div className="flex flex-col gap-3 pt-2">
          {flavor === 'azure' ? (
            <>
              <Field
                id="versa-azure-endpoint"
                label={institutionalCopy.endpoint}
                info={institutionalCopy.savedAs('VERSA_AZURE_ENDPOINT')}
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
                label={institutionalCopy.apiVersion}
                info={institutionalCopy.savedAs('VERSA_AZURE_API_VERSION')}
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
                label={institutionalCopy.endpoint}
                info={institutionalCopy.savedAs('VERSA_BEDROCK_ENDPOINT')}
              >
                <Input
                  type="text"
                  value={bedrockEndpoint}
                  onChange={(e) => setBedrockEndpoint(e.target.value)}
                  disabled={isLoading}
                />
              </Field>
              <Field
                id="versa-bedrock-region"
                label={institutionalCopy.region}
                info={institutionalCopy.savedAs('VERSA_BEDROCK_REGION')}
              >
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

      {/* One verb, on the 32px rung, and it keeps its words while it works: a bare spinner left
          the button with no name and half its width (FR-S-6, FR-R1-11). */}
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button onClick={handleSubmit} disabled={!canSubmit} data-testid="institutional-connect">
          {isLoading ? (
            <>
              <Spinner size={14} />
              {institutionalCopy.connecting}
            </>
          ) : (
            institutionalCopy.connect
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
