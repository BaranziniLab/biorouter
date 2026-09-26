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
import {
  dismissNameOffer,
  noNameToOffer,
  offeredName,
  rememberOfferedName,
  useJoinContext,
} from './joinContext';

/** An open offer of the server-account name, and the three answers to it. */
export interface NameSuggestion {
  /** The name on the person's server account. */
  name: string;
  /** The workspace, as a sentence names it. */
  workspace: string;
  /** Use is saving. */
  pending: boolean;
  /** Save the name as this person's display name; the offer closes once that succeeded. */
  use(): void;
  /** Close the offer and open Edit profile, where the same name is prefilled. */
  edit(): void;
  /** Close the offer without changing anything. */
  dismiss(): void;
}

/**
 * The server-account name offer (naming design D2), for whichever surface shows it: the composer's
 * note and the host's setup checklist. Offered, never applied silently — until the person accepts
 * it, their display name is their username.
 *
 * Eligible while the workspace has verified this connection, the person's display name is still
 * their username, and they have not answered the offer on this computer (`joinContext` derives
 * that for every connection, so the host and people who joined before names existed are offered it
 * too, Q3-51). The name itself comes from the broker (`profile.suggest`); a broker with no usable
 * name offers nothing, for this session. `enabled` false asks nothing (a surface that would not
 * show the offer).
 */
export function useNameSuggestion(enabled = true): NameSuggestion | null {
  const crew = useCrew();
  const { connectionId, snapshot, observedPrivacy, request } = crew;
  const context = useJoinContext(connectionId);
  const directory = usePeopleDirectory(snapshot, crew.labels);
  // Keyed on the connection and account it was fetched for, so another connection's answer is
  // never offered here.
  const [fetched, setFetched] = useState<{ key: string; name: string } | null>(null);
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  const actor = snapshot?.actor ?? null;
  const unnamed = actor
    ? displayNameIsUsername(
        personDisplayName(actor.display_name ?? actor.nickname, actor.username),
        actor.username
      )
    : false;
  const eligible = enabled && verified && Boolean(context.suggestName) && unnamed;
  const username = actor?.username ?? '';
  const key = `${connectionId}:${username}`;
  // A surface that mounts again (another channel, the checklist after the composer) offers at once.
  const known = offeredName(connectionId, username);

  useEffect(() => {
    if (!eligible || known) return;
    const controller = new AbortController();
    request<{ full_name?: unknown }>('profile.suggest', {}, { signal: controller.signal })
      .then((result) => {
        if (controller.signal.aborted) return;
        const name = usableName(result?.full_name);
        if (name && !displayNameIsUsername(name, username)) {
          rememberOfferedName(connectionId, username, name);
          setFetched({ key, name });
        } else noNameToOffer(connectionId);
      })
      .catch(() => {
        // A broker without the suggestion simply offers nothing.
        if (!controller.signal.aborted) noNameToOffer(connectionId);
      });
    return () => controller.abort();
  }, [eligible, known, request, connectionId, username, key]);

  const name = known ?? (fetched?.key === key ? fetched.name : null);
  if (!eligible || !name || !snapshot) return null;
  const avatar = snapshot.actor.avatar ?? null;
  return {
    name,
    workspace: workspaceName(snapshot.workspace, directory.host),
    pending: crew.isPending('mutate:profile.update'),
    use: () => {
      void crew
        .act('global', 'mutate:profile.update', async () => {
          await crew.mutate('profile.update', { nickname: name, avatar });
          return true;
        })
        // A failed save leaves the offer open, beside the error, to try again.
        .then((saved) => {
          if (saved) dismissNameOffer(connectionId);
        });
    },
    edit: () => {
      dismissNameOffer(connectionId);
      crew.openDialog({ kind: 'edit-profile' });
    },
    dismiss: () => dismissNameOffer(connectionId),
  };
}

/**
 * "Use “Bob Lee” as your name in lab?" with Use, Edit… and Dismiss, above the composer. Shown until
 * the person answers it, and only while their display name is still their username.
 */
export function NameSuggestionNote() {
  const suggestion = useNameSuggestion();
  if (!suggestion) return null;
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
            disabled={suggestion.pending}
            onClick={suggestion.use}
          >
            {nameSuggestionCopy.use}
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={suggestion.edit}>
            {nameSuggestionCopy.edit}
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={suggestion.dismiss}>
            {nameSuggestionCopy.dismiss}
          </Button>
        </div>
      }
    >
      {nameSuggestionCopy.prompt(suggestion.name, suggestion.workspace)}
    </Note>
  );
}
