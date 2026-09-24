import type { Channel, CrewConnection, CrewMessage, ObservedRun, Snapshot, Team } from '../crewApi';
import type { ConnectFailureKind } from './connectFailure';
import type { ConnectionStatusKey, CrewScreen } from './crewStatus';

/** Options a layout passes to the controller. The legacy layout passes none. */
export interface CrewControllerOptions {
  /**
   * Open Sign in automatically after a user-initiated connect fails with
   * `crew_ssh_auth_required`. Legacy: false.
   */
  autoOpenSignIn?: boolean;
  /** Keep a presentation-only copy of the last verified view during refresh. Legacy: false. */
  keepLastVerifiedView?: boolean;
}

/** The connection privacy the observer verified for one connection. */
export interface ObservedPrivacy {
  connectionId: string;
  mode: 'private' | 'public';
  institutionId: string | null;
  policyEpoch: number;
}

/**
 * The daemon's projected labels from an observation `state` frame (naming slice S1a), passed
 * through untouched. The validator and the typed shape belong to `crewApi.ts`; read it there.
 */
export type CrewFrameLabels = Readonly<Record<string, unknown>>;

/**
 * A presentation-only copy of the last verified view of one connection. It is never used to
 * authorize, enable or send anything; `clearProtectedState` drops it with the live state.
 */
export interface VerifiedView {
  connectionId: string;
  snapshot: Snapshot;
  observedPrivacy: ObservedPrivacy;
  runs: ObservedRun[];
  labels: CrewFrameLabels | null;
  teamId: string;
  channelId: string;
  /** The selected channel's messages when they had loaded; otherwise an earlier copy or none. */
  messages: CrewMessage[];
}

/** A file already uploaded and waiting in the composer. */
export interface DraftFile {
  id: string;
  name: string;
}
/** A server path shared by reference and waiting in the composer. */
export interface DraftReference {
  id: string;
  label: string;
}
export interface CrewDraft {
  body: string;
  attachments: DraftFile[];
  references: DraftReference[];
}

/** The body the daemon accepts for `POST /crew/connections` and `PATCH /crew/connections/{id}`. */
export interface SaveConnectionInput {
  name: string;
  ssh_target: string;
  port?: number;
  identity_file?: string;
  proxy_jump?: string;
  socket_path: string;
  owner_uid: number;
  workspace_id: string;
  workspace_public_key: string;
  cluster_connection_id?: string;
  remote_root?: string;
  remote_execution: boolean;
  mode: 'private' | 'public';
  institution_id: string | null;
  preparation_id?: string;
}

/** `POST /crew/devices/prepare`: the prepared (or recovered) hosting identity. */
export interface PreparedDevice {
  preparation_id: string;
  public_key: string;
  device_id: string;
}

export interface LastConnectFailure {
  kind: ConnectFailureKind;
  /** The daemon's text, unchanged. */
  message: string;
  /** The daemon's typed code, when it sent one. */
  code?: string;
  /** Bounded, redacted OpenSSH output for "Copy details" only. */
  detail?: string;
}

/** Join status reported by `GET /crew/connections/{id}/join` (naming slice S3a). */
export type CrewJoinStatus =
  | 'invited'
  | 'approved'
  | 'code_mismatch'
  | 'not_invited'
  | 'expired'
  | 'joined'
  | 'unsupported'
  | (string & {});

export type WorkspaceSettingsTab = 'general' | 'people' | 'privacy' | 'agent-access';
export type DetailsTab = 'about' | 'members' | 'files' | 'access';

/** Every confirmation of the spec's confirmation table that is a dialog (not an inline two-step). */
export type ConfirmIntent =
  | { action: 'make-connection-public'; connectionId: string }
  | { action: 'allow-workspace-public' }
  | { action: 'make-workspace-private' }
  | { action: 'set-institution'; institutionId: string }
  | { action: 'remove-person'; principalId: string }
  | { action: 'archive-channel'; channelId: string }
  | { action: 'remove-channel-member'; channelId: string; principalId: string }
  | { action: 'remove-connection'; connectionId: string }
  | { action: 'stop-task'; runId: string };

/** Every dialog of the spec's dialog inventory. Areas open each other's dialogs through these. */
export type DialogIntent =
  | { kind: 'join' }
  | { kind: 'host' }
  | { kind: 'connection-settings'; connectionId: string }
  | { kind: 'workspace-settings'; tab?: WorkspaceSettingsTab }
  | { kind: 'invite-people' }
  | { kind: 'let-in'; username: string }
  | { kind: 'create-team' }
  | { kind: 'create-channel'; teamId: string }
  | { kind: 'add-people'; target: 'team' | 'channel'; targetId: string }
  | { kind: 'transfer-ownership'; channelId: string; successorId?: string }
  | { kind: 'rename'; target: 'team' | 'channel' | 'workspace'; targetId: string }
  | { kind: 'edit-profile' }
  | { kind: 'keys' }
  | { kind: 'share-path' }
  /** Sign in is tracked by `signIn`; opening this intent is the same as `openSignIn()`. */
  | { kind: 'sign-in' }
  | { kind: 'confirm'; confirm: ConfirmIntent };
