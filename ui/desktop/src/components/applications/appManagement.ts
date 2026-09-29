import { client } from '../../api/client.gen';

/**
 * The declared app contract (Apps SDK v2, Pillar 1) as serialized by the
 * daemon's `SurfaceDecl`. The server omits the whole block when nothing is
 * declared, so v1 apps simply have no `surface` key.
 */
export interface AppSurfaceDecl {
  state_schema?: unknown;
  actions?: { name: string; description?: string }[];
  signals?: { name: string }[];
  components?: { name: string }[];
}

/**
 * The app's theme selection (Apps SDK v2, Pillar 6) as serialized by the
 * daemon's `ThemeConfig`. Omitted entirely for the default `biorouter` look.
 */
export interface AppThemeConfig {
  pack?: string;
  accent?: string;
  tokens?: Record<string, string>;
}

/** An Agent-Drafter-built app, as returned by biorouterd `GET /apps`. */
export interface AppManifest {
  id: string;
  title: string;
  description?: string;
  kind: 'static' | 'agentic';
  created_at?: number;
  updated_at?: number;
  built_at?: number | null;
  /** Chat session this app was built in, when known (lets us reopen it). */
  session_id?: string;
  agent?: {
    model?: { provider?: string; model?: string };
    extensions?: string[];
    skills?: string[];
    knowledge_base?: string;
  } | null;
  /** Declared v2 surface (actions/signals/…); absent on v1 apps. */
  surface?: AppSurfaceDecl | null;
  /** Theme selection; absent when the app uses the default look. */
  theme?: AppThemeConfig | null;
}

/** What an export should carry. Mirrors `export_app`'s v2 params. */
export type ExportMode = 'launcher' | 'full';

/** Server-side payload items a full export bundles (design §3.9). */
export interface ExportInclude {
  knowledge_bases?: string[];
  skills?: string[];
  extensions?: string[];
}

export interface ExportOptions {
  mode: ExportMode;
  /** Bundle the platform-matching daemon binary (`bundle_daemon=current`). */
  bundleDaemon: boolean;
  include: ExportInclude;
}

/**
 * Build the `GET /apps/{id}/export` URL with the v2 query params. Servers that
 * predate the params ignore unknown query strings and return the same scaffold
 * map, so this degrades gracefully to today's launcher export.
 */
export function buildExportUrl(baseUrl: string, appId: string, options: ExportOptions): string {
  const params = new URLSearchParams();
  params.set('mode', options.mode);
  params.set('bundle_daemon', options.bundleDaemon ? 'current' : 'none');
  if (options.mode === 'full') {
    const include: ExportInclude = {};
    if (options.include.knowledge_bases?.length) {
      include.knowledge_bases = options.include.knowledge_bases;
    }
    if (options.include.skills?.length) include.skills = options.include.skills;
    if (options.include.extensions?.length) include.extensions = options.include.extensions;
    if (Object.keys(include).length > 0) params.set('include', JSON.stringify(include));
  }
  return `${baseUrl}/apps/${encodeURIComponent(appId)}/export?${params.toString()}`;
}

export function configuredBaseUrl(): string {
  const cfg = client.getConfig();
  return ((cfg.baseUrl as string) || '').replace(/\/+$/, '');
}

export async function resolveBaseUrl(): Promise<string> {
  const configured = configuredBaseUrl();
  if (configured) return configured;
  return ((await window.electron.getBiorouterdHostPort()) || '').replace(/\/+$/, '');
}

export async function secretHeader(): Promise<Record<string, string>> {
  try {
    const key = await window.electron.getSecretKey();
    if (key) return { 'X-Secret-Key': key };
  } catch {
    // Fall back to the generated client config below.
  }

  const cfg = client.getConfig() as {
    headers?: Record<string, string> | { get: (name: string) => string | null };
  };
  const headers = cfg.headers;
  if (headers && typeof headers.get === 'function') {
    const key = headers.get('X-Secret-Key');
    return key ? { 'X-Secret-Key': key } : {};
  }
  const headerRecord = headers as Record<string, string> | undefined;
  const key = headerRecord?.['X-Secret-Key'] ?? headerRecord?.['x-secret-key'];
  return key ? { 'X-Secret-Key': key } : {};
}

export async function requireOk(res: Response): Promise<void> {
  if (res.ok) return;
  const detail = await res.text().catch(() => '');
  throw new Error(`HTTP ${res.status}${detail ? `: ${detail}` : ''}`);
}

export async function deleteAgentDrafterApp(id: string): Promise<void> {
  const baseUrl = await resolveBaseUrl();
  if (!baseUrl) {
    throw new Error('Backend URL unavailable. Is biorouterd running?');
  }
  const res = await fetch(`${baseUrl}/apps/${encodeURIComponent(id)}`, {
    method: 'DELETE',
    headers: await secretHeader(),
  });
  await requireOk(res);
}
