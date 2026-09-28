import type { BrowserWindow } from 'electron';
import type { BiorouterdResult } from '../biorouterd';
import type { ManagedAppPreviewBackend } from './managedAppPreviewPolicy';

const revokedBackends = new WeakSet<BiorouterdResult['process']>();

/** Main-process provenance; a renderer-provided URL cannot mint this grant. */
export function bindManagedAppPreviewBackend(
  result: BiorouterdResult,
  owner: BrowserWindow,
  secretKey?: string
): ManagedAppPreviewBackend | undefined {
  if (!result.managed) return undefined;
  const controller = new AbortController();
  const revoke = () => {
    controller.abort();
    result.process.removeListener('exit', revokeBackend);
    result.process.removeListener('error', revokeBackend);
    owner.removeListener('closed', revoke);
  };
  const revokeBackend = () => {
    revokedBackends.add(result.process);
    revoke();
  };
  // close can lag exit indefinitely when descendants inherit stdio.
  result.process.once('exit', revokeBackend);
  result.process.once('error', revokeBackend);
  owner.once('closed', revoke);
  if (
    result.process.exitCode !== null ||
    result.process.signalCode != null ||
    revokedBackends.has(result.process) ||
    result.process.killed ||
    owner.isDestroyed()
  ) {
    revoke();
  }
  return { baseUrl: result.baseUrl, signal: controller.signal, secretKey };
}
