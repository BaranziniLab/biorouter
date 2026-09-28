/**
 * The settings that decide where a provider sends its requests, and with them
 * its saved key or this computer's own sign-in: hosts, endpoints, base paths,
 * the Vertex AI location, the AWS region, profile and endpoint overrides, the
 * certificate paths and the coding-agent commands.
 *
 * The daemon refuses to change one of these over HTTP without the proof that a
 * person asked (`destination_change_refusal` in
 * `crates/biorouter-server/src/routes/config_management.rs`). The desktop app
 * sends that proof with every settings write. A browser served by
 * `biorouter serve` can never send it, so there these settings belong to the
 * computer running Biorouter, and Settings says so beside the field instead of
 * letting a Save meet the refusal.
 *
 * A mirror of `DESTINATION_CONFIG_KEYS` and `DESTINATION_KEY_PREFIXES` in
 * `crates/biorouter/src/providers/destination_keys.rs`. A test there reads this
 * file and fails when the two disagree, so a key added on one side only does
 * not ship.
 */
export const DESTINATION_CONFIG_KEYS: readonly string[] = [
  'ANTHROPIC_HOST',
  'AZURE_OPENAI_ENDPOINT',
  'DATABRICKS_HOST',
  'GOOGLE_HOST',
  'LITELLM_HOST',
  'OPENAI_HOST',
  'OPENROUTER_HOST',
  'SNOWFLAKE_HOST',
  'TETRATE_HOST',
  'VENICE_HOST',
  'VERSA_AZURE_ENDPOINT',
  'VERSA_BEDROCK_ENDPOINT',
  'XAI_HOST',
  'XIAOMI_MIMO_HOST',
  'ZAI_HOST',
  'OPENAI_BASE_PATH',
  'LITELLM_BASE_PATH',
  'VENICE_BASE_PATH',
  'VENICE_MODELS_PATH',
  'GCP_LOCATION',
  'AWS_REGION',
  'AWS_ENDPOINT_URL',
  'AWS_ENDPOINT_URL_BEDROCK_RUNTIME',
  'AWS_ENDPOINT_URL_SAGEMAKER_RUNTIME',
  'AWS_PROFILE',
  'BIOROUTER_CA_CERT_PATH',
  'CLAUDE_CODE_COMMAND',
  'CODEX_COMMAND',
  'BIOROUTER_CLIENT_CERT_PATH',
  'BIOROUTER_CLIENT_KEY_PATH',
];

/** Name prefixes that make a key a destination key whatever follows. */
export const DESTINATION_CONFIG_KEY_PREFIXES: readonly string[] = ['AWS_ENDPOINT_URL_'];

/**
 * Would the daemon refuse to change `key` without the proof of a person?
 * Compared in upper case, as the daemon compares it.
 */
export function isDestinationConfigKey(key: string): boolean {
  const upper = key.toUpperCase();
  return (
    DESTINATION_CONFIG_KEYS.includes(upper) ||
    DESTINATION_CONFIG_KEY_PREFIXES.some((prefix) => upper.startsWith(prefix))
  );
}
