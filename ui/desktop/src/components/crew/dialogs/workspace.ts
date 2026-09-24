import { useMemo } from 'react';
import type { CrewConnection, Snapshot } from '../crewApi';
import {
  connectionNames,
  connectionServer,
  isMachineIdShaped,
  sanitizeDisplayText,
  usePeopleDirectory,
  workspaceName,
  type PeopleDirectory,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';

/**
 * What every dialog reads besides its own intent: the snapshot to name things from, the people in
 * it, and the words for the workspace and the server.
 *
 * The snapshot is the verified one, else the last verified view of the SAME connection. That view
 * is presentation only (a refresh blanks the live snapshot for a moment, and a dialog that stays
 * open across it must not lose its labels); nothing a dialog sends is decided by it — every
 * request goes to the daemon and broker, which decide.
 */

/**
 * `{workspace}` in the copy deck: the workspace's S2 name, else the saved connection's local name
 * (`name — server` when two share one), else "{host}'s workspace".
 */
export function workspaceLabelFor(
  connections: readonly CrewConnection[],
  connectionId: string,
  snapshot: Snapshot | null,
  dir?: PeopleDirectory | null
): string {
  const named = sanitizeDisplayText(snapshot?.workspace?.name);
  if (named && !isMachineIdShaped(named)) return named;
  const local = connectionNames(connections).get(connectionId);
  if (local) return local;
  return workspaceName(snapshot?.workspace ?? null, dir?.host ?? null);
}

/**
 * The phrase a typed confirmation asks for: the workspace's own name — its S2 name, else the saved
 * connection's name — never the `name — server` label two same-named connections get, which would
 * ask the person to type an em dash. Falls back to the label only when neither exists.
 */
export function workspacePhraseFor(
  connections: readonly CrewConnection[],
  connectionId: string,
  snapshot: Snapshot | null,
  dir?: PeopleDirectory | null
): string {
  const named = sanitizeDisplayText(snapshot?.workspace?.name);
  if (named && !isMachineIdShaped(named)) return named;
  const saved = sanitizeDisplayText(connections.find((item) => item.id === connectionId)?.name);
  if (saved && !isMachineIdShaped(saved)) return saved;
  return workspaceLabelFor(connections, connectionId, snapshot, dir);
}

/**
 * Whether the broker speaks the S2 naming rules (unique names, renames). The daemon does not yet
 * surface the `hello` capability `unique_names_v1` to the renderer, so this reads the one
 * projection only an S2 broker sends: a `handle` on every team and channel.
 */
export function uniqueNamesSupported(snapshot: Snapshot | null | undefined): boolean {
  if (!snapshot) return false;
  const objects = [...(snapshot.teams ?? []), ...(snapshot.channels ?? [])];
  return objects.some((object) => typeof object.handle === 'string' && object.handle.length > 0);
}

export interface DialogView {
  crew: CrewController;
  /** The verified snapshot, else the last verified view of the selected connection. */
  snapshot: Snapshot | null;
  dir: PeopleDirectory;
  workspace: string;
  /** What a typed confirmation asks for: the workspace's own name. */
  phrase: string;
  /** The server the selected connection reaches, without its `user@`. */
  server: string;
}

export function useDialogView(connectionId?: string): DialogView {
  const crew = useCrew();
  const id = connectionId ?? crew.connectionId;
  const snapshot =
    id === crew.connectionId ? (crew.snapshot ?? crew.lastVerified?.snapshot ?? null) : null;
  const dir = usePeopleDirectory(snapshot, crew.labels);
  const workspace = useMemo(
    () => workspaceLabelFor(crew.connections, id, snapshot, dir),
    [crew.connections, id, snapshot, dir]
  );
  const phrase = useMemo(
    () => workspacePhraseFor(crew.connections, id, snapshot, dir),
    [crew.connections, id, snapshot, dir]
  );
  const saved = crew.connections.find((item) => item.id === id) ?? null;
  return { crew, snapshot, dir, workspace, phrase, server: connectionServer(saved) };
}
