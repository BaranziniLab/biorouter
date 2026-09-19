import type { Message } from '../api';
import type { ArtifactSource } from '../components/artifacts/artifactTypes';
import { baseToolName, fileArtifactPathsFromToolCall } from '../components/artifacts/artifactUtils';

export type ArtifactRefreshEvent = {
  id: string;
  paths: string[];
  appId?: string;
  checkActiveFile?: boolean;
};

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function refreshedAppId(
  name: string,
  args: Record<string, unknown> | undefined
): string | undefined {
  if (!['build_app', 'update_app', 'configure_app'].includes(name)) return undefined;
  const visibleUpdate =
    name !== 'update_app' ||
    args?.path == null ||
    args.path === 'index.html' ||
    args.path === 'manifest.json' ||
    (typeof args.path === 'string' && /^(?:dist|assets)\//.test(args.path));
  return visibleUpdate && typeof args?.id === 'string' ? args.id : undefined;
}

/** A refresh hint, never permission to open a new file or navigate a new URL. */
export function artifactRefreshEvents(
  messages: readonly Message[],
  sessionId: string,
  workingDir?: string
): ArtifactRefreshEvent[] {
  const calls = new Map<string, { name: string; args: unknown }>();
  const events = new Map<string, ArtifactRefreshEvent>();
  const localMessages = messages.filter((message) => {
    const origin = message.metadata?.provenance?.fromSessionId;
    return !origin || origin === sessionId;
  });
  for (const message of localMessages) {
    for (const content of message.content) {
      if (content.type !== 'toolRequest' || content.toolCall.status !== 'success') continue;
      const call = record(content.toolCall.value);
      if (typeof call?.name === 'string') {
        calls.set(content.id, { name: call.name, args: call.arguments });
      }
    }
  }
  for (const message of localMessages) {
    for (const content of message.content) {
      if (content.type !== 'toolResponse' || content.toolResult.status !== 'success') continue;
      const call = calls.get(content.id);
      const result = record(content.toolResult.value);
      if (!call || !result || result.isError === true || result.is_error === true) continue;
      const name = baseToolName(call.name);
      const args = record(call.args);
      // An opaque execution can modify the already-open file. Re-read that file
      // only: command text is not evidence that a guessed output path was written.
      const desktopMutation =
        /^computercontroller__(?:click|perform_secondary_action|scroll|drag|type_text|press_key|set_value)$/.test(
          call.name
        );
      if (desktopMutation || ['shell', 'bash', 'execute_code', 'run_code'].includes(name)) {
        events.set(content.id, { id: content.id, paths: [], checkActiveFile: true });
        if (name === 'execute_code' || name === 'run_code') {
          const executedCalls = record(result._meta)?.['biorouter/tool-calls'];
          if (Array.isArray(executedCalls)) {
            for (const value of executedCalls) {
              const nested = record(value);
              if (
                nested?.status !== 'ok' ||
                typeof nested.tool !== 'string' ||
                !/^agent_drafter__(?:build_app|update_app|configure_app)$/.test(nested.tool) ||
                typeof nested.args !== 'string'
              )
                continue;
              let nestedArgs: Record<string, unknown> | undefined;
              try {
                nestedArgs = record(JSON.parse(nested.args));
              } catch {
                // Executed-call telemetry is bounded and can contain truncated JSON.
                continue;
              }
              const appId = refreshedAppId(baseToolName(nested.tool), nestedArgs);
              if (appId !== undefined) {
                const id = JSON.stringify(['nested-app', content.id, appId]);
                events.set(id, { id, paths: [], appId });
              }
            }
          }
        }
        continue;
      }
      if (name === 'build_app' || name === 'update_app' || name === 'configure_app') {
        const appId = refreshedAppId(name, args);
        if (appId !== undefined) {
          events.set(content.id, { id: content.id, paths: [], appId });
        }
        continue;
      }
      // Undo changes an open file, although the artifact-discovery helper rightly
      // excludes it from creating a new preview tab.
      const mutationArgs =
        args?.command === 'undo_edit' && ['text_editor', 'str_replace_editor'].includes(name)
          ? { ...args, command: 'write' }
          : call.args;
      const paths = fileArtifactPathsFromToolCall(call.name, mutationArgs, workingDir);
      if (paths.length) events.set(content.id, { id: content.id, paths });
    }
  }
  return [...events.values()];
}

export function artifactRefreshTarget(artifact: ArtifactSource | null): string | null {
  if (artifact?.kind === 'file') return `file:${artifact.path}`;
  if (artifact?.kind !== 'externalUrl') return null;
  try {
    const url = new URL(artifact.url);
    const match = /^\/apps\/([A-Za-z0-9_-]+)\/?$/.exec(url.pathname);
    // Main-process provenance remains authoritative; this only narrows UI hints.
    return url.protocol === 'http:' && url.hostname === '127.0.0.1' && match
      ? `app:${match[1]}`
      : null;
  } catch {
    return null;
  }
}

export function refreshEventMatches(event: ArtifactRefreshEvent, target: string): boolean {
  if (target.startsWith('app:')) return event.appId === target.slice(4);
  return (
    target.startsWith('file:') &&
    (event.checkActiveFile === true || event.paths.includes(target.slice(5)))
  );
}