export type DialogKind = DialogIntent['kind'];

/** The one non-modal details pane has one mode at a time. */
export type PaneIntent =
  | { mode: 'details'; tab?: DetailsTab }
  | { mode: 'agent' }
  | { mode: 'chat-access'; sessionId?: string };
export type PaneMode = PaneIntent['mode'];

export interface CrewUi {
  dialog: DialogIntent | null;
  pane: PaneIntent | null;
}

/**
 * Where an action error came from. `errorSlotFor` renders it there while that surface is mounted,
 * otherwise in the connection bar. `connect` is a failed connect or sign-in: the surface that
 * explains its classified cause (Sign in, a trust pane, "not set up") registers for it; a cause with
 * no surface of its own (unreachable, unclassified) falls back to the bar.
 */
export type ErrorSource =
  | 'composer'
  | `pane:${PaneMode}`
  | `dialog:${DialogKind}`
  | 'connect'
  | 'observer'
  | 'global';

export interface CrewActionError {
  message: string;
  code?: string;
  source: ErrorSource;
}

/**
 * A pending-action key. A control disables only while its own action, or one that conflicts, runs.
 * Known keys: `send`, `run.start`, `run.cancel`, `grant`, `connect`, `disconnect`, `sign-in`,
 * `refresh`, `connection.save`, `connection.update`, `connection.remove`, `device.prepare`, and
 * `mutate:<method>` for broker mutations.
 */
export type ActionKey =
  | 'send'
  | 'run.start'
  | 'run.cancel'
  | 'grant'
  | 'connect'
  | 'disconnect'
  | 'sign-in'
  | 'refresh'
  | `mutate:${string}`
  | (string & {});

export interface ActOptions {
  /** Keep the current error while this action runs (legacy manual refresh). */
  preserveError?: boolean;
}

/** Why the controller reset the surfaces a layout may have open. */
export type SurfaceResetReason =
  | 'refresh'
  | 'protected-cleared'
  | 'channel-changed'
  | 'channel-revoked'
  | 'connection-changed'
  | 'mutated'
  | 'run-started';
export type SurfaceResetListener = (reason: SurfaceResetReason) => void;

export interface SignInState {
  open: boolean;
  reason: 'user' | 'auto' | null;
}

export interface StartOwnedRunInput {
  prompt: string;
  provider: string;
  model: string;
  /** Additional channels the agent may read. The current channel is always included first. */
  contextChannels: string[];
  deliberateRestart?: boolean;
  /**
   * Clear the composer body in the same update as a successful start (legacy, where the Task and
   * the draft are one field). The new layout uses `clearBodyIfEquals(seed)` instead.
   */
  clearBody?: boolean;
}

export interface CrewController {
  // Connections
  connections: CrewConnection[];
  connectionId: string;
  /** The saved connection merged with the privacy the observer verified for it. */
  connection: CrewConnection | null;
  /** `loading` until the first `GET /crew/connections` answers; `failed` if it never has. */
  connectionsState: 'loading' | 'loaded' | 'failed';
  selectConnection(id: string): void;
  /** POST, reload the list and select the saved connection. Throws on failure. */
  saveConnection(input: SaveConnectionInput): Promise<CrewConnection>;
  /** Full-body PATCH (L18) and reload the list. Throws on failure. */
  updateConnection(id: string, input: SaveConnectionInput): Promise<CrewConnection>;
  /** DELETE and reload the list. Throws on failure. */
  removeConnection(id: string): Promise<void>;
  /** Prepare or recover the hosting identity. Throws on failure. */
  prepareHostingDevice(): Promise<PreparedDevice>;
  /**
   * POST connect, reload, refresh. A failure is recorded twice: classified in `lastConnectFailure`
   * and as an error from the `connect` source. Never throws.
   */
  connect(opts?: { userInitiated?: boolean }): Promise<void>;
  /** POST disconnect, stop observing and clear the protected view. Never throws. */
  disconnect(): Promise<void>;
  lastConnectFailure: LastConnectFailure | null;
  /** Classify and record a failure a layout observed (for example sign-in ending with a code). */
  reportConnectFailure(failure: unknown): void;

