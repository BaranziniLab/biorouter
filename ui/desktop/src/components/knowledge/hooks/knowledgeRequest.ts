import { client } from '../../../api/client.gen';
import { userActionHeaders } from '../../../utils/userAction';

type ElectronBridge = {
  getBiorouterdHostPort?: () => Promise<string | null>;
  getSecretKey?: () => Promise<string>;
};

function getElectronBridge(): ElectronBridge | undefined {
  return (window as unknown as { electron?: ElectronBridge }).electron;
}

export async function getSecretKey(): Promise<string> {
  const electron = getElectronBridge();
  if (electron?.getSecretKey) {
    try {
      return await electron.getSecretKey();
    } catch {
      return '';
    }
  }
  return '';
}

export async function getBackendBaseUrl(): Promise<string> {
  const electron = getElectronBridge();
  if (electron?.getBiorouterdHostPort) {
    try {
      const baseUrl = await electron.getBiorouterdHostPort();
      if (baseUrl) {
        return baseUrl.replace(/\/$/, '');
      }
    } catch {
      // Fall back to the SDK client config below.
    }
  }

  const cfg = client.getConfig();
  return ((cfg.baseUrl as string | undefined) ?? '').replace(/\/$/, '');
}

export async function buildKnowledgeUrl(path: string): Promise<string> {
  return `${await getBackendBaseUrl()}${path}`;
}

/**
 * A request to the daemon's `/knowledge/*` routes, as the Knowledge view makes
 * it: with the secret, and with the user-action proof.
 *
 * Issue #56, QA 2026-09-10 H2: every route that names a knowledge base now
 * answers a caller WITHOUT the proof as a public model — a private base is
 * refused and omitted from listings — because a public chat's shell could
 * recover the secret and read private bases with it. The desktop is the person
 * at the keyboard, so it says so on every knowledge request; a request that
 * forgot would see private bases vanish, not an error.
 */
export async function knowledgeFetch(path: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers ?? {});
  const secret = await getSecretKey();
  if (secret) {
    headers.set('X-Secret-Key', secret);
  }
  for (const [name, value] of Object.entries(await userActionHeaders())) {
    headers.set(name, value);
  }

  return fetch(await buildKnowledgeUrl(path), {
    ...init,
    headers,
  });
}

export interface ExpandedKnowledgePathFile {
  path: string;
  name: string;
  relative_path: string;
}

export interface ExpandedKnowledgePathWarning {
  level: 'warning' | 'error';
  title: string;
  message: string;
}

export interface ExpandedKnowledgePathResponse {
  files: ExpandedKnowledgePathFile[];
  warnings: ExpandedKnowledgePathWarning[];
}

export async function expandKnowledgePath(path: string): Promise<ExpandedKnowledgePathResponse> {
  const response = await knowledgeFetch('/knowledge/expand-path', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ path }),
  });

  if (!response.ok) {
    throw new Error(await response.text());
  }

  return (await response.json()) as ExpandedKnowledgePathResponse;
}
