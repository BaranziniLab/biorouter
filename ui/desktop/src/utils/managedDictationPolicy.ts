import fs from 'node:fs';
import path from 'node:path';
// js-yaml 5 has no default export in its ESM build: `yaml.load` threw there, and the reader's
// catch read every policy file as denying. The named `load` is the safe schema.
import { load } from 'js-yaml';

export type LaptopDictationPolicy = { enabled: boolean; capture: boolean };

export const MANAGED_DICTATION_PATHS = {
  darwin: '/Library/Application Support/Biorouter/managed-policy.yaml',
  linux: '/etc/biorouter/managed-policy.yaml',
} as const;

export function managedPolicyPath(
  env: typeof process.env = process.env,
  platform: typeof process.platform = process.platform
): string | null {
  const root = env.BIOROUTER_PATH_ROOT;
  if (root?.trim()) return path.join(root, 'managed', 'managed-policy.yaml');
  if (platform === 'darwin') return MANAGED_DICTATION_PATHS.darwin;
  if (platform === 'linux') return MANAGED_DICTATION_PATHS.linux;
  if (platform === 'win32')
    return env.ProgramData ? path.join(env.ProgramData, 'Biorouter', 'managed-policy.yaml') : null;
  return null;
}

const ALLOWED: LaptopDictationPolicy = { enabled: true, capture: true };
const DENIED: LaptopDictationPolicy = { enabled: false, capture: false };

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/** The laptop reads only its two capture doors. The backend reads its own full policy. */
export function readLaptopDictationPolicy(
  env: typeof process.env = process.env,
  platform: typeof process.platform = process.platform
): LaptopDictationPolicy {
  const filename = managedPolicyPath(env, platform);
  if (!filename) return ALLOWED;
  let raw: string;
  try {
    raw = fs.readFileSync(filename, 'utf8');
  } catch (error) {
    if ((error as { code?: string }).code === 'ENOENT') return ALLOWED;
    return DENIED;
  }
  try {
    const document: unknown = load(raw);
    if (!record(document)) return DENIED;
    if (document.dictation === undefined) return ALLOWED;
    if (!record(document.dictation)) return DENIED;
    const block = document.dictation;
    const knownKeys = new Set([
      'enabled',
      'capture',
      'model_downloads',
      'allowed_models',
      'models_dir',
      'external_engine',
      'dictionary',
      'dictionary_agent_writes',
    ]);
    if (Object.keys(block).some((key) => !knownKeys.has(key))) return DENIED;
    if (block.enabled !== undefined && typeof block.enabled !== 'boolean') return DENIED;
    if (block.capture !== undefined && typeof block.capture !== 'boolean') return DENIED;
    return {
      enabled: block.enabled !== false,
      capture: block.enabled !== false && block.capture !== false,
    };
  } catch {
    return DENIED;
  }
}