  // Observation
  /** The verified snapshot only. */
  snapshot: Snapshot | null;
  /** Presentation only; cleared by `clearProtectedState`. Always null unless the option is set. */
  lastVerified: VerifiedView | null;
  observedPrivacy: ObservedPrivacy | null;
  runs: ObservedRun[];
  messages: CrewMessage[];
  /** True once the selected channel's messages (or a history page) arrived after the last reset. */
  messagesLoaded: boolean;
  /** The sequence before which a history page is shown, or null for the live tail. */
  historyBefore: string | null;
  labels: CrewFrameLabels | null;
  refreshError: string | null;
  /** Unchanged order: connections before observe. */
  refresh(): Promise<void>;
  loadOlder(): void;
  jumpToLatest(): void;

  // Selection
  teamId: string;
  channelId: string;
  team: Team | null;
  channel: Channel | null;
  selectTeam(id: string): void;
  selectChannel(id: string): void;

  // Actions, errors, pending
  act<T>(
    source: ErrorSource,
    key: ActionKey,
    fn: () => Promise<T>,
    options?: ActOptions
  ): Promise<T | undefined>;
  error: CrewActionError | null;
  /**
   * True when the current error renders in the slot of `source`: its own surface while that
   * registered surface is mounted, otherwise the connection bar, which asks for `global` (and
   * `observer`). Exactly one slot answers true for any error.
   */
  errorSlotFor(source: ErrorSource): boolean;
  /** A surface that can show its own errors registers while mounted; returns the unregister. */
  registerErrorSlot(source: ErrorSource): () => void;
  reportError(message: string, source?: ErrorSource, code?: string): void;
  dismissError(): void;
  isPending(key: ActionKey): boolean;
  /** Legacy: any action pending. */
  busy: boolean;
  request<T = unknown>(
    method: string,
    params?: Record<string, unknown>,
    opts?: { mutation?: boolean; signal?: AbortSignal }
  ): Promise<T>;
  /** request (as a mutation) → refresh (unless `refresh: false`) → close the dialog. Throws. */
  mutate<T = unknown>(
    method: string,
    params: Record<string, unknown>,
    opts?: { refresh?: boolean }
  ): Promise<T>;
  /** `channel.read` without a refresh (L12). Throws. */
  markRead(channelId: string, sequence: string): Promise<void>;

  // Composer (single flight and idempotency unchanged)
  draft: CrewDraft;
  setBody(body: string): void;
  addAttachment(file: DraftFile): void;
  removeAttachment(id: string): void;
  addReference(reference: DraftReference): void;
  removeReference(id: string): void;
  /** Additional context channels for Ask my agent and chat access; cleared with the draft. */
  contextChannels: string[];
  setContextChannels(ids: string[]): void;
  send(): Promise<void>;
  clearBodyIfEquals(seed: string): void;

  // Owned runs (module-scoped unknown-outcome lock, C10)
  /** Records its error under `pane:agent`; resolves true only when the start was accepted. */
  startOwnedRun(input: StartOwnedRunInput): Promise<boolean>;
  unknownRunDestination: string | null;
  inspectedPriorRun: boolean;
  setInspectedPriorRun(value: boolean): void;
  /** POST cancel, then refresh. Records its error under `global`; never throws. */
  cancelRun(runId: string): Promise<void>;

  // Chat grants (from ?sessionId)
  grantSessionId: string | null;
  /**
   * POST a read-and-post grant for a chat (default: `grantSessionId`) in the selected channel,
   * pinned to the verified privacy epochs. Does nothing without a chat. Throws; callers choose the
   * error source.
   */
  grantSession(input: { contextChannels: string[]; sessionId?: string }): Promise<void>;

  // Sign in
  signIn: SignInState;
  openSignIn(): void;
  closeSignIn(): void;
  /** Close, reload connections and refresh. Never a second POST connect. */
  onSignedIn(): void;

  // UI intents, so areas open each other's surfaces without importing each other
  ui: CrewUi;
  openDialog(intent: DialogIntent): void;
  closeDialog(): void;
  openPane(intent: PaneIntent): void;
  closePane(): void;
  /** Listen for the controller closing surfaces (refresh, lost access, a finished mutation…). */
  subscribeSurfaceReset(listener: SurfaceResetListener): () => void;

  // Join (S3a)
  joinStatus: CrewJoinStatus | null;
  /** A layout that polls `GET …/join` reports the answer for the selected connection here. */
  setJoinStatus(status: CrewJoinStatus | null): void;

  // Derived (pure functions in crewStatus.ts, exported for tests)
  status: ConnectionStatusKey | null;
  screen: CrewScreen;
  /** Null until the observer verified this connection's privacy. */
  effectivePrivacy: 'private' | 'public' | null;
  /** `host_principal_id` (S1) when present, else `actor.uid === host_uid`. */
  isHost: boolean;
}
