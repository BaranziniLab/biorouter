import { client } from '../../api/client.gen';
import type { AppExportResult } from '../../utils/appExportRequest';

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
  /**
   * Set only on a row the daemon could not read a manifest for: it lists such an
   * app as `{ id, title: id, kind: 'static', broken: '<why>' }` so it can still
   * be seen and deleted. Nothing else on the row is real. Older daemons leave the
   * app out of the list instead, and never send this. Test it with
   * `typeof app.broken === 'string'`: the reason may be empty, and null is not set.
   */
  broken?: string | null;
}

/**
 * Extensions that ship inside Biorouter itself, so an export never lists them: the recipient's
 * daemon already has them. Mirrors `BUILTIN_EXTENSION_NAMES` in
 * `crates/biorouter-mcp/src/agent_drafter/mod.rs`; a Rust test reads this file and fails when
 * the two lists disagree. The daemon filters them out of an export either way, so a drift here
 * only shows a row that carries nothing.
 */
export const BUILTIN_EXTENSIONS: readonly string[] = [
  'developer',
  'computercontroller',
  'webdocuments',
  'autovisualiser',
  'memory',
  'agent_drafter',
  'knowledge',
];

/**
 * Skills that ship with Biorouter, so an export never carries them: every install seeds its own
 * copy, and one installed over it under the seeded name could no longer be removed. Mirrors
 * `SHIPPED_SKILL_NAMES` in `crates/biorouter-mcp/src/agent_drafter/render.rs`, which a Rust test
 * holds to this file and to the seeder's own list. The daemon leaves them out either way.
 */
export const SHIPPED_SKILLS: readonly string[] = [
  'office-word',
  'office-powerpoint',
  'office-excel',
  'office-pdf',
  'about-biorouter',
  'develop-biorouter',
  'develop-biorouter-extension',
  'develop-biorouter-skill',
  'knowledge-choose-a-format',
  'knowledge-ingest-okf',
  'knowledge-ingest-biookf',
  'knowledge-lint',
  'update-soul',
  'knowledge-bases',
];

/** The parts of an app's agent config an export reads; `GET /apps` serves the whole manifest. */
interface ExportReferences {
  knowledge_base?: string | null;
  skills?: string[];
  extensions?: string[];
  capabilities?: {
    data?: { sources?: { kind?: string; ids?: string[] }[] } | null;
    memory?: { kb?: string | null; shared_kb?: string | null } | null;
  } | null;
}

const references = (app: AppManifest): ExportReferences =>
  (app.agent ?? {}) as unknown as ExportReferences;

/**
 * Every knowledge base the app reaches, sorted: its agent's, each `knowledge` data source's,
 * and its memory and shared-memory bases. Exactly what a full export carries when it is not told
 * otherwise (`export::referenced_knowledge_bases`). The dialog used to list the agent's one base
 * only, so the others travelled without ever being shown.
 */
export function referencedKnowledgeBases(app: AppManifest): string[] {
  const agent = references(app);
  const ids = new Set<string>();
  const add = (id: unknown) => {
    if (typeof id === 'string' && id.trim()) ids.add(id);
  };
  add(agent.knowledge_base);
  for (const source of agent.capabilities?.data?.sources ?? []) {
    if (source?.kind === 'knowledge') (source.ids ?? []).forEach(add);
  }
  add(agent.capabilities?.memory?.kb);
  add(agent.capabilities?.memory?.shared_kb);
  return [...ids].sort();
}

/** The app's skills that do not ship with Biorouter, de-duplicated, in manifest order. */
export function referencedSkills(app: AppManifest): string[] {
  return [
    ...new Set(
      (references(app).skills ?? []).filter(
        (name) => typeof name === 'string' && !SHIPPED_SKILLS.includes(name)
      )
    ),
  ];
}

/** The app's extensions that do not ship with Biorouter, in manifest order. */
export function exportableExtensions(app: AppManifest): string[] {
  return [
    ...new Set(
      (references(app).extensions ?? []).filter(
        (name) => typeof name === 'string' && !BUILTIN_EXTENSIONS.includes(name)
      )
    ),
  ];
}

/** The toast after an export landed: where, and what it carries. */
export function exportedMessage(result: Extract<AppExportResult, { status: 'exported' }>): string {
  const { summary } = result;
  const where = /\.zip$/.test(result.target)
    ? `Downloaded ${result.target}.`
    : `Exported to ${result.target}.`;
  if (summary.mode !== 'full') return where;
  const parts = [
    `${summary.kbs.length} knowledge base${summary.kbs.length === 1 ? '' : 's'}`,
    `${summary.skills.length} skill${summary.skills.length === 1 ? '' : 's'}`,
  ];
  return `${where} It carries ${parts.join(' and ')}.`;
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
