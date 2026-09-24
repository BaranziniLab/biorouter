import { useCallback, useEffect, useState } from 'react';

/**
 * Which team sections this viewer collapsed, per saved connection.
 *
 * A per-viewer convenience, so it lives in `localStorage` — and every read and write is wrapped,
 * because storage can be absent, full or throwing (a private window, cleared site data), and a
 * sidebar that cannot remember a collapse must still render expanded rather than fail.
 */
export const COLLAPSED_TEAMS_STORAGE_KEY = 'biorouter:crew:collapsed-teams';

type Stored = Record<string, string[]>;

function readAll(): Stored {
  try {
    const raw = window.localStorage?.getItem(COLLAPSED_TEAMS_STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed as Record<string, unknown>).map(([connection, teams]) => [
        connection,
        Array.isArray(teams)
          ? teams.filter((team): team is string => typeof team === 'string')
          : [],
      ])
    );
  } catch {
    return {};
  }
}

function read(connectionId: string): Set<string> {
  return new Set(connectionId ? (readAll()[connectionId] ?? []) : []);
}

function write(connectionId: string, collapsed: Set<string>) {
  if (!connectionId) return;
  try {
    const all = readAll();
    if (collapsed.size === 0) delete all[connectionId];
    else all[connectionId] = [...collapsed];
    window.localStorage?.setItem(COLLAPSED_TEAMS_STORAGE_KEY, JSON.stringify(all));
  } catch {
    // Remembering is a convenience; the collapse itself already happened.
  }
}

export interface CollapsedTeams {
  isCollapsed(teamId: string): boolean;
  setCollapsed(teamId: string, collapsed: boolean): void;
}

export function useCollapsedTeams(connectionId: string): CollapsedTeams {
  const [state, setState] = useState(() => ({ connectionId, teams: read(connectionId) }));

  // A connection switch reads that connection's own choices.
  useEffect(() => {
    setState((current) =>
      current.connectionId === connectionId ? current : { connectionId, teams: read(connectionId) }
    );
  }, [connectionId]);

  const teams = state.connectionId === connectionId ? state.teams : read(connectionId);

  const setCollapsed = useCallback(
    (teamId: string, collapsed: boolean) => {
      setState((current) => {
        const base = current.connectionId === connectionId ? current.teams : read(connectionId);
        if (base.has(teamId) === collapsed) return current;
        const next = new Set(base);
        if (collapsed) next.add(teamId);
        else next.delete(teamId);
        write(connectionId, next);
        return { connectionId, teams: next };
      });
    },
    [connectionId]
  );

  return { isCollapsed: (teamId) => teams.has(teamId), setCollapsed };
}
