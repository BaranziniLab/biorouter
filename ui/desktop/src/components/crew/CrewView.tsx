import { useCallback, useEffect, useRef, useState } from 'react';
import { Users, Plus, Send, RefreshCw } from '../icons/app-icons';
import {
  crewHttp,
  CrewHttpError,
  crewRequest,
  observeCrew,
  type ObservedRun,
  type CrewConnection,
  type Snapshot,
  type CrewMessage,
} from './crewApi';
import CrewAuthentication from './CrewAuthentication';
import CrewCredentials from './CrewCredentials';
import CrewHostTrust from './CrewHostTrust';
import { CrewUpload, CrewAttachment, CrewRemoteReference } from './CrewFiles';
import { clearPublishedTransfers } from './crewTransfers';
import { useNavigate, useSearchParams } from 'react-router-dom';
import './crew.css';
import { useConfig } from '../ConfigContext';
import type { ProviderDetails } from '../../api';

type Panel =
  | 'connection'
  | 'team'
  | 'channel'
  | 'profile'
  | 'invite'
  | 'settings'
  | 'agent'
  | 'reference'
  | 'grant'
  | 'enroll'
  | 'offboard'
  | null;
const emptyConnection = {
  name: '',
  ssh_target: '',
  port: 22,
  identity_file: '',
  proxy_jump: '',
  socket_path: '',
  owner_uid: 1000,
  workspace_id: '',
  workspace_public_key: '',
  remote_root: '',
  remote_execution: false,
  mode: 'private' as 'private' | 'public',
};

interface PendingRunAttempt {
  fingerprint: string;
  key: string;
  unknownDestination?: string;
}
// Inspection can navigate to another route; retain the uncertain attempt in memory, never on disk.
let unfinishedRunAttempt: PendingRunAttempt | null = null;

