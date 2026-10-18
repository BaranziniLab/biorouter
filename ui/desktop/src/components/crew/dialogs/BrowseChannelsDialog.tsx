import * as React from 'react';
import { ModalShell } from '../../ModalShell';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import type { CrewJoinable } from '../crewApi';
import { channelName, teamName } from '../identity';
import type { ErrorSource } from '../state/types';
import { browseCopy as copy } from './copy';
import { DialogErrorNote } from './fields';
import { isOpenWorkspace } from './governance';
import { useDialogView } from './workspace';
import './dialogs.css';

const SOURCE: ErrorSource = 'dialog:browse-channels';
const TEAM_JOIN_KEY = 'mutate:team.join';
const CHANNEL_JOIN_KEY = 'mutate:channel.join';

export interface BrowseChannelsDialogProps {
  /** Only this team's channels; without it, every joinable team and channel. */
  teamId?: string;
  onClose(): void;
}

/**
 * Browse channels (OW-2): in an open workspace, the teams the person is not in and the channels of
 * their teams they are not in, each with Join. The lists are the broker's own `joinable_teams` and
 * `joinable_channels`, which it sends only in an open workspace, so a managed workspace's dialog
 * says there is nothing to join rather than offering what would be refused.
 *
 * Joining is the person's own act and widens nothing an agent holds (OW-3: additions in an open
 * workspace do not end grants). One join at a time: every Join waits while one is on its way.
 * A join opens what was joined, the team at its #general, and closes the dialog.
 */
export function BrowseChannelsDialog({ teamId, onClose }: BrowseChannelsDialogProps) {
  const { crew, snapshot } = useDialogView();
  const open = isOpenWorkspace(snapshot);
  const team = teamId ? (snapshot?.teams.find((item) => item.id === teamId) ?? null) : null;
  const teams = open && !teamId ? (snapshot?.joinable_teams ?? []) : [];
  const channels = open
    ? (snapshot?.joinable_channels ?? []).filter(
        (channel) => teamId === undefined || channel.team_id === teamId
      )
    : [];
  const joining = crew.isPending(TEAM_JOIN_KEY) || crew.isPending(CHANNEL_JOIN_KEY);
  const teamsById = React.useMemo(
    () => new Map((snapshot?.teams ?? []).map((item) => [item.id, item])),
    [snapshot]
  );

  const joinTeam = (joinable: CrewJoinable) =>
    void crew
      .act(SOURCE, TEAM_JOIN_KEY, async () => {
        await crew.mutate('team.join', { team_id: joinable.id });
        return true as const;
      })
      .then((done) => {
        if (done !== true) return;
        crew.selectTeam(joinable.id);
        onClose();
      });
  const joinChannel = (joinable: CrewJoinable) =>
    void crew
      .act(SOURCE, CHANNEL_JOIN_KEY, async () => {
        await crew.mutate('channel.join', { channel_id: joinable.id });
        return true as const;
      })
      .then((done) => {
        if (done !== true) return;
        crew.selectChannel(joinable.id);
        onClose();
      });

  return (
    <ModalShell
      open
      onOpenChange={(next) => !next && onClose()}
      size="md"
      purpose={joining ? 'required' : 'info'}
      scrollBody
      title={team ? copy.titleTeam(teamName(team)) : copy.title}
      footer={
        <Button type="button" onClick={onClose} disabled={joining}>
          {copy.done}
        </Button>
      }
    >
      <div className="flex flex-col gap-4 py-3" data-crew-browse="">
        {teams.length > 0 ? (
          <BrowseSection title={copy.teams}>
            {teams.map((joinable) => (
              <BrowseRow
                key={joinable.id}
                name={teamName(joinable)}
                detail={copy.teamDetail}
                members={joinable.member_count}
                label={copy.joinTeamLabel(teamName(joinable))}
                disabled={joining}
                onJoin={() => joinTeam(joinable)}
              />
            ))}
          </BrowseSection>
        ) : null}
        {channels.length > 0 ? (
          <BrowseSection title={copy.channels}>
            {channels.map((joinable) => {
              const inTeam = joinable.team_id ? teamsById.get(joinable.team_id) : undefined;
              return (
                <BrowseRow
                  key={joinable.id}
                  name={channelName(joinable)}
                  detail={!teamId && inTeam ? copy.channelIn(teamName(inTeam)) : undefined}
                  members={joinable.member_count}
                  restricted={joinable.classification === 'restricted'}
                  label={copy.joinChannelLabel(channelName(joinable))}
                  disabled={joining}
                  onJoin={() => joinChannel(joinable)}
                />
              );
            })}
          </BrowseSection>
        ) : null}
        {teams.length === 0 && channels.length === 0 ? (
          <Note tone="neutral" role="status">
            {copy.nothing}
          </Note>
        ) : null}
        <DialogErrorNote source={SOURCE} />
      </div>
    </ModalShell>
  );
}

/** One list under its caps label, announced as a list. */
function BrowseSection({ title, children }: { title: string; children: React.ReactNode }) {
  const headingId = React.useId();
  return (
    <section className="biorouter-settings-section" aria-labelledby={headingId}>
      <h3 id={headingId} className="text-caps text-text-muted">
        {title}
      </h3>
      <ul role="list" className="biorouter-settings-list">
        {children}
      </ul>
    </section>
  );
}

function BrowseRow({
  name,
  detail,
  members,
  restricted = false,
  label,
  disabled,
  onJoin,
}: {
  name: string;
  detail?: string;
  members?: number;
  restricted?: boolean;
  /** Join's accessible name: what it joins. */
  label: string;
  disabled: boolean;
  onJoin(): void;
}) {
  const line = [detail, members !== undefined ? copy.members(members) : null]
    .filter((part): part is string => Boolean(part))
    .join(' · ');
  return (
    <li className="biorouter-settings-row crew-settings-row flex min-w-0 items-center gap-3 px-3 py-2">
      <div className="min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-2">
          <bdi className="truncate text-label text-text-default">{name}</bdi>
          {restricted ? (
            <Badge tone="neutral" variant="badge">
              {copy.restricted}
            </Badge>
          ) : null}
        </span>
        {line ? <span className="block text-supporting text-text-muted">{line}</span> : null}
      </div>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        aria-label={label}
        disabled={disabled}
        onClick={onJoin}
      >
        {copy.join}
      </Button>
    </li>
  );
}
