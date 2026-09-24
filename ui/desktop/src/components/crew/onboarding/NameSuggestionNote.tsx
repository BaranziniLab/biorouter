import { useEffect, useState } from 'react';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import {
  displayNameIsUsername,
  personDisplayName,
  usableName,
  usePeopleDirectory,
  workspaceName,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { nameSuggestionCopy } from './copy';
import { updateJoinContext, useJoinContext } from './joinContext';

/**
 * After joining: "Use “Bob Lee” as your name in lab?" with Use and Edit…. The name on the server
 * account is offered, never applied silently (naming design D2) — until the person accepts it,
 * their display name is their username. Shown once, and only while that is still true.
 */
export function NameSuggestionNote() {
  const crew = useCrew();
  const { connectionId, snapshot, observedPrivacy, request } = crew;
  const context = useJoinContext(connectionId);
  const directory = usePeopleDirectory(snapshot, crew.labels);
  const [suggestion, setSuggestion] = useState<string | null>(null);
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  const actor = snapshot?.actor ?? null;
  const unnamed = actor
    ? displayNameIsUsername(
        personDisplayName(actor.display_name ?? actor.nickname, actor.username),
        actor.username
      )
    : false;
  const eligible = verified && Boolean(context.suggestName) && unnamed;
  const username = actor?.username ?? '';

  useEffect(() => {
    if (!eligible) return;
    const controller = new AbortController();
    request<{ full_name?: unknown }>('profile.suggest', {}, { signal: controller.signal })
      .then((result) => {
        if (controller.signal.aborted) return;
        const name = usableName(result?.full_name);
        if (name && !displayNameIsUsername(name, username)) setSuggestion(name);
        else updateJoinContext(connectionId, { suggestName: false });
      })
      .catch(() => {
        // A broker without the suggestion simply offers nothing.
        if (!controller.signal.aborted) updateJoinContext(connectionId, { suggestName: false });
      });
    return () => controller.abort();
  }, [eligible, request, connectionId, username]);

  if (!eligible || !suggestion || !snapshot) return null;
  const done = () => updateJoinContext(connectionId, { suggestName: false });
  const workspace = workspaceName(snapshot.workspace, directory.host);
  const pending = crew.isPending('mutate:profile.update');

  return (
    <Note
      tone="info"
      role="status"
      action={
        <div className="flex items-center gap-1">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={pending}
            onClick={() => {
              void crew
                .act('global', 'mutate:profile.update', () =>
                  crew.mutate('profile.update', {
                    nickname: suggestion,
                    avatar: snapshot.actor.avatar ?? null,
                  })
                )
                .then(done);
            }}
          >
            {nameSuggestionCopy.use}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => {
              done();
              crew.openDialog({ kind: 'edit-profile' });
            }}
          >
            {nameSuggestionCopy.edit}
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={done}>
            {nameSuggestionCopy.dismiss}
          </Button>
        </div>
      }
    >
      {nameSuggestionCopy.prompt(suggestion, workspace)}
    </Note>
  );
}
