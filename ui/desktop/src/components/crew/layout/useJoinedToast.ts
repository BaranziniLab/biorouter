import { useEffect, useRef } from 'react';
import { toastSuccess } from '../../../toasts';
import type { Snapshot } from '../crewApi';
import { buildPeopleDirectory } from '../identity';
import { workspaceTitle } from '../sidebar';
import { joinedLabel, rememberJoinerNames } from '../sidebar/sidebarView';
import { useCrew } from '../state/CrewControllerContext';
import { layoutCopy } from './copy';

interface Seen {
  connectionId: string;
  workspaceId: string;
  people: ReadonlySet<string>;
}

/** The people who are members now (removed principals stay in the list as inactive). */
function activePeople(snapshot: Snapshot): Set<string> {
  return new Set(
    snapshot.principals.filter((person) => person.active !== false).map((person) => person.id)
  );
}

/**
 * "Bob Lee (@bob) joined lab" (ui-redesign-spec, "Where errors render": toasts only for results
 * that happen off-screen). A host lets someone in and moves on; the person joins whenever their
 * own Crew next checks in. So the host hears about it once, when a verified view of the same
 * workspace shows a member the previous one did not.
 *
 * Only between two verified views of the same connection and workspace, so opening Crew,
 * switching workspaces or re-verifying after a failure never announces the people already there.
 * Display only: membership is the broker's, and this reads the snapshot it projected.
 *
 * The joiner is named as the Let in dialog names them (Q4-42): someone who has chosen no name yet
 * is "Jack Moreno (@crew_jack) joined wong-lab" when a verified view listed their server-account
 * name while they waited — it is gone from the very snapshot that shows them joined, so every
 * verified view is remembered first (`rememberJoinerNames`). Without it, "@crew_jack joined
 * wong-lab", as before.
 */
export function useJoinedToast(): void {
  const crew = useCrew();
  const verified =
    crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId
      ? crew.snapshot
      : null;
  const seen = useRef<Seen | null>(null);
  const { connectionId, isHost, connections, labels } = crew;

  useEffect(() => {
    if (!verified) return;
    rememberJoinerNames(verified);
    const people = activePeople(verified);
    const before = seen.current;
    seen.current = { connectionId, workspaceId: verified.workspace.id, people };
    if (
      !isHost ||
      !before ||
      before.connectionId !== connectionId ||
      before.workspaceId !== verified.workspace.id
    )
      return;
    const joined = [...people].filter((id) => !before.people.has(id));
    if (joined.length === 0) return;
    const dir = buildPeopleDirectory(verified, labels);
    const workspace = workspaceTitle(verified, connections, connectionId);
    for (const id of joined) {
      toastSuccess({
        msg: layoutCopy.joined(joinedLabel(id, dir, verified.workspace.id), workspace),
      });
    }
  }, [verified, connectionId, isHost, connections, labels]);
}
