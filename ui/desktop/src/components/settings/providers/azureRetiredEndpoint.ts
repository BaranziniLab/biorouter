/**
 * W2-PRV-1 — the Azure OpenAI endpoint older versions filled in by mistake.
 *
 * Until 2026-09-27 the Azure OpenAI card and `biorouter configure` pre-filled
 * `AZURE_OPENAI_ENDPOINT` with UCSF's gateway, and a person who typed only a key
 * and a deployment saved it: a commercial key and every transcript went to UCSF
 * under a Public label, and failed with a 401. The default is gone
 * (`providers/azure.rs`), but a config that saved it keeps it. Nothing is
 * rewritten or refused (a UCSF person may rely on the value); the card says what
 * it is and offers Configure. The daemon logs the same condition once
 * (`azure::is_retired_default_endpoint`).
 */
export const RETIRED_AZURE_DEFAULT_ENDPOINT = 'https://unified-api.ucsf.edu/general';

export const RETIRED_AZURE_ENDPOINT_NOTICE =
  "This endpoint is UCSF's gateway, which older versions filled in by mistake. Enter your own Azure resource's endpoint, or use Versa API Azure for UCSF.";

/** Whether a saved endpoint is exactly the retired default (a trailing `/` aside). */
export function isRetiredAzureDefaultEndpoint(value: unknown): boolean {
  return (
    typeof value === 'string' && value.trim().replace(/\/$/, '') === RETIRED_AZURE_DEFAULT_ENDPOINT
  );
}
