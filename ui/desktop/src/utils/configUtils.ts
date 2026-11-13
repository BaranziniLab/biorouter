export const configLabels: Record<string, string> = {
  // biorouter settings
  BIOROUTER_PROVIDER: 'Provider',
  BIOROUTER_MODEL: 'Model',
  BIOROUTER_TEMPERATURE: 'Temperature',
  BIOROUTER_MODE: 'Mode',
  BIOROUTER_WORKER_PROVIDER: 'Default Worker Provider',
  BIOROUTER_WORKER_MODEL: 'Default Worker Model',
  BIOROUTER_PLANNER_PROVIDER: 'Planner Provider',
  BIOROUTER_PLANNER_MODEL: 'Planner Model',
  BIOROUTER_TOOLSHIM: 'Tool Shim',
  BIOROUTER_TOOLSHIM_OLLAMA_MODEL: 'Tool Shim Ollama Model',
  BIOROUTER_CLI_MIN_PRIORITY: 'CLI Min Priority',
  BIOROUTER_ALLOWLIST: 'Allow List',
  BIOROUTER_WORKFLOW_GITHUB_REPO: 'Workflow GitHub Repo',

  // openai
  OPENAI_API_KEY: 'OpenAI API Key',
  OPENAI_HOST: 'OpenAI Host',
  OPENAI_BASE_PATH: 'OpenAI Base Path',

  // groq
  GROQ_API_KEY: 'Groq API Key',

  // xiaomi mimo
  XIAOMI_MIMO_API_KEY: 'Xiaomi MiMo API Key',
  XIAOMI_MIMO_HOST: 'Xiaomi MiMo Host (region endpoint)',

  // zai (Zhipu GLM)
  ZAI_API_KEY: 'z.ai API Key',
  ZAI_HOST: 'z.ai Host',

  // openrouter
  OPENROUTER_API_KEY: 'OpenRouter API Key',

  // anthropic
  ANTHROPIC_API_KEY: 'Anthropic API Key',
  ANTHROPIC_HOST: 'Anthropic Host',

  // google
  GOOGLE_API_KEY: 'Google API Key',

  // databricks
  DATABRICKS_HOST: 'Databricks Host',

  // ollama
  OLLAMA_HOST: 'Ollama Host',

  // llama server (llama.cpp sidecar)
  LLAMACPP_PORT: 'Llama Server Port',
  LLAMACPP_CONTEXT_SIZE: 'Llama Server Context Size',
  LLAMACPP_TIMEOUT: 'Llama Server Timeout',
  LLAMACPP_ENABLE_THINKING: 'Llama Server Thinking Mode',
  LLAMACPP_EXTERNAL_HOST: 'Llama Server External Host',

  // azure openai
  AZURE_OPENAI_API_KEY: 'Azure OpenAI API Key',
  AZURE_OPENAI_ENDPOINT: 'Azure OpenAI Endpoint',
  AZURE_OPENAI_DEPLOYMENT_NAME: 'Azure OpenAI Deployment Name',
  AZURE_OPENAI_API_VERSION: 'Azure OpenAI API Version',

  // gcp vertex
  GCP_PROJECT_ID: 'GCP Project ID',
  GCP_LOCATION: 'GCP Location',

  // snowflake
  SNOWFLAKE_HOST: 'Snowflake Host',
  SNOWFLAKE_TOKEN: 'Snowflake Token',

  // claude agent (drives the installed claude CLI)
  CLAUDE_CODE_COMMAND: 'Claude CLI Command',

  // codex (drives the installed codex CLI)
  CODEX_COMMAND: 'Codex CLI Command',
};

export const providerPrefixes: Record<string, string[]> = {
  openai: ['OPENAI_'],
  anthropic: ['ANTHROPIC_'],
  google: ['GOOGLE_'],
  groq: ['GROQ_'],
  xiaomi_mimo: ['XIAOMI_MIMO_'],
  zai: ['ZAI_'],
  databricks: ['DATABRICKS_'],
  openrouter: ['OPENROUTER_'],
  ollama: ['OLLAMA_'],
  llamacpp: ['LLAMACPP_'],
  azure_openai: ['AZURE_'],
  gcp_vertex_ai: ['GCP_'],
  snowflake: ['SNOWFLAKE_'],
  claude_code: ['CLAUDE_CODE_'],
  codex: ['CODEX_'],
};

export const getUiNames = (key: string): string => {
  if (configLabels[key]) {
    return configLabels[key];
  }
  return key
    .split('_')
    .map((word) => word.charAt(0) + word.slice(1).toLowerCase())
    .join(' ');
};
