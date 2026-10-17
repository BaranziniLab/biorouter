import WebSocket from 'ws';
import { StringDecoder } from 'node:string_decoder';

export interface CrewDaemonAccess {
  baseUrl: string;
  secret: string;
  userAction: string;
}

/**
 * What a sign-in is for: a saved connection, or a setup job waiting for the person to sign in
 * (OW-9). Both prepare a session the same way and share the one terminal route.
 */
export type CrewDaemonTerminalTarget =
  | { kind: 'connection'; connectionId: string }
  | { kind: 'setup'; jobId: string };

/** The daemon route that prepares the sign-in session for `target`. */
export function crewAuthenticationPreparePath(target: CrewDaemonTerminalTarget): string {
  return target.kind === 'connection'
    ? `/crew/connections/${encodeURIComponent(target.connectionId)}/authentication`
    : `/crew/setup/${encodeURIComponent(target.jobId)}/authentication`;
}

/** A typed daemon code a terminal error frame carried (`crew_bridge_missing`, …), else null. */
function frameCode(value: unknown): string | null {
  return typeof value === 'string' && /^crew_[a-z0-9_]{1,64}$/.test(value) ? value : null;
}

/**
 * Main-process only: proof and controller handles never enter renderer state. `target` is a saved
 * connection's id, or a target naming a setup job. `onExit` hears the exit code, and the typed code
 * of the daemon's last error frame (a handoff that found Crew missing on the server says
 * `crew_bridge_missing`, so the renderer can offer setup rather than a dead end).
 */
export async function createCrewDaemonTerminal(
  access: CrewDaemonAccess,
  target: string | CrewDaemonTerminalTarget,
  onData: (data: string) => void,
  onExit: (code: number | null, failureCode?: string | null) => void
) {
  const resolved: CrewDaemonTerminalTarget =
    typeof target === 'string' ? { kind: 'connection', connectionId: target } : target;
  const controller = crypto.randomUUID();
  const headers = {
    'Content-Type': 'application/json',
    'X-Secret-Key': access.secret,
    'X-User-Action': access.userAction,
    'X-Crew-Controller': controller,
  };
  const response = await fetch(`${access.baseUrl}${crewAuthenticationPreparePath(resolved)}`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      request_id: crypto.randomUUID(),
      controller_id: controller,
      cols: 80,
      rows: 12,
    }),
    signal: AbortSignal.timeout(45000),
  });
  const prepared = await response.json();
  if (!response.ok) throw new Error(prepared.error || 'SSH authentication could not start.');
  // A connection's session names that connection. A setup's names the key its master rides (the
  // connection or the pending host setup), which this process does not know; it names the job, when
  // it says, and must name the one asked for.
  const bound =
    resolved.kind === 'connection'
      ? prepared.connection_id === resolved.connectionId
      : prepared.job_id === undefined || prepared.job_id === resolved.jobId;
  if (typeof prepared.authentication_id !== 'string' || !bound)
    throw new Error('Invalid daemon authentication session.');
  const endpoint = `${access.baseUrl}/crew/authentication/${encodeURIComponent(prepared.authentication_id)}`;
  const websocketUrl = new URL(`${endpoint}/terminal`);
  websocketUrl.protocol = websocketUrl.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(websocketUrl, {
    headers,
    maxPayload: 16384,
    handshakeTimeout: 45000,
  });
  const decoder = new StringDecoder('utf8');
  let disposed = false;
  let exitCode: number | null = null;
  let failureCode: string | null = null;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    socket.terminate();
    void fetch(endpoint, { method: 'DELETE', headers, signal: AbortSignal.timeout(10000) }).catch(
      () => {}
    );
  };
  socket.on('message', (data, binary) => {
    if (disposed) return;
    if (binary) {
      const bytes = Buffer.isBuffer(data) ? data : Buffer.from(data as ArrayBuffer);
      onData(decoder.write(bytes));
      return;
    }
    try {
      const event = JSON.parse(data.toString());
      if (event.type === 'exit')
        exitCode =
          event.authenticated === true
            ? 0
            : event.exit_code === 0
              ? null
              : (event.exit_code ?? null);
      if (event.type === 'error') {
        failureCode = frameCode(event.code) ?? failureCode;
        onData('\r\nAuthentication refused. Close and reconnect.\r\n');
      }
    } catch {
      dispose();
    }
  });
  socket.on('close', () => {
    const remaining = decoder.end();
    if (remaining && !disposed) onData(remaining);
    onExit(exitCode, failureCode);
  });
  socket.on('error', () => {});
  try {
    await new Promise<void>((resolve, reject) => {
      socket.once('open', resolve);
      socket.once('error', () =>
        reject(new Error('Could not attach to daemon SSH authentication.'))
      );
      socket.once('close', () => reject(new Error('Daemon SSH authentication closed.')));
    });
  } catch (error) {
    dispose();
    throw error;
  }
  const send = (event: object) => {
    if (disposed || socket.readyState !== WebSocket.OPEN || socket.bufferedAmount > 65536)
      throw new Error('Authentication terminal unavailable; close and reconnect.');
    socket.send(JSON.stringify(event));
  };
  return {
    dispose,
    write(data: string) {
      if (Buffer.byteLength(data, 'utf8') > 4096)
        throw new Error('Authentication input exceeds frame limit.');
      send({ type: 'input', data });
    },
    resize(cols: number, rows: number) {
      send({
        type: 'resize',
        cols: Math.max(20, Math.min(500, Math.floor(cols))),
        rows: Math.max(5, Math.min(200, Math.floor(rows))),
      });
    },
  };
}
