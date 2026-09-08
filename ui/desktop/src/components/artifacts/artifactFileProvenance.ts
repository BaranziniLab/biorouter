import type { Message } from '../../api';
import { getTextContent } from '../../types/message';
import {
  baseToolName,
  fileArtifactPathsFromToolCall,
  looksLikePreviewableFile,
} from './artifactUtils';
import { localFileBasename, resolveFileLink } from './artifactFileLinks';

type KnownPath = { path: string; afterMessage: number };
type PathIndex = { paths: KnownPath[]; byBasename: Map<string, KnownPath[]> };
type ProvenanceIndex = PathIndex & { sessionId: string; workingDir?: string };
const provenanceCache = new WeakMap<readonly Message[], ProvenanceIndex>();

// Belt and braces for the shape `sharedSessions.normalizeSharedMessages` fills
// in at the boundary. This index is built for EVERY message of both transcript
// renderers, so it is the first thing a malformed shared transcript reaches —
// and it is where the error boundary was measured. `?.` here contradicts the
// generated `Message` type on purpose: the type is a promise about the local
// daemon's payload, not about a remote one.
function localOrigin(message: Message, sessionId: string): boolean {
  const origin = message.metadata?.provenance?.fromSessionId;
  return !origin || origin === sessionId;
}

/** Only visible references, never examples inside fenced code or hidden context. */
export function referencedFilePaths(
  text: string,
  workingDir?: string,
  plainPathPattern?: RegExp
): string[] {
  const paths = new Set<string>();
  const add = (value: string) => {
    if (!/[\\/]/.test(value) || !looksLikePreviewableFile(value)) return;
    const resolved = resolveFileLink(value, workingDir);
    if (resolved.kind === 'resolved') paths.add(resolved.path);
  };
  let prose = text
    .replace(/<info-msg>[\s\S]*?<\/info-msg>/gi, '')
    .replace(/<think>[\s\S]*?<\/think>/gi, '')
    .replace(/```[^\n]*\n[\s\S]*?```|~~~[^\n]*\n[\s\S]*?~~~/g, '');
  prose = prose.replace(/\[[^\]\n]*\]\((<[^>\n]+>|[^\s)]+)\)/g, (_match, target: string) => {
    add(target.startsWith('<') ? target.slice(1, -1) : target);
    return '';
  });
  prose = prose.replace(/(?<!`)`([^`\n]+)`(?!`)/g, (_match, target: string) => {
    add(target);
    return '';
  });
  const plainPath =
    /(?<![^\s([{])(?:file:\/\/|~[\\/]|\.{1,2}[\\/]|[a-z]:[\\/]|\/|\\\\)[^\s)\]}\x60"'<>]+\.[a-z\d]{1,12}(?::\d+|#L\d+|%[^\s)\]}\x60"'<>.,!?;]*)?(?=$|[\s)\]},;]|[.!?](?=$|[\s)\]},;]))/gi;
  for (const match of prose.matchAll(plainPathPattern ?? plainPath)) add(match[0]);
  return [...paths];
}

function buildIndex(
  messages: readonly Message[],
  sessionId: string,
  workingDir?: string
): PathIndex {
  const paths: KnownPath[] = [];
  const byBasename = new Map<string, KnownPath[]>();
  const seen = new Set<string>();
  const calls = new Map<string, { name: string; arguments: unknown }>();
  const add = (path: string, afterMessage: number) => {
    if (seen.has(path)) return;
    seen.add(path);
    const entry = { path, afterMessage };
    paths.push(entry);
    const name = localFileBasename(path);
    const matching = byBasename.get(name) ?? [];
    matching.push(entry);
    byBasename.set(name, matching);
  };

  messages.forEach((message, index) => {
    if (!localOrigin(message, sessionId)) return;
    // Read once: a message that arrives without content indexes nothing, and
    // `getTextContent` reads the same field without a guard of its own.
    const contents = message.content ?? [];
    if (message.role === 'assistant' && message.metadata?.userVisible) {
      const text = contents.length ? getTextContent(message) : '';
      for (const path of referencedFilePaths(text, workingDir)) add(path, index);
      for (const content of contents) {
        if (content.type !== 'toolRequest') continue;
        const call = content.toolCall as {
          status?: string;
          value?: { name?: string; arguments?: unknown };
        };
        if (call.status === 'success' && typeof call.value?.name === 'string') {
          calls.set(content.id, { name: call.value.name, arguments: call.value.arguments });
        }
      }
    }

    for (const content of contents) {
      if (content.type !== 'toolResponse') continue;
      const call = calls.get(content.id);
      if (!call) continue;
      calls.delete(content.id);
      // Wrapper success does not prove a statically discovered inner write ran.
      if (['shell', 'bash', 'execute_code', 'run_code'].includes(baseToolName(call.name))) continue;
      const result = content.toolResult as {
        status?: string;
        value?: { isError?: boolean; is_error?: boolean };
      };
      if (
        result.status !== 'success' ||
        !result.value ||
        result.value.isError ||
        result.value.is_error
      )
        continue;
      for (const path of fileArtifactPathsFromToolCall(call.name, call.arguments, workingDir)) {
        add(path, index);
      }
    }
  });
  return { paths, byBasename };
}

function indexForMessages(
  messages: readonly Message[],
  sessionId: string,
  workingDir?: string
): ProvenanceIndex {
  let cached = provenanceCache.get(messages);
  if (!cached || cached.sessionId !== sessionId || cached.workingDir !== workingDir) {
    cached = { sessionId, workingDir, ...buildIndex(messages, sessionId, workingDir) };
    provenanceCache.set(messages, cached);
  }
  return cached;
}

export function filePathsBeforeMessage(
  messages: readonly Message[],
  messageIndex: number,
  sessionId: string,
  workingDir?: string
): string[] {
  return indexForMessages(messages, sessionId, workingDir)
    .paths.filter(({ afterMessage }) => afterMessage < messageIndex)
    .map(({ path }) => path);
}

export function filePathLookupBeforeMessage(
  messages: readonly Message[],
  messageIndex: number,
  sessionId: string,
  workingDir?: string
): (basename: string) => readonly string[] {
  const index = indexForMessages(messages, sessionId, workingDir);
  return (basename) =>
    (index.byBasename.get(basename) ?? [])
      .filter(({ afterMessage }) => afterMessage < messageIndex)
      .map(({ path }) => path);
}
