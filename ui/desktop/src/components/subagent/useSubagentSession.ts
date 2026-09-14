/**
 * BR-71 §4.5: everything the subagent tab header needs.
 *
 * ⚠ **The chat's row comes from the chat store, never from a read of its own.**
 * The store (`hooks/chatStreamStore.tsx`) is the one owner of a loaded chat: its
 * `/agent/resume` answers with the whole row (`get_session(id, true)`, transcript
 * included), and in a browser a subagent's chat is loaded by the store's own
 * `GET /sessions/{id}` (`loadReadOnlySubagentChat`). This hook used to ask again,
 * at mount, and that read could share a request with the composer's only by
 * accident of timing — measured under `biorouter serve` on 2026-09-13, it went out
 * at 16 ms while the composer, which a browser withholds until the store's row
 * has landed (`composerSlotMode`), read the same row 55–290 ms later: two requests
 * on almost every open. Everything it needs — `session_type`,
 * `parent_session_id`, the spawn-context record — is already in the store's row,
 * and none of it changes over a chat's life. The one read left is
 * `GET /sessions/{id}/extensions`, for a subagent's chat only.
 */
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { cancelTurn, getSessionExtensions, type Message } from '../../api';
import { useChatStreamController } from '../../hooks/chatStreamStore';
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

/**
 * The spawn-context record: the first message stamped provenance `spawn_context`
 * (Task 32). Casing verified against the generated client: `MessageMetadata` is
 * camelCase with `provenance?: MessageProvenance | null`, `MessageProvenance` is
 * `{ fromSessionId?, fromSessionName?, kind }`, and `ProvenanceKind` is the
 * snake_case union `'agent_injection' | 'user_direct' | 'spawn_context'`.
 */
function spawnContextOf(conversation: Message[] | null | undefined): string | undefined {
  const record = (conversation ?? []).find(
    (m) => m?.metadata?.provenance?.kind === 'spawn_context'
  );
  return record?.content?.map((c) => ('text' in c ? c.text : '')).join('\n');
}

export function useSubagentSession(sessionId: string): SubagentSessionInfo {
  // The same controller `useChatStream` holds for this tab — `getController` is a
  // create-or-get on the one registry, so this subscribes and loads nothing.
  const controller = useChatStreamController(sessionId);
  const loaded = useSyncExternalStore(
    controller.subscribe,
    () => controller.getSnapshot().session,
    () => controller.getSnapshot().session
  );
  // ⚠ Compared, not assumed. `ChatGroupsShell` keys BaseChat by TAB id and the
  // session behind a tab is rebindable, so a row for any other id is not this
  // tab's answer — and the previous child's lineage, grants and Stop button must
  // not stay rendered over a chat they have nothing to do with.
  const child =
    sessionId !== '' && loaded?.id === sessionId && loaded.session_type === 'sub_agent'
      ? loaded
      : undefined;
  const childId = child?.id;
  const conversation = child?.conversation;
  const spawnContext = useMemo(() => spawnContextOf(conversation), [conversation]);

  // The child's extension grants — the one thing the row does not carry.
  // Tagged with the id they were read for, so a grant list can never be shown
  // over another chat.
  const [grants, setGrants] = useState<{ sessionId: string; extensions: string[] } | null>(null);
  useEffect(() => {
    if (!childId) return;
    let cancelled = false;
    (async () => {
      const response = (
        await getSessionExtensions({
          path: { session_id: childId },
          // Issue #56 Task 58: reading a private chat needs the proof-of-user.
          headers: await userActionHeaders(),
        })
      ).data;
      if (cancelled) return;
      setGrants({
        sessionId: childId,
        extensions: (response?.extensions ?? []).map((e) => e.name),
      });
    })().catch(() => {
      /* a failed load renders no header — never breaks the chat */
    });
    return () => {
      cancelled = true;
    };
  }, [childId]);

  const stop = useCallback(async () => {
    await cancelTurn({
      body: { session_id: sessionId },
      headers: await userActionHeaders(),
    });
  }, [sessionId]);

  // The header appears once everything it states is in, exactly as before: a
  // grants row that reads "no extensions" until the list lands would be the
  // glass box stating something false.
  if (!child || grants?.sessionId !== child.id) {
    return { isSubagent: false, extensions: NO_EXTENSIONS, stop };
  }
  return {
    isSubagent: true,
    parentSessionId: child.parent_session_id ?? undefined,
    spawnContext,
    extensions: grants.extensions,
    stop,
  };
}

const NO_EXTENSIONS: string[] = [];
