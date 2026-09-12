/**
 * BR-71 §4.5: everything the subagent tab header needs, from the generated
 * client. `getSession` and `cancelTurn` are two of the functions the store
 * already imports (the `from '../api'` brace list in chatStreamStore.tsx).
 * `getSessionExtensions` is NOT in that list; it is a third generated function
 * from the same module (`sdk.gen.ts`, alongside `getSession` and `cancelTurn`).
 */
import { useCallback, useEffect, useState } from 'react';
import { cancelTurn, getSession, getSessionExtensions } from '../../api';
import { userActionHeaders } from '../../utils/userAction';

type SubagentSessionInfo = {
  isSubagent: boolean;
  parentSessionId?: string;
  spawnContext?: string;
  extensions: string[];
  stop: () => Promise<void>;
};

/** The record's section heading — matched only as a whole line, never as prose. */
const KB_HEADING = '### Knowledge bases';

/**
 * BR-71: the child's KB grants, from the one place they are recorded.
 *
 * The heading is matched ANCHORED TO A LINE START and only when it occurs
 * EXACTLY ONCE; anything else yields no grants at all. That is deliberate, and
 * it is the whole security posture of this parser.
 *
 * `persist_spawn_context` (subagent_handler.rs) interleaves the structured
 * grant sections with two blobs of parent-agent-controlled free text:
 * `### Task instructions` sits BEFORE the grants, and `### Rendered system
 * prompt` sits after them — and the latter re-embeds the former, because
 * `subagent_system.md` is rendered with `task_instructions: system_instructions`.
 * So a task string containing this heading forges a grants section on BOTH
 * sides of the real one, and neither "first match" nor "last match" is sound.
 *
 * When the record is ambiguous we therefore show NOTHING. This header exists so
 * a human can see what the child was actually granted; under-reporting a grant
 * is a visible, recoverable gap, whereas displaying a fabricated one defeats the
 * entire point of the glass box. A genuine record has exactly one such line —
 * every heading in `subagent_system.md` is single-hash, so the rendered prompt
 * never contributes a second on its own.
 */
export function extractKnowledgeBases(spawnContext?: string): string[] {
  if (!spawnContext) return [];
  const lines = spawnContext.split('\n');

  let heading = -1;
  for (let i = 0; i < lines.length; i++) {
    // `trimEnd` only: a heading indented by the writer is prose, but a trailing
    // `\r` or space is still the daemon's own line.
    if (lines[i].trimEnd() !== KB_HEADING) continue;
    if (heading !== -1) return []; // ambiguous — refuse to guess.
    heading = i;
  }
  if (heading === -1) return [];

  // The section runs to the next line-start `### ` heading, which is the same
  // boundary the backend's own `section()` helper splits on.
  const body: string[] = [];
  for (let i = heading + 1; i < lines.length && !lines[i].startsWith('### '); i++) {
    body.push(lines[i]);
  }

  const section = body.join('\n').trim();
  if (!section || section === '(none)') return [];
  return section
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
}

export function useSubagentSession(sessionId: string): SubagentSessionInfo {
  const [info, setInfo] = useState<Omit<SubagentSessionInfo, 'stop'>>({
    isSubagent: false,
    extensions: [],
  });

  useEffect(() => {
    // Drop the previous child BEFORE anything is awaited. ChatGroupsShell keys
    // BaseChat by TAB id, not session id — the session behind a tab is
    // explicitly rebindable — so one instance of this hook outlives a sessionId
    // change. Without this, the early returns below leave the old child's
    // lineage, grants and Stop button rendered over a session they have nothing
    // to do with, and even a subagent→subagent switch would show the previous
    // child's spawn context for the length of the fetch. A functional update, so
    // an already-clear state costs no re-render.
    setInfo((prev) => (prev.isSubagent ? { isSubagent: false, extensions: [] } : prev));

    let cancelled = false;
    (async () => {
      // BaseChat mounts with an empty sessionId before the sidebar's "New
      // Session" has created one; a GET /sessions/ then 404s on every such
      // mount. There is nothing to look up, so do not ask.
      if (!sessionId) return;
      const session = (
        await getSession({
          path: { session_id: sessionId },
          // Issue #56 Task 58: reading a private chat needs the proof-of-user.
          headers: await userActionHeaders(),
        })
      ).data;
      if (cancelled || !session || session.session_type !== 'sub_agent') return;
      // The spawn-context record: first message stamped provenance spawn_context
      // (Task 32). Casing verified against the generated client: `MessageMetadata`
      // is camelCase with `provenance?: MessageProvenance | null`,
      // `MessageProvenance` is `{ fromSessionId?, fromSessionName?, kind }`, and
      // `ProvenanceKind` is the snake_case union
      // `'agent_injection' | 'user_direct' | 'spawn_context'`.
      const record = (session.conversation ?? []).find(
        (m) => m?.metadata?.provenance?.kind === 'spawn_context'
      );
      const spawnContext = record?.content?.map((c) => ('text' in c ? c.text : '')).join('\n');
      const extensionsResponse = (
        await getSessionExtensions({
          path: { session_id: sessionId },
          headers: await userActionHeaders(),
        })
      ).data;
      if (cancelled) return;
      setInfo({
        isSubagent: true,
        parentSessionId: session.parent_session_id ?? undefined,
        spawnContext,
        extensions: (extensionsResponse?.extensions ?? []).map((e) => e.name),
      });
    })().catch(() => {
      /* a failed load renders no header — never breaks the chat */
    });
    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  const stop = useCallback(async () => {
    await cancelTurn({
      body: { session_id: sessionId },
      headers: await userActionHeaders(),
    });
  }, [sessionId]);

  return { ...info, stop };
}