export default function CrewView() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const grantSessionId = searchParams.get('sessionId');
  const { read, getProviders, getProviderModels } = useConfig();
  const [availableProviders, setAvailableProviders] = useState<ProviderDetails[]>([]);
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [attachments, setAttachments] = useState<{ id: string; name: string }[]>([]);
  const [enrollment, setEnrollment] = useState('');
  const [remoteReferencePath, setRemoteReferencePath] = useState('');
  const [remoteReferenceLabel, setRemoteReferenceLabel] = useState('');
  const [references, setReferences] = useState<{ id: string; label: string }[]>([]);
  const [inviteKind, setInviteKind] = useState<'team' | 'channel'>('team');
  const [inviteUid, setInviteUid] = useState('');
  const [invitePublicKey, setInvitePublicKey] = useState('');
  const [addExistingDevice, setAddExistingDevice] = useState(false);
  const [createdInvitation, setCreatedInvitation] = useState('');
  const [runs, setRuns] = useState<ObservedRun[]>([]);
  const [connections, setConnections] = useState<CrewConnection[]>([]);
  const [connectionId, setConnectionId] = useState('');
  const [observedPrivacy, setObservedPrivacy] = useState<{
    connectionId: string;
    mode: 'private' | 'public';
  } | null>(null);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [teamId, setTeamId] = useState('');
  const [channelId, setChannelId] = useState('');
  const [messages, setMessages] = useState<CrewMessage[]>([]);
  const [historyBefore, setHistoryBefore] = useState<string | null>(null);
  const [panel, setPanel] = useState<Panel>(null);
  const [error, setError] = useState('');
  const [refreshError, setRefreshError] = useState('');
  const visibleError = error || refreshError;
  const [busy, setBusy] = useState(false);
  const [body, setBody] = useState('');
  const [name, setName] = useState('');
  const [avatar, setAvatar] = useState('');
  const [personId, setPersonId] = useState('');
  const [classification, setClassification] = useState('restricted');
  const [connectionForm, setConnectionForm] = useState(emptyConnection);
  const [editingConnection, setEditingConnection] = useState('');
  const [preparedDevice, setPreparedDevice] = useState<{
    preparation_id: string;
    public_key: string;
    device_id: string;
  } | null>(null);
  const [contextChannels, setContextChannels] = useState<string[]>([]);
  const [provider, setProvider] = useState('');
  const [model, setModel] = useState('');
  const [authentication, setAuthentication] = useState(false);
  const generation = useRef(0);
  const observer = useRef<AbortController | null>(null);
  const [observationRevision, setObservationRevision] = useState(0);
  const historyPage = useRef<string | null>(null);
  useEffect(() => {
    historyPage.current = historyBefore;
  }, [historyBefore]);
  const dialogRef = useRef<HTMLElement>(null);
  const pendingMessage = useRef<{ fingerprint: string; key: string } | null>(null);
  const pendingRun = useRef<PendingRunAttempt | null>(unfinishedRunAttempt);
  const [unknownRunDestination, setUnknownRunDestination] = useState(
    unfinishedRunAttempt?.unknownDestination ?? ''
  );
  const [inspectedPriorRun, setInspectedPriorRun] = useState(false);
  const savedConnection = connections.find((item) => item.id === connectionId);
  const connection =
    savedConnection && observedPrivacy?.connectionId === connectionId
      ? { ...savedConnection, mode: observedPrivacy.mode }
      : savedConnection;
  const connectionStatus =
    snapshot && observedPrivacy?.connectionId === connectionId
      ? 'Connected · identity verified'
      : connection?.status === 'connected'
        ? refreshError
          ? 'Updates unavailable'
          : 'Checking connection'
        : connection?.status.replace(/_/g, ' ');
  const channel = snapshot?.channels.find((item) => item.id === channelId);
  const team = snapshot?.teams.find((item) => item.id === teamId);
  const owner = channel?.owner_id === snapshot?.actor.id;
  const channels = snapshot?.channels.filter((item) => item.team_id === teamId) ?? [];
  const existingEnrollee = snapshot?.principals.find((person) => person.uid === Number(inviteUid));
  const offboardPerson = snapshot?.principals.find((person) => person.id === personId);
  const actorName = (id: string) => {
    const actor = snapshot?.principals.find((item) => item.id === id);
    return actor ? `${actor.nickname || actor.username} (@${actor.username})` : id;
  };
  const request = useCallback(
    <T,>(method: string, params: Record<string, unknown> = {}, mutation = false) =>
      crewRequest<T>(connectionId, method, params, mutation),
    [connectionId]
  );

  const verifiedScope = useRef<{ connection: string; epoch: number; mode?: string } | null>(null);
  const selectedSources = useRef(contextChannels);
  useEffect(() => {
    selectedSources.current = contextChannels;
  }, [contextChannels]);
  const clearDraft = useCallback(() => {
    setBody('');
    setAttachments([]);
    setReferences([]);
    setContextChannels([]);
    pendingMessage.current = null;
  }, []);
  const clearProtectedState = useCallback(() => {
    setSnapshot(null);
    setObservedPrivacy(null);
    setRuns([]);
    setMessages([]);
    setHistoryBefore(null);
    historyPage.current = null;
    setPanel(null);
  }, []);
  const observationFailure = useCallback(
    (message: string, code?: string) => {
      clearProtectedState();
      if (
        code &&
        [
          'channel_access_changed',
          'policy_changed',
          'scope_changed',
          'access_denied',
          'principal_revoked',
          'forbidden',
          'privacy_denied',
          'human_authority_required',
        ].includes(code)
      ) {
        clearDraft();
        setRefreshError(
          `${message} Access or privacy changed, so the unsent draft and attachments were cleared. Retry to verify access.`
        );
      } else {
        setRefreshError(
          `${message} Your unsent draft is retained for this channel. Retry to verify access before sending.`
        );
      }
    },
    [clearProtectedState, clearDraft]
  );
  const loadConnections = useCallback(async (signal?: AbortSignal, current?: number) => {
    const result = await crewHttp<{ connections: CrewConnection[] }>(
      '/connections',
      'GET',
      undefined,
      signal
    );
    if (signal?.aborted || (current !== undefined && generation.current !== current)) return;
    setConnections(result.connections);
    setConnectionId((old) =>
      result.connections.some((item) => item.id === old) ? old : (result.connections[0]?.id ?? '')
    );
  }, []);
  const refresh = useCallback(async () => {
    observer.current?.abort();
    const controller = new AbortController();
    observer.current = controller;
    const current = ++generation.current;
    setSnapshot(null);
    setObservedPrivacy(null);
    setRuns([]);
    setMessages([]);
    setPanel(null);
    setRefreshError('');
    try {
      await loadConnections(controller.signal, current);
      if (!controller.signal.aborted && generation.current === current)
        setObservationRevision((revision) => revision + 1);
    } catch (failure) {
      if (!controller.signal.aborted && generation.current === current)
        observationFailure(
          failure instanceof Error
            ? failure.message
            : 'Saved Crew connections could not be refreshed.',
          failure instanceof CrewHttpError ? failure.code : undefined
        );
    }
  }, [loadConnections, observationFailure]);
  useEffect(
    () => () => {
      observer.current?.abort();
      generation.current += 1;
    },
    []
  );

  const act = async (operation: () => Promise<unknown>) => {
    setBusy(true);
    setError('');
    try {
      await operation();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Crew could not complete that action.');
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    void loadConnections().catch((err: Error) => setError(err.message));
  }, [loadConnections]);
  useEffect(() => {
    generation.current += 1;
    setSnapshot(null);
    setMessages([]);
    setChannelId('');
    setTeamId('');
    setBody('');
    setAttachments([]);
    setReferences([]);
    setRuns([]);
    setContextChannels([]);
    setAuthentication(false);
    setError('');
    setRefreshError('');
  }, [connectionId]);
  useEffect(() => {
    historyPage.current = null;
    setHistoryBefore(null);
    setBody('');
    setAttachments([]);
    setReferences([]);
    setContextChannels([]);
    setPanel(null);
  }, [channelId]);
  useEffect(() => {
    if (panel === 'settings' && !owner) setPanel(null);
  }, [panel, owner]);
  useEffect(() => {
    if (!connectionId) return;
    const controller = new AbortController();
    observer.current = controller;
    const current = ++generation.current;
    const active = () => !controller.signal.aborted && current === generation.current;
    let cursor: string | null = null;
    let immediateReconnects = 0;
    void (async () => {
      while (active()) {
        const started = Date.now();
        const outcome = await observeCrew(
          connectionId,
          channelId || undefined,
          cursor,
          controller.signal,
          (frame) => {
            if (!active()) return;
            if (frame.type === 'state') {
              if (frame.connection_id !== connectionId)
                throw new Error(
                  'The daemon returned a different Crew connection. Retry to verify the workspace.'
                );
              const previousScope = verifiedScope.current;
              if (
                previousScope?.connection === connectionId &&
                (previousScope.epoch !== frame.snapshot.workspace.policy_epoch ||
                  previousScope.mode !== frame.connection_mode ||
                  selectedSources.current.some(
                    (id) => !frame.snapshot.channels.some((item) => item.id === id)
                  ))
              ) {
                clearDraft();
                setError(
                  'Workspace privacy or selected channel access changed while reconnecting. The unsent draft and attachments were cleared; review the current policy before composing again.'
                );
              }
              verifiedScope.current = {
                connection: connectionId,
                epoch: frame.snapshot.workspace.policy_epoch,
                mode: frame.connection_mode,
              };
              setObservedPrivacy({ connectionId, mode: frame.connection_mode });
              setConnections((items) =>
                items.map((item) =>
                  item.id === connectionId ? { ...item, mode: frame.connection_mode } : item
                )
              );
              setSnapshot(frame.snapshot);
              setRuns(frame.runs);
              setRefreshError('');
              setTeamId((old) =>
                frame.snapshot.teams.some((item) => item.id === old)
                  ? old
                  : (frame.snapshot.teams[0]?.id ?? '')
              );
              if (channelId && !frame.snapshot.channels.some((item) => item.id === channelId)) {
                controller.abort();
                generation.current += 1;
                setMessages([]);
                setBody('');
                setAttachments([]);
                setReferences([]);
                setContextChannels([]);
                setHistoryBefore(null);
                historyPage.current = null;
                pendingMessage.current = null;
                setPanel(null);
                setChannelId('');
                setError(
                  'Access to the selected channel changed. Its messages and unsent composer content have been cleared; choose an authorized channel to continue.'
                );
              }
            } else if (frame.type === 'messages' && frame.channel_id === channelId) {
              cursor = frame.cursor ?? null;
              if (historyPage.current !== null) return;
              setMessages((previous) => {
                const next = frame.reset ? [] : [...previous];
                for (const message of frame.messages) {
                  const index = next.findIndex((old) => old.id === message.id);
                  if (index < 0) next.push(message);
                  else next[index] = message;
                }
                return next.slice(-200);
              });
            } else if (frame.type === 'reconnect') {
              cursor = frame.cursor ?? null;
            } else if (frame.type === 'error') {
              observationFailure(frame.error, frame.code);
              generation.current += 1;
            }
          }
        );
        if (outcome !== 'reconnect' || !active()) return;
        immediateReconnects = Date.now() - started < 1000 ? immediateReconnects + 1 : 0;
        if (immediateReconnects >= 3)
          throw new Error(
            'The daemon repeatedly ended Crew observation. Retry after checking the daemon.'
          );
      }
    })().catch((failure: unknown) => {
      if (!active()) return;
      observationFailure(
        failure instanceof Error ? failure.message : 'Crew observation failed.',
        failure instanceof CrewHttpError ? failure.code : undefined
      );
      generation.current += 1;
    });
    return () => {
      controller.abort();
      observer.current?.abort();
      observer.current = null;
      generation.current += 1;
    };
  }, [connectionId, channelId, observationRevision, observationFailure, clearDraft]);

  useEffect(() => {
    if (historyBefore === null || !connectionId || !channelId) return;
    const controller = new AbortController();
    const current = generation.current;
    setMessages([]);
    void crewRequest<{ messages: CrewMessage[]; cursor: string | null }>(
      connectionId,
      'messages.history',
      {
        channel_id: channelId,
        limit: 200,
        latest: true,
        before: historyBefore,
      },
      false,
      controller.signal
    )
      .then((page) => {
        if (!controller.signal.aborted && current === generation.current)
          setMessages(page.messages);
      })
      .catch((failure: unknown) => {
        if (controller.signal.aborted || current !== generation.current) return;
        observer.current?.abort();
        generation.current += 1;
        observationFailure(
          failure instanceof Error ? failure.message : 'Earlier messages could not be loaded.',
          failure instanceof CrewHttpError ? failure.code : undefined
        );
      });
    return () => controller.abort();
  }, [historyBefore, connectionId, channelId, observationRevision, observationFailure]);
  useEffect(() => {
    if (!snapshot) return;
    setChannelId((old) =>
      channels.some((item) => item.id === old)
        ? old
        : (channels.find((item) => !item.archived)?.id ?? '')
    );
  }, [teamId, snapshot]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (panel !== 'agent') return;
    let active = true;
    void Promise.all([
      getProviders(false),
      read('BIOROUTER_PROVIDER', false),
      read('BIOROUTER_MODEL', false),
    ])
      .then(([items, selectedProvider, selectedModel]) => {
        if (!active) return;
        setAvailableProviders(items.filter((item) => item.is_configured));
        if (typeof selectedProvider === 'string') setProvider(selectedProvider);
        if (typeof selectedModel === 'string') setModel(selectedModel);
      })
      .catch((err: Error) => {
        if (active) setError(err.message);
      });
    return () => {
      active = false;
    };
  }, [panel, getProviders, read]);
  useEffect(() => {
    if (panel !== 'agent' || !provider) return;
    let active = true;
    void getProviderModels(provider)
      .then((items) => {
        if (active) setAvailableModels(items);
      })
      .catch(() => {
        if (active) setAvailableModels([]);
      });
    return () => {
      active = false;
    };
  }, [panel, provider, getProviderModels]);

  useEffect(() => {
    if (!panel) return;
    const previous = document.activeElement as HTMLElement | null;
    dialogRef.current?.querySelector<HTMLElement>('input, select, textarea, button')?.focus();
    return () => previous?.focus();
  }, [panel]);

  const openPanel = (next: Panel) => {
    if (next === 'connection') {
      setEditingConnection('');
      setConnectionForm(emptyConnection);
    }
    setPanel(next);
    setName(next === 'profile' ? (snapshot?.actor.nickname ?? '') : '');
    setAvatar(snapshot?.actor.avatar ?? '');
    setPersonId('');
    setAddExistingDevice(false);
  };
  const mutate = async (method: string, params: Record<string, unknown>) => {
    await request(method, params, true);
    await refresh();
    setPanel(null);
  };
  const connect = () =>
    act(async () => {
      await crewHttp(`/connections/${connectionId}/connect`, 'POST', {});
      await loadConnections();
      await refresh();
    });
  const submitOwnedRun = async (deliberateRestart = false) => {
    if (!snapshot || observedPrivacy?.connectionId !== connectionId)
      throw new Error(
        'Refresh the workspace to verify connection privacy before granting agent access.'
      );
    if (pendingRun.current?.unknownDestination && !deliberateRestart) {
      throw new Error(
        'Inspect the previous task conversations and remote effects, then acknowledge the inspection before starting another task.'
      );
    }
    if (deliberateRestart && (!pendingRun.current?.unknownDestination || !inspectedPriorRun)) {
      throw new Error('Confirm that you inspected the previous task before starting a new one.');
    }
    const payload = {
      expected_mode: observedPrivacy.mode,
      channel_id: channelId,
      prompt: body,
      provider,
      model,
      context_channels: [channelId, ...contextChannels],
      posting_grant: true,
    };
    const fingerprint = JSON.stringify({ connectionId, ...payload });
    if (deliberateRestart || pendingRun.current?.fingerprint !== fingerprint) {
      pendingRun.current = { fingerprint, key: crypto.randomUUID() };
    }
    unfinishedRunAttempt = pendingRun.current;
    if (deliberateRestart) {
      setUnknownRunDestination('');
      setInspectedPriorRun(false);
    }
    try {
      await crewHttp(`/connections/${connectionId}/runs`, 'POST', {
        ...payload,
        request_id: pendingRun.current.key,
      });
    } catch (failure) {
      if (failure instanceof CrewHttpError && failure.code === 'crew_start_outcome_unknown') {
        const destination = `${connection?.name ?? connectionId} / ${team?.name ?? teamId} / #${channel?.name ?? channelId}`;
        pendingRun.current.unknownDestination = destination;
        unfinishedRunAttempt = pendingRun.current;
        setUnknownRunDestination(destination);
        setInspectedPriorRun(false);
      }
      throw failure;
    }
    pendingRun.current = null;
    unfinishedRunAttempt = null;
    setUnknownRunDestination('');
    setInspectedPriorRun(false);
    setPanel(null);
    setBody('');
    await refresh();
  };

  const refreshChannel = async () => {
    setBusy(true);
    try {
      await refresh();
    } catch (failure) {
      setRefreshError(failure instanceof Error ? failure.message : String(failure));
      setSnapshot(null);
      setMessages([]);
    } finally {
      setBusy(false);
    }
  };

  const send = () =>
    act(async () => {
      if (!snapshot || observedPrivacy?.connectionId !== connectionId)
        throw new Error('Refresh the workspace to verify connection privacy before sending.');
      const payload = {
        personal_mode: observedPrivacy.mode,
        channel_id: channelId,
        body,
        attachments: attachments.map((item) => item.id),
        references: references.map((item) => item.id),
      };
      const fingerprint = JSON.stringify({ connectionId, ...payload });
      if (pendingMessage.current?.fingerprint !== fingerprint)
        pendingMessage.current = { fingerprint, key: crypto.randomUUID() };
      await request(
        'message.post',
        { ...payload, idempotency_key: pendingMessage.current.key },
        true
      );
      try {
        await clearPublishedTransfers(
          connectionId,
          attachments.map((item) => item.id)
        );
      } catch {
        setError(
          'Message posted. Local transfer metadata could not be cleared; forget the completed record in saved transfers.'
        );
      }
      pendingMessage.current = null;
      setHistoryBefore(null);
      setReferences([]);
      setBody('');
      setAttachments([]);
      await refresh();
    });

  return (
    <div className="crew-view" data-testid="crew-view">
      <header className="crew-topbar">
        <div className="crew-brand">
          <Users size={22} />
          <div>
            <h1>Crew</h1>
            <p>
              Your people, projects, and agents
              {window.appConfig?.get('BIOROUTER_DEV_PROFILE_NAME')
                ? ` · Profile: ${window.appConfig.get('BIOROUTER_DEV_PROFILE_NAME')}`
                : ''}
            </p>
          </div>
        </div>
        <button className="crew-button" onClick={() => openPanel('connection')}>
          <Plus size={15} /> Add workspace
        </button>
      </header>
      <div className="crew-layout">
        <aside className="crew-rail" aria-label="Crew workspaces and channels">
          <label className="crew-label">
            SSH workspace
            <select
              aria-label="SSH workspace"
              disabled={busy}
              value={connectionId}
              onChange={(e) => {
                generation.current += 1;
                setSnapshot(null);
                setMessages([]);
                setConnectionId(e.target.value);
              }}
            >
              <option value="">Choose a connection</option>
              {connections.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name}
                </option>
              ))}
            </select>
          </label>
          {connection && (
            <div className="crew-connection">
              <strong>{connection.ssh_target}</strong>
              <span className="crew-status">{connectionStatus}</span>
              <div className="crew-inline">
                <button onClick={connect} disabled={busy || authentication}>
                  Reconnect
                </button>
                <button onClick={() => setAuthentication(true)}>Authenticate</button>
                <button
                  onClick={() => {
                    setEditingConnection(connection.id);
                    setConnectionForm({
                      name: connection.name,
                      ssh_target: connection.ssh_target,
                      port: connection.port || 22,
                      identity_file: connection.identity_file || '',
                      proxy_jump: connection.proxy_jump || '',
                      socket_path: connection.socket_path,
                      owner_uid: connection.owner_uid,
                      workspace_id: connection.workspace_id,
                      workspace_public_key: connection.workspace_public_key,
                      remote_root: connection.remote_root || '',
                      remote_execution: connection.remote_execution,
                      mode: connection.mode,
                    });
                    setPanel('connection');
                  }}
                >
                  Edit
                </button>
              </div>
              <label className="crew-label">
                Connection privacy
                <select
                  aria-label="Connection privacy"
                  value={connection.mode}
                  disabled={busy}
                  onChange={(e) =>
                    void act(async () => {
                      await crewHttp(`/connections/${connectionId}`, 'PATCH', {
                        name: connection.name,
                        ssh_target: connection.ssh_target,
                        port: connection.port,
                        identity_file: connection.identity_file,
                        proxy_jump: connection.proxy_jump,
                        socket_path: connection.socket_path,
                        owner_uid: connection.owner_uid,
                        workspace_id: connection.workspace_id,
                        workspace_public_key: connection.workspace_public_key,
                        cluster_connection_id: connection.cluster_connection_id,
                        remote_root: connection.remote_root,
                        remote_execution: connection.remote_execution,
                        mode: e.target.value,
                      });
                      await loadConnections();
                      await refresh();
                    })
                  }
                >
                  <option value="private">Private · block public models</option>
                  <option value="public">Public · source restrictions apply</option>
                </select>
              </label>
              {snapshot && snapshot.actor.uid === snapshot.workspace.host_uid && (
                <label className="crew-label">
                  Shared workspace policy
                  <select
                    aria-label="Shared workspace policy"
                    value={snapshot.workspace.mode}
                    disabled={busy}
                    onChange={(event) =>
                      void act(async () => {
                        await request('policy.set', { mode: event.target.value }, true);
                        await refresh();
                      })
                    }
                  >
                    <option value="private">Private for everyone</option>
                    <option value="public">Allow Public preferences and public-safe work</option>
                  </select>
                  <span className="crew-small">
                    Only the workspace host changes this baseline. Existing content keeps its
                    restrictions; active agents need fresh grants after a change.
                  </span>
                </label>
              )}
              <p>
                Effective:{' '}
                {connection.mode === 'private' || snapshot?.workspace.mode === 'private'
                  ? 'private'
                  : snapshot
                    ? 'public'
                    : 'unknown until the workspace policy is verified'}
                . Existing content keeps its restrictions.
              </p>
            </div>
          )}
          {snapshot && (
            <>
              <div className="crew-section-title">
                <span>Teams</span>
                <button aria-label="Create team" onClick={() => openPanel('team')}>
                  <Plus size={15} />
                </button>
              </div>
              <select
                aria-label="Team"
                disabled={busy}
                value={teamId}
                onChange={(e) => {
                  generation.current += 1;
                  setReferences([]);
                  setMessages([]);
                  setBody('');
                  setAttachments([]);
                  setContextChannels([]);
                  setTeamId(e.target.value);
                }}
              >
                {snapshot.teams.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                  </option>
                ))}
              </select>
              <div className="crew-section-title">
                <span>Channels</span>
                <button
                  aria-label="Create channel"
                  disabled={!teamId}
                  onClick={() => openPanel('channel')}
                >
                  <Plus size={15} />
                </button>
              </div>
              <nav>
                {channels.map((item) => (
                  <button
                    key={item.id}
                    disabled={busy}
                    className={`crew-channel ${item.id === channelId ? 'selected' : ''}`}
                    onClick={() => {
                      if (item.id === channelId) return;
                      generation.current += 1;
                      setReferences([]);
                      setMessages([]);
                      setAttachments([]);
                      setBody('');
                      setChannelId(item.id);
                      setContextChannels([]);
                    }}
                  >
                    <span># {item.name}</span>
                    {(snapshot.unread?.[item.id] ?? 0) > 0 && (
                      <small aria-label={`${snapshot.unread?.[item.id]} unread messages`}>
                        {snapshot.unread?.[item.id]}
                      </small>
                    )}
                    {item.archived && <small>Archived</small>}
                  </button>
                ))}
              </nav>
              <div className="crew-section-title">
                <span>People · {snapshot.principals.length}</span>
              </div>
              <div className="crew-people">
                {snapshot.principals.map((person) => (
                  <div key={person.id}>
                    <span className="crew-avatar">
                      {person.avatar ||
                        (person.nickname || person.username).slice(0, 2).toUpperCase()}
                    </span>
                    <span>
                      {person.nickname || person.username}
                      <small>
                        @{person.username}
                        {person.id === snapshot.actor.id ? ' · you' : ''}
                      </small>
                    </span>
                    {snapshot.actor.uid === snapshot.workspace.host_uid &&
                      person.id !== snapshot.actor.id && (
                        <button
                          className="crew-button"
                          aria-label={`Remove ${person.username} from workspace`}
                          onClick={() => {
                            openPanel('offboard');
                            setPersonId(person.id);
                          }}
                        >
                          Remove access
                        </button>
                      )}
                  </div>
                ))}
              </div>
              <button className="crew-button" onClick={() => openPanel('profile')}>
                Edit my profile
              </button>
              {snapshot.actor.uid === snapshot.workspace.host_uid && (
                <button
                  className="crew-button"
                  onClick={() => {
                    setCreatedInvitation('');
                    openPanel('enroll');
                  }}
                >
                  Enroll a colleague
                </button>
              )}
              {team?.created_by === snapshot.actor.id && (
                <button
                  className="crew-button"
                  onClick={() => {
                    setInviteKind('team');
                    openPanel('invite');
                  }}
                >
                  Invite to team
                </button>
              )}
            </>
          )}
          <CrewCredentials />
        </aside>
        <main className="crew-main">
          {visibleError && (
            <div className="crew-error" role="alert">
              <strong>Action needs attention</strong>
              <p>{visibleError}</p>
              {refreshError && connectionId && (
                <button onClick={() => void refresh()}>Retry Crew updates</button>
              )}
              {!refreshError && <button onClick={() => setError('')}>Dismiss</button>}
              {/host|SSH|key|authentication/i.test(visibleError) && <CrewHostTrust />}
            </div>
          )}
          {authentication && connection && (
            <CrewAuthentication
              connectionId={connectionId}
              onConnected={() => {
                setAuthentication(false);
                void act(async () => {
                  await loadConnections();
                  await refresh();
                });
              }}
              onClose={() => setAuthentication(false)}
            />
          )}
          {!snapshot ? (
            <div className="crew-empty">
              <div className="crew-empty-mark">
                <Users size={38} />
              </div>
              <h2>A shared place for your lab</h2>
              <p>
                Connect with your own SSH account to talk, share files, and work with your team.
                Human conversations do not need a model.
              </p>
              {connection ? (
                <button
                  className="crew-button primary"
                  disabled={busy || authentication}
                  onClick={connect}
                >
                  Connect to {connection.name}
                </button>
              ) : (
                <button className="crew-button primary" onClick={() => openPanel('connection')}>
                  Add your first workspace
                </button>
              )}
              <p className="crew-small">
                Private by default. Your SSH identity determines your permissions.
              </p>
              {connection && (
                <div>
                  <p>
                    First connection? Enroll with the invitation token from your workspace host.
                  </p>
                  <input
                    aria-label="Enrollment invitation"
                    placeholder="Invitation token"
                    value={enrollment}
                    onChange={(e) => setEnrollment(e.target.value)}
                  />
                  <div className="crew-inline">
                    <button
                      className="crew-button"
                      disabled={busy || !enrollment.trim()}
                      onClick={() =>
                        void act(async () => {
                          await request(
                            'auth.enroll',
                            { invitation: enrollment, public_key: connection.public_key },
                            true
                          );
                          setEnrollment('');
                          await refresh();
                        })
                      }
                    >
                      Join workspace
                    </button>
                    <button
                      className="crew-button"
                      disabled={busy}
                      onClick={() =>
                        void act(async () => {
                          await request(
                            'auth.bootstrap',
                            { public_key: connection.public_key },
                            true
                          );
                          await refresh();
                        })
                      }
                    >
                      Initialize as workspace host
                    </button>
                  </div>
                  <p className="crew-small">Enrollment public key: {connection.public_key}</p>
                </div>
              )}
            </div>
          ) : !channel ? (
            <div className="crew-empty">
              <h2>{snapshot.teams.length ? 'Choose a channel' : 'Start your first team'}</h2>
              <p>Create a team or accept an invitation to begin collaborating.</p>
              <button className="crew-button primary" onClick={() => openPanel('team')}>
                Create team
              </button>
            </div>
          ) : (
            <>
              {channel.pending_owner === snapshot.actor.id && (
                <div className="crew-invitation">
                  <span>You have been offered ownership of #{channel.name}.</span>
                  <button
                    className="crew-button"
                    disabled={busy}
                    onClick={() =>
                      void act(() => mutate('transfer.accept', { channel_id: channelId }))
                    }
                  >
                    Accept ownership
                  </button>
                </div>
              )}
              {!grantSessionId && (
                <div className="crew-invitation">
                  <span>
                    To give a personal conversation access to this channel, open it from Chat
                    history and use /crew to review permissions. Start a new conversation with a
                    non-sensitive first message before connecting it.
                  </span>
                  <button className="crew-button" onClick={() => navigate('/sessions')}>
                    Open Chat history
                  </button>
                </div>
              )}
              {grantSessionId && (
                <div className="crew-invitation">
                  <span>Connect your current agent conversation to #{channel.name}.</span>
                  <button className="crew-button" onClick={() => openPanel('grant')}>
                    Review access and posting permission
                  </button>
                </div>
              )}
              <header className="crew-channel-header">
                <div>
                  <p>
                    {connection?.name} / {team?.name}
                  </p>
                  <h2>
                    # {channel.name}
                    {channel.archived && <span className="crew-pill">Archived</span>}
                  </h2>
                  <p>
                    {channel.members.length} members ·{' '}
                    {channel.classification === 'restricted'
                      ? 'Restricted content'
                      : 'Public-safe content'}{' '}
                    · SSH @{snapshot.actor.username}
                  </p>
                </div>
                <div className="crew-inline">
                  <button
                    className="crew-button"
                    disabled={busy || !messages.length}
                    onClick={() =>
                      void act(() =>
                        mutate('channel.read', {
                          channel_id: channelId,
                          sequence: messages[messages.length - 1]?.sequence,
                        })
                      )
                    }
                  >
                    Mark read
                  </button>
                  <button
                    aria-label="Refresh channel"
                    disabled={busy}
                    onClick={() => void refreshChannel()}
                  >
                    <RefreshCw size={16} />
                  </button>
                  {owner && (
                    <button
                      className="crew-button"
                      onClick={() => {
                        setInviteKind('channel');
                        openPanel('invite');
                      }}
                    >
                      Invite
                    </button>
                  )}
                  {owner && (
                    <button className="crew-button" onClick={() => openPanel('settings')}>
                      Channel settings
                    </button>
                  )}
                </div>
              </header>
              <div className="crew-inline" aria-label="Message history navigation">
                <button
                  className="crew-button"
                  disabled={busy || messages.length < 200}
                  onClick={() => {
                    historyPage.current = messages[0]?.sequence ?? null;
                    setHistoryBefore(historyPage.current);
                  }}
                >
                  Older messages
                </button>
                {historyBefore !== null && (
                  <>
                    <span className="crew-small">Viewing earlier messages</span>
                    <button
                      className="crew-button"
                      disabled={busy}
                      onClick={() => {
                        historyPage.current = null;
                        setHistoryBefore(null);
                        void refresh();
                      }}
                    >
                      Latest messages
                    </button>
                  </>
                )}
              </div>
              <div
                className="crew-timeline"
                role="log"
                aria-label={`${channel.name} messages`}
                aria-live="polite"
              >
                {messages.length === 0 && (
                  <div className="crew-welcome">
                    <h3>Welcome to #{channel.name}</h3>
                    <p>
                      Start a conversation with your team. Only members of this channel can read it.
                    </p>
                  </div>
                )}
                {messages.map((message) => (
                  <article key={message.id} className="crew-message">
                    <div className="crew-avatar">
                      {snapshot.principals.find((p) => p.id === message.actor_id)?.avatar ||
                        (
                          snapshot.principals.find((p) => p.id === message.actor_id)?.nickname ||
                          '?'
                        )
                          .slice(0, 2)
                          .toUpperCase()}
                    </div>
                    <div>
                      <div className="crew-message-meta">
                        <strong>{actorName(message.actor_id)}</strong>
                        {message.run_id && <span className="crew-pill">Agent</span>}
                        <time>
                          {new Date(
                            message.created_at < 1e12
                              ? message.created_at * 1000
                              : message.created_at
                          ).toLocaleString()}
                        </time>
                        {message.restricted && <span className="crew-pill">Restricted</span>}
                      </div>
                      <p className="crew-message-body">{message.body}</p>
                      {message.references?.map((id) => (
                        <CrewRemoteReference
                          connectionId={connectionId}
                          referenceId={id}
                          key={id}
                        />
                      ))}
                      {message.attachments.map((id) => (
                        <CrewAttachment connectionId={connectionId} blobId={id} key={id} />
                      ))}
                    </div>
                  </article>
                ))}
                {runs
                  .filter((run) => run.channel_id === channelId)
                  .map((run) => (
                    <div className="crew-run" key={run.run_id}>
                      <strong>My agent</strong>
                      <span>
                        {run.status}
                        {run.error && ` · ${run.error}`}
                      </span>
                      <button
                        className="crew-button"
                        onClick={() =>
                          navigate(`/pair?resumeSessionId=${encodeURIComponent(run.session_id)}`)
                        }
                      >
                        Open agent session
                      </button>
                      <button
                        className="crew-button"
                        disabled={
                          busy ||
                          ![
                            'starting',
                            'running',
                            'waiting_for_approval',
                            'cancellation_pending',
                            'cancellation_unconfirmed',
                            'interrupted',
                            'outcome_not_durable',
                          ].includes(run.status)
                        }
                        onClick={() =>
                          void act(async () => {
                            await crewHttp(
                              `/connections/${connectionId}/runs/${run.run_id}/cancel`,
                              'POST',
                              {}
                            );
                            await refresh();
                          })
                        }
                      >
                        Cancel
                      </button>
                    </div>
                  ))}
              </div>
              <div className="crew-composer">
                <label className="crew-label" htmlFor="crew-message">
                  Message #{channel.name}
                </label>
                <textarea
                  id="crew-message"
                  value={body}
                  onChange={(e) => setBody(e.target.value)}
                  placeholder={`Message your teammates in #${channel.name}…`}
                  disabled={busy || channel.archived}
                  onKeyDown={(e) => {
                    if (!busy && e.key === 'Enter' && (e.metaKey || e.ctrlKey) && body.trim()) {
                      e.preventDefault();
                      void send();
                    }
                  }}
                />
                <CrewUpload
                  expectedMode={
                    observedPrivacy?.connectionId === connectionId
                      ? observedPrivacy.mode
                      : undefined
                  }
                  key={`${connectionId}:${channelId}`}
                  connectionId={connectionId}
                  channelId={channelId}
                  disabled={busy || channel.archived}
                  onReady={(blob) =>
                    setAttachments((items) =>
                      items.some((item) => item.id === blob.id) ? items : [...items, blob]
                    )
                  }
                  onRemoteReference={() => openPanel('reference')}
                />
                {attachments.map((file) => (
                  <span className="crew-pill" key={file.id}>
                    {file.name}
                    <button
                      aria-label={`Remove ${file.name}`}
                      onClick={() =>
                        setAttachments((items) => items.filter((item) => item.id !== file.id))
                      }
                    >
                      {' '}
                      ×
                    </button>
                  </span>
                ))}
                {references.map((item) => (
                  <span className="crew-pill" key={item.id}>
                    Remote reference: {item.label}
                    <button
                      aria-label={`Remove remote reference ${item.label}`}
                      onClick={() =>
                        setReferences((items) =>
                          items.filter((reference) => reference.id !== item.id)
                        )
                      }
                    >
                      {' '}
                      ×
                    </button>
                  </span>
                ))}
                <div className="crew-composer-footer">
                  <span>
                    {channel.archived
                      ? 'This channel is archived.'
                      : `Posting as @${snapshot.actor.username} · ${connection?.name} / ${team?.name} / ${channel.name}`}
                  </span>
                  <div className="crew-inline">
                    <button
                      className="crew-button"
                      disabled={busy || channel.archived}
                      onClick={() => openPanel('agent')}
                    >
                      Ask my agent
                    </button>
                    <button
                      className="crew-button primary"
                      disabled={
                        busy ||
                        (!body.trim() && attachments.length === 0 && references.length === 0) ||
                        channel.archived
                      }
                      onClick={send}
                    >
                      <Send size={15} /> Send message
                    </button>
                  </div>
                </div>
              </div>
            </>
          )}
          {snapshot?.invitations
            .filter(
              (invite) =>
                invite.principal_id === snapshot.actor.id &&
                (!invite.status || invite.status === 'pending')
            )
            .map((invite) => (
              <div className="crew-invitation" key={invite.id}>
                <span>
                  Invitation to {invite.kind} {invite.target_id}
                </span>
                <button
                  className="crew-button"
                  disabled={busy}
                  onClick={() =>
                    void act(() => mutate('invitation.accept', { invitation_id: invite.id }))
                  }
                >
                  Accept invitation
                </button>
              </div>
            ))}
        </main>
      </div>
      {panel && (
        <div className="crew-modal-backdrop">
          <section
            className="crew-modal"
            ref={dialogRef}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault();
                setPanel(null);
              }
              if (event.key !== 'Tab') return;
              const items = [
                ...(dialogRef.current?.querySelectorAll<HTMLElement>(
                  'input:not([disabled]),select:not([disabled]),textarea:not([disabled]),button:not([disabled])'
                ) ?? []),
              ];
              const first = items[0];
              const last = items[items.length - 1];
              if (event.shiftKey && document.activeElement === first) {
                event.preventDefault();
                last?.focus();
              }
              if (!event.shiftKey && document.activeElement === last) {
                event.preventDefault();
                first?.focus();
              }
            }}
            role="dialog"
            aria-modal="true"
            aria-labelledby="crew-panel-title"
          >
            <div className="crew-section-title">
              <h2 id="crew-panel-title">
                {
                  {
                    connection: 'Add SSH workspace',
                    team: 'Create team',
                    channel: 'Create channel',
                    profile: 'Your profile',
                    invite: 'Invite a teammate',
                    settings: 'Channel ownership',
                    agent: 'Ask my agent',
                    enroll: 'Enroll a colleague',
                    offboard: 'Remove workspace access',
                    reference: 'Share a remote file reference',
                    grant: 'Connect this agent conversation',
                  }[panel]
                }
              </h2>
              <button onClick={() => setPanel(null)} aria-label="Close dialog">
                ✕
              </button>
            </div>
            {panel === 'connection' ? (
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void act(async () => {
                    const saved = await crewHttp<CrewConnection>(
                      editingConnection ? `/connections/${editingConnection}` : '/connections',
                      editingConnection ? 'PATCH' : 'POST',
                      {
                        ...connectionForm,
                        preparation_id: !editingConnection
                          ? preparedDevice?.preparation_id
                          : undefined,
                        cluster_connection_id: editingConnection
                          ? connection?.cluster_connection_id
                          : undefined,
                        identity_file: connectionForm.identity_file || undefined,
                        proxy_jump: connectionForm.proxy_jump || undefined,
                        remote_root: connectionForm.remote_root || undefined,
                      }
                    );
                    await loadConnections();
                    setConnectionId(saved.id);
                    setPreparedDevice(null);
                    setPanel(null);
                  });
                }}
              >
                <p>
                  Use the workspace invitation’s verified endpoint and your own SSH account. SSH
                  configuration and jump hosts are supported.
                </p>
                {!editingConnection && (
                  <details className="crew-trust">
                    <summary>Hosting a new workspace?</summary>
                    <p>
                      Prepare this device first, then initialize the broker using your own ordinary
                      SSH account. Keep the private key on this computer.
                    </p>
                    <button
                      type="button"
                      className="crew-button"
                      disabled={busy}
                      onClick={() =>
                        void act(async () => {
                          const prepared = await crewHttp<{
                            preparation_id: string;
                            public_key: string;
                            device_id: string;
                          }>('/devices/prepare', 'POST', {});
                          setPreparedDevice(prepared);
                        })
                      }
                    >
                      {preparedDevice
                        ? 'Recover prepared device key'
                        : 'Prepare my hosting identity'}
                    </button>
                    {preparedDevice && (
                      <>
                        <label className="crew-label">
                          Public bootstrap key
                          <input
                            readOnly
                            value={preparedDevice.public_key}
                            onFocus={(event) => event.currentTarget.select()}
                          />
                        </label>
                        <p>
                          Install the reviewed Linux broker at{' '}
                          <code>~/.local/bin/biorouter-crew</code>, then run these commands in your
                          SSH terminal:
                        </p>
                        <pre className="crew-setup-command">{`mkdir -p "$HOME/.local/share/biorouter-crew/workspace"
chmod 700 "$HOME/.local/share/biorouter-crew/workspace"
~/.local/bin/biorouter-crew start --state-dir "$HOME/.local/share/biorouter-crew/workspace" --bootstrap-key ${preparedDevice.public_key}
~/.local/bin/biorouter-crew status --state-dir "$HOME/.local/share/biorouter-crew/workspace"`}</pre>
                        <p>
                          Copy the socket, workspace ID, owner UID and workspace public key from the
                          status output into this form. After saving, connect and select Initialize
                          as workspace host. Preparing again after an app restart recovers the same
                          unused identity.
                        </p>
                      </>
                    )}
                  </details>
                )}
                {(
                  [
                    'name',
                    'ssh_target',
                    'identity_file',
                    'proxy_jump',
                    'socket_path',
                    'workspace_id',
                    'workspace_public_key',
                  ] as const
                ).map((field) => (
                  <label className="crew-label" key={field}>
                    {
                      {
                        name: 'Connection name',
                        ssh_target: 'SSH target (username@host or trusted alias)',
                        identity_file: 'Identity file (optional)',
                        proxy_jump: 'Jump hosts (optional)',
                        socket_path: 'Broker socket path from invitation',
                        workspace_id: 'Workspace ID from invitation',
                        workspace_public_key: 'Pinned workspace public key (64 hex characters)',
                      }[field]
                    }
                    <input
                      required={!['identity_file', 'proxy_jump'].includes(field)}
                      value={connectionForm[field]}
                      onChange={(e) =>
                        setConnectionForm({ ...connectionForm, [field]: e.target.value })
                      }
                    />
                  </label>
                ))}
                <div className="crew-inline">
                  <label className="crew-label">
                    SSH port
                    <input
                      type="number"
                      min="1"
                      max="65535"
                      value={connectionForm.port}
                      onChange={(e) =>
                        setConnectionForm({ ...connectionForm, port: Number(e.target.value) })
                      }
                    />
                  </label>
                  <label className="crew-label">
                    Broker owner UID
                    <input
                      type="number"
                      min="0"
                      required
                      value={connectionForm.owner_uid}
                      onChange={(e) =>
                        setConnectionForm({ ...connectionForm, owner_uid: Number(e.target.value) })
                      }
                    />
                  </label>
                </div>
                <label className="crew-label">
                  Remote work folder (optional, absolute path)
                  <input
                    value={connectionForm.remote_root}
                    onChange={(e) =>
                      setConnectionForm({ ...connectionForm, remote_root: e.target.value })
                    }
                    placeholder="/home/your-user/project"
                  />
                </label>
                <label className="crew-check">
                  <input
                    type="checkbox"
                    checked={connectionForm.remote_execution}
                    onChange={(e) =>
                      setConnectionForm({ ...connectionForm, remote_execution: e.target.checked })
                    }
                  />
                  Allow my agent to execute tasks in this remote work folder
                </label>
                <p className="crew-small">
                  New connections are Private. SSH host keys and the workspace identity must be
                  independently verified.
                </p>
                <CrewHostTrust />
                <button className="crew-button primary" disabled={busy}>
                  Save connection
                </button>
                {editingConnection && (
                  <button
                    type="button"
                    className="crew-button"
                    disabled={busy}
                    onClick={() =>
                      void act(async () => {
                        await crewHttp(`/connections/${editingConnection}`, 'DELETE');
                        setPanel(null);
                        await loadConnections();
                      })
                    }
                  >
                    Remove saved connection
                  </button>
                )}
              </form>
            ) : (
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  void act(async () => {
                    if (panel === 'reference') {
                      const result = await request<{ id: string }>(
                        'reference.create',
                        {
                          channel_id: channelId,
                          path: remoteReferencePath,
                          label: remoteReferenceLabel || remoteReferencePath,
                        },
                        true
                      );
                      setReferences((items) => [
                        ...items,
                        { id: result.id, label: remoteReferenceLabel || remoteReferencePath },
                      ]);
                      setRemoteReferencePath('');
                      setRemoteReferenceLabel('');
                      setPanel(null);
                    }
                    if (panel === 'team') await mutate('team.create', { name });
                    if (panel === 'channel')
                      await mutate('channel.create', { name, team_id: teamId, classification });
                    if (panel === 'profile')
                      await mutate('profile.update', { nickname: name, avatar: avatar || null });
                    if (panel === 'enroll') {
                      const result = await request<{ invitation: string }>(
                        'enrollment.invite',
                        {
                          uid: Number(inviteUid),
                          public_key: invitePublicKey,
                          existing_principal_id: addExistingDevice
                            ? existingEnrollee?.id
                            : undefined,
                        },
                        true
                      );
                      setCreatedInvitation(result.invitation);
                    }
                    if (panel === 'offboard') {
                      if (!offboardPerson || name !== offboardPerson.username) {
                        throw new Error(
                          'Enter the exact account username to remove workspace access.'
                        );
                      }
                      await mutate('enrollment.revoke', { principal_id: personId });
                    }
                    if (panel === 'invite')
                      await mutate('invitation.create', {
                        kind: inviteKind,
                        target_id: inviteKind === 'team' ? teamId : channelId,
                        principal_id: personId,
                      });
                    if (panel === 'settings')
                      await mutate('channel.transfer', {
                        channel_id: channelId,
                        successor_id: personId,
                      });
                    if (panel === 'grant' && grantSessionId) {
                      if (!snapshot || observedPrivacy?.connectionId !== connectionId)
                        throw new Error(
                          'Refresh the workspace to verify connection privacy before granting agent access.'
                        );
                      await crewHttp(
                        `/connections/${connectionId}/sessions/${encodeURIComponent(grantSessionId)}/grant`,
                        'POST',
                        {
                          expected_mode: observedPrivacy.mode,
                          channel_id: channelId,
                          context_channels: [channelId, ...contextChannels],
                        }
                      );
                      navigate(`/pair?resumeSessionId=${encodeURIComponent(grantSessionId)}`);
                    }
                    if (panel === 'agent') await submitOwnedRun();
                  });
                }}
              >
                {panel === 'reference' && (
                  <>
                    <p>
                      Share a path on <strong>{connection?.ssh_target}</strong> without uploading
                      the file. This does not verify that the file exists, grant access, or run
                      anything.
                    </p>
                    <label className="crew-label">
                      Remote absolute path
                      <input
                        required
                        value={remoteReferencePath}
                        onChange={(event) => setRemoteReferencePath(event.target.value)}
                        pattern="/.*"
                        placeholder="/home/your-user/project/large-dataset.h5ad"
                      />
                    </label>
                    <label className="crew-label">
                      Display label
                      <input
                        value={remoteReferenceLabel}
                        maxLength={255}
                        onChange={(event) => setRemoteReferenceLabel(event.target.value)}
                      />
                    </label>
                    <p>
                      References stay restricted. An agent needs a separate approved remote-folder
                      grant to read the target.
                    </p>
                  </>
                )}
                {panel === 'enroll' && (
                  <>
                    <p>
                      The colleague must supply their own remote Unix UID and the enrollment public
                      key shown in their saved Crew connection.
                    </p>
                    <label className="crew-label">
                      Colleague Unix UID
                      <input
                        type="number"
                        min="1"
                        required
                        value={inviteUid}
                        onChange={(e) => {
                          setInviteUid(e.target.value);
                          setAddExistingDevice(false);
                          setCreatedInvitation('');
                        }}
                      />
                    </label>
                    <label className="crew-label">
                      Colleague enrollment public key
                      <input
                        required
                        pattern="[a-fA-F0-9]{64}"
                        value={invitePublicKey}
                        onChange={(e) => setInvitePublicKey(e.target.value)}
                      />
                    </label>
                    {existingEnrollee && (
                      <label className="crew-check">
                        <input
                          type="checkbox"
                          required
                          checked={addExistingDevice}
                          onChange={(event) => setAddExistingDevice(event.target.checked)}
                        />
                        Add another device for @{existingEnrollee.username} (UID{' '}
                        {existingEnrollee.uid}), keeping this person’s current memberships. If this
                        Unix account has been reassigned, remove the old person’s workspace access
                        before enrolling.
                      </label>
                    )}
                    {createdInvitation && (
                      <label className="crew-label">
                        Invitation token (share directly with this colleague)
                        <textarea readOnly value={createdInvitation} />
                      </label>
                    )}
                  </>
                )}
                {panel === 'offboard' && offboardPerson && (
                  <>
                    <p>
                      Remove @{offboardPerson.username} from this workspace and revoke all their
                      devices and active grants. Their messages remain in channel history. A later
                      enrollment creates a new identity without these memberships.
                    </p>
                    <label className="crew-label">
                      Type {offboardPerson.username} to confirm
                      <input
                        required
                        value={name}
                        onChange={(event) => setName(event.target.value)}
                      />
                    </label>
                  </>
                )}
                {['team', 'channel', 'profile'].includes(panel) && (
                  <label className="crew-label">
                    {panel === 'profile' ? 'Nickname' : 'Name'}
                    <input
                      required
                      autoFocus
                      value={name}
                      onChange={(e) => setName(e.target.value)}
                    />
                  </label>
                )}
                {panel === 'profile' && (
                  <>
                    <label className="crew-label">
                      Avatar (emoji or initials)
                      <input
                        value={avatar}
                        maxLength={12}
                        onChange={(e) => setAvatar(e.target.value)}
                      />
                    </label>
                    <p>Your verified SSH username stays visible alongside your nickname.</p>
                  </>
                )}
                {panel === 'channel' && (
                  <label className="crew-label">
                    Content policy
                    <select
                      value={classification}
                      onChange={(e) => setClassification(e.target.value)}
                    >
                      <option value="restricted">Restricted</option>
                      <option value="public_safe">Public-safe</option>
                    </select>
                    <p>Membership and connection privacy still apply.</p>
                  </label>
                )}
                {(panel === 'invite' || panel === 'settings') && (
                  <label className="crew-label">
                    {panel === 'settings' ? 'New owner (must accept transfer)' : 'Teammate'}
                    <select required value={personId} onChange={(e) => setPersonId(e.target.value)}>
                      <option value="">Choose a person</option>
                      {snapshot?.principals
                        .filter(
                          (person) =>
                            person.id !== snapshot.actor.id &&
                            (panel !== 'settings' || channel?.members.includes(person.id))
                        )
                        .map((person) => (
                          <option key={person.id} value={person.id}>
                            {person.nickname || person.username} (@{person.username})
                          </option>
                        ))}
                    </select>
                  </label>
                )}
                {panel === 'settings' && (
                  <>
                    <p>
                      Created by {channel ? actorName(channel.created_by) : ''}. Current owner:{' '}
                      {channel ? actorName(channel.owner_id) : ''}.
                    </p>
                    <p>
                      Only the current owner can archive or transfer this channel. The original
                      creator remains in its history.
                    </p>
                    <button
                      type="button"
                      className="crew-button"
                      disabled={busy || channel?.archived}
                      onClick={() =>
                        void act(() => mutate('channel.archive', { channel_id: channelId }))
                      }
                    >
                      Archive channel
                    </button>
                    <h3 className="crew-section-title">Channel members</h3>
                    {channel?.members
                      .filter((id) => id !== snapshot?.actor.id)
                      .map((id) => (
                        <div className="crew-inline" key={id}>
                          <span>{actorName(id)}</span>
                          <button
                            type="button"
                            className="crew-button"
                            disabled={busy}
                            onClick={() =>
                              void act(() =>
                                mutate('membership.revoke', {
                                  channel_id: channelId,
                                  principal_id: id,
                                })
                              )
                            }
                          >
                            Remove from channel
                          </button>
                        </div>
                      ))}
                  </>
                )}
                {panel === 'agent' && unknownRunDestination && (
                  <section className="crew-error" aria-label="Previous task outcome unknown">
                    <strong>Inspect the previous task before starting again</strong>
                    <p>
                      The earlier request to {unknownRunDestination} was admitted, but its final
                      setup outcome is unknown. No new task will start automatically.
                    </p>
                    <p>
                      Inspect Crew run cards, the relevant task conversations in Chat history, and
                      any remote outputs or jobs. Repeating the task could duplicate earlier
                      effects.
                    </p>
                    <button
                      type="button"
                      className="crew-button"
                      disabled={busy}
                      onClick={() => {
                        setPanel(null);
                        setInspectedPriorRun(false);
                      }}
                    >
                      Inspect Crew run cards
                    </button>
                    <button
                      type="button"
                      className="crew-button"
                      disabled={busy}
                      onClick={() => {
                        setInspectedPriorRun(false);
                        navigate('/sessions');
                      }}
                    >
                      Open Chat history
                    </button>
                    <label className="crew-check">
                      <input
                        type="checkbox"
                        checked={inspectedPriorRun}
                        onChange={(event) => setInspectedPriorRun(event.target.checked)}
                      />
                      I inspected the previous task and remote effects, and want to start a new
                      task.
                    </label>
                    <button
                      type="button"
                      className="crew-button"
                      disabled={busy || !inspectedPriorRun}
                      onClick={(event) => {
                        if (event.currentTarget.form?.reportValidity())
                          void act(() => submitOwnedRun(true));
                      }}
                    >
                      Start a new task
                    </button>
                  </section>
                )}
                {(panel === 'agent' || panel === 'grant') && (
                  <>
                    <p>
                      Your agent will post to{' '}
                      <strong>
                        {connection?.name} / {team?.name} / #{channel?.name}
                      </strong>
                      . This grants permission for this task and destination only.
                    </p>
                    {panel === 'agent' && (
                      <>
                        <label className="crew-label">
                          Task
                          <textarea
                            required
                            value={body}
                            onChange={(e) => setBody(e.target.value)}
                          />
                        </label>
                        <label className="crew-label">
                          Configured provider
                          <select
                            required
                            value={provider}
                            onChange={(e) => setProvider(e.target.value)}
                          >
                            <option value="">Choose a configured provider</option>
                            {availableProviders.map((item) => (
                              <option key={item.name} value={item.name}>
                                {item.name} · {item.resolved_tier || 'policy checked by server'}
                              </option>
                            ))}
                          </select>
                        </label>
                        <label className="crew-label">
                          Model
                          <input
                            required
                            list="crew-model-options"
                            value={model}
                            onChange={(e) => setModel(e.target.value)}
                          />
                          <datalist id="crew-model-options">
                            {availableModels.map((item) => (
                              <option key={item} value={item} />
                            ))}
                          </datalist>
                        </label>
                      </>
                    )}
                    <div className="crew-run">
                      <span>
                        Remote work folder:{' '}
                        <strong>{connection?.remote_root || 'Not configured'}</strong>
                        <br />
                        Single-process remote execution:{' '}
                        <strong>
                          {connection?.remote_execution
                            ? 'Allowed for this task within that folder'
                            : 'Not allowed'}
                        </strong>
                      </span>
                    </div>
                    <fieldset>
                      <legend>Additional context channels</legend>
                      <p>
                        Current channel is included. Other channels are used only when you select
                        them.
                      </p>
                      {snapshot?.channels
                        .filter((item) => item.id !== channelId && !item.archived)
                        .map((item) => (
                          <label className="crew-check" key={item.id}>
                            <input
                              type="checkbox"
                              checked={contextChannels.includes(item.id)}
                              onChange={(e) =>
                                setContextChannels(
                                  e.target.checked
                                    ? [...contextChannels, item.id]
                                    : contextChannels.filter((id) => id !== item.id)
                                )
                              }
                            />
                            {snapshot.teams.find((t) => t.id === item.team_id)?.name} / #{item.name}
                          </label>
                        ))}
                    </fieldset>
                    <p className="crew-small">
                      Private connections block public models. Restricted sources keep their
                      permissions when summarized.
                    </p>
                  </>
                )}
                <button
                  className="crew-button primary"
                  disabled={
                    busy ||
                    (panel === 'agent' && Boolean(unknownRunDestination)) ||
                    (panel === 'offboard' && (!offboardPerson || name !== offboardPerson.username))
                  }
                >
                  {panel === 'offboard'
                    ? 'Remove workspace access'
                    : panel === 'grant'
                      ? 'Allow this conversation to read and post here'
                      : panel === 'agent'
                        ? 'Start my agent and allow posting here'
                        : panel === 'settings'
                          ? 'Request ownership transfer'
                          : panel === 'invite'
                            ? 'Send invitation'
                            : 'Save'}
                </button>
              </form>
            )}
            {error && (
              <p role="alert" className="crew-error">
                {error}
              </p>
            )}
          </section>
        </div>
      )}
    </div>
  );
}
