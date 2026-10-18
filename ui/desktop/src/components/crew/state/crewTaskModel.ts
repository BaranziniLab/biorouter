import type { ProviderDetails } from '../../../api';
import { CODING_AGENT_ORDER } from '../../onboarding/codingAgentStatus';

/**
 * The model Crew tasks start on (design OW-11): the config key `CREW_TASK_MODEL`, an object
 * `{provider, model}` in `config.yaml`, written by Settings › Models › Crew tasks and by the pane's
 * "Use for future tasks", and read by the CLI the same way (`/config/read` on the shared daemon).
 *
 * A preference, not a capability key: the daemon never reads it. Every task names its provider and
 * model in a request a person proves, and the daemon and the broker admit each one, so a default
 * that cannot run somewhere is refused there exactly as a model chosen in the pane would be.
 *
 * The CLI reads and writes the same key through the same daemon (`biorouter crew defaults`), with
 * the same fallback, so the terminal and the app start a task on one model.
 */
export const CREW_TASK_MODEL_KEY = 'CREW_TASK_MODEL';

/** A provider and one of its models, as the run request names them. */
export interface TaskModelChoice {
  provider: string;
  model: string;
}

/** Where the model a Crew task starts on came from: the Crew setting, or the app's default. */
export type TaskModelSource = 'crew' | 'app';

export interface ResolvedTaskModel {
  choice: TaskModelChoice;
  source: TaskModelSource;
}

const text = (value: unknown): string => (typeof value === 'string' ? value.trim() : '');

/**
 * `CREW_TASK_MODEL` as stored, or `null` unless both fields are non-blank strings: a half-written
 * value is "unset", never half a choice.
 */
export function parseTaskModel(value: unknown): TaskModelChoice | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const provider = text((value as { provider?: unknown }).provider);
  const model = text((value as { model?: unknown }).model);
  return provider && model ? { provider, model } : null;
}

/**
 * Whether a configured provider can run a Crew task at all. Claude Code and Codex never can: the
 * daemon refuses a tool-bridge provider for every Crew task (`uses_tool_bridge`), so offering one
 * as the default would make every `@agent` fail.
 */
export function crewTaskModelUsable(provider: ProviderDetails | undefined): boolean {
  return (
    provider !== undefined &&
    provider.is_configured === true &&
    !(CODING_AGENT_ORDER as readonly string[]).includes(provider.name)
  );
}

/**
 * The model a Crew task starts on: `CREW_TASK_MODEL` when it names a usable configured provider,
 * else the app's `BIOROUTER_PROVIDER` / `BIOROUTER_MODEL` under the same rule, else `null`. The
 * source says which, so Settings can say "Same as new chats".
 */
export function resolveTaskModel({
  crew,
  appProvider,
  appModel,
  configured,
}: {
  crew: unknown;
  appProvider: unknown;
  appModel: unknown;
  configured: readonly ProviderDetails[];
}): ResolvedTaskModel | null {
  const usable = (choice: TaskModelChoice | null) =>
    choice !== null &&
    crewTaskModelUsable(configured.find((item) => item.name === choice.provider));
  const own = parseTaskModel(crew);
  if (own && usable(own)) return { choice: own, source: 'crew' };
  const app = parseTaskModel({ provider: appProvider, model: appModel });
  if (app && usable(app)) return { choice: app, source: 'app' };
  return null;
}
