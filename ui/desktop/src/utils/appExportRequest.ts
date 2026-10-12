/**
 * Applications ▸ Export: the request every surface sends and the answer it gets back.
 *
 * One daemon route does the work, `POST /apps/{id}/export`, built on the same Rust export
 * the `export_app` tool uses (`agent_drafter/export.rs`). It answers a zip; the desktop's main
 * process unpacks it into the folder the person picked (`appExport.ts`), and a `biorouter serve`
 * browser downloads it (`exportAppInThisBrowser`, below). This module holds only what both
 * sides need, so the renderer can import it without pulling in Node.
 *
 * ⚠ The desktop used to GET a JSON map of text files and write each one through the generic
 * `write-file` IPC. The daemon ignored every option the dialog sent, so a Full export came out
 * as a launcher export; binary files were dropped; the launchers were written without the exec
 * bit; and an unchecked knowledge base was sent as "no key", which the daemon read as
 * "everything the app references". Each of those is why a piece of this file exists.
 */

export type AppExportMode = 'launcher' | 'full';

/**
 * A full export's selection. All three lists are sent, always: `[]` means "none of these", and
 * leaving a key out would mean "everything the app references".
 */
export interface AppExportInclude {
  knowledge_bases: string[];
  skills: string[];
  extensions: string[];
}

/** What the person chose in the export dialog. */
export interface AppExportRequest {
  mode: AppExportMode;
  /** Read in full mode only. */
  include?: AppExportInclude;
  /** Put this machine's `biorouterd` under `payload/bin/`. Not offered in a browser. */
  bundleDaemon: boolean;
  /** Export despite the harness errors a previous answer listed. Only a person sets this. */
  allowLintErrors: boolean;
  /**
   * The token of the `lint` answer being overridden: the desktop reuses the folder that answer
   * was for instead of asking for one again.
   */
  retry?: string;
}

/** The daemon's report of what an export carried, from `X-Biorouter-Export-Report`. */
export interface AppExportSummary {
  mode: AppExportMode;
  files: number;
  kbs: string[];
  skills: string[];
  extensions: string[];
  notes: string[];
  lintWarnings: string[];
  skipped: string[];
}

export type AppExportResult =
  /** Nothing was asked of the daemon, or the person declined to replace a folder. */
  | { status: 'canceled' }
  /**
   * `target` is the folder written, or on a browser the name of the downloaded zip. `legacy` is
   * an export an older background service made through `GET`: launcher-only, text files only.
   */
  | { status: 'exported'; target: string; summary: AppExportSummary; legacy: boolean }
  /** Harness errors. `retry` with `allowLintErrors` exports anyway. */
  | { status: 'lint'; message: string; findings: string[]; retry: string }
  | { status: 'failed'; message: string };

/** The response header carrying the export's report (base64url JSON). */
export const EXPORT_REPORT_HEADER = 'X-Biorouter-Export-Report';

/** An app id as the daemon mints them: what may be put in a path and a URL as it is. */
export const APP_ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;

const MAX_LIST_ITEMS = 500;
const MAX_NAME_LENGTH = 256;

function stringList(value: unknown, key: string): string[] | string {
  if (!Array.isArray(value)) return `include.${key} must be a list.`;
  if (value.length > MAX_LIST_ITEMS) return `include.${key} lists too many items.`;
  const names: string[] = [];
  for (const item of value) {
    if (typeof item !== 'string' || item.length === 0 || item.length > MAX_NAME_LENGTH) {
      return `include.${key} must list names.`;
    }
    names.push(item);
  }
  return names;
}

/**
 * Read a request that crossed a process boundary, or say what is wrong with it. The main
 * process asks this of everything the renderer sends: the shape is checked here, and the names
 * themselves by the daemon, which refuses a skill name that is a path before any join.
 */
export function parseExportRequest(raw: unknown): AppExportRequest | string {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    return 'The export request is not readable.';
  }
  const value = raw as Record<string, unknown>;
  if (value.mode !== 'launcher' && value.mode !== 'full') {
    return 'The export mode must be launcher or full.';
  }
  const request: AppExportRequest = {
    mode: value.mode,
    bundleDaemon: value.bundleDaemon === true,
    allowLintErrors: value.allowLintErrors === true,
  };
  if (typeof value.retry === 'string' && value.retry.length > 0 && value.retry.length <= 128) {
    request.retry = value.retry;
  }
  if (request.mode === 'full') {
    const include = value.include;
    if (typeof include !== 'object' || include === null || Array.isArray(include)) {
      return 'A full export must say which knowledge bases, skills and extensions it carries.';
    }
    const lists = include as Record<string, unknown>;
    const knowledgeBases = stringList(lists.knowledge_bases, 'knowledge_bases');
    const skills = stringList(lists.skills, 'skills');
    const extensions = stringList(lists.extensions, 'extensions');
    for (const list of [knowledgeBases, skills, extensions]) {
      if (typeof list === 'string') return list;
    }
    request.include = {
      knowledge_bases: knowledgeBases as string[],
      skills: skills as string[],
      extensions: extensions as string[],
    };
  }
  return request;
}

/** The JSON body of `POST /apps/{id}/export` for `request`. */
export function exportRequestBody(request: AppExportRequest): string {
  return JSON.stringify({
    mode: request.mode,
    ...(request.mode === 'full'
      ? {
          include: {
            knowledge_bases: request.include?.knowledge_bases ?? [],
            skills: request.include?.skills ?? [],
            extensions: request.include?.extensions ?? [],
          },
        }
      : {}),
    bundle_daemon: request.bundleDaemon ? 'current' : 'none',
    allow_lint_errors: request.allowLintErrors,
  });
}

const strings = (value: unknown): string[] =>
  Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];

function decodeBase64Url(raw: string): string {
  const base64 = raw.replace(/-/g, '+').replace(/_/g, '/');
  const padded = base64 + '='.repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

/**
 * The report header, read. A missing or unreadable header is an empty report with the mode
 * that was asked for: the folder still arrived, and the person is told what the daemon said
 * when it said anything.
 */
export function decodeExportReport(
  raw: string | null | undefined,
  requested: AppExportMode
): AppExportSummary {
  let value: Record<string, unknown> = {};
  if (raw) {
    try {
      const parsed: unknown = JSON.parse(decodeBase64Url(raw));
      if (typeof parsed === 'object' && parsed !== null) value = parsed as Record<string, unknown>;
    } catch {
      // An unreadable report is no report.
    }
  }
  return {
    mode: value.mode === 'full' || value.mode === 'launcher' ? value.mode : requested,
    files: typeof value.files === 'number' ? value.files : 0,
    kbs: strings(value.kbs),
    skills: strings(value.skills),
    extensions: strings(value.extensions),
    notes: strings(value.notes),
    lintWarnings: strings(value.lint_warnings),
    skipped: strings(value.skipped),
  };
}

/** A refusal's sentence, from a response that is not the zip. */
export async function refusalText(response: Response): Promise<string> {
  const body = (await response.text().catch(() => '')).trim();
  if (body) {
    try {
      const parsed: unknown = JSON.parse(body);
      if (typeof parsed === 'object' && parsed !== null) {
        const message = (parsed as Record<string, unknown>).error;
        if (typeof message === 'string' && message) return message;
      }
    } catch {
      // A plain sentence.
    }
    return body.length > 500 ? `${body.slice(0, 500)}…` : body;
  }
  return `The background service answered ${response.status}.`;
}

/** A 422's findings, as `{ error, findings }`. */
export async function lintRefusal(
  response: Response
): Promise<{ message: string; findings: string[] }> {
  try {
    const parsed = (await response.json()) as Record<string, unknown>;
    const message =
      typeof parsed.error === 'string' && parsed.error
        ? parsed.error
        : 'The app has problems that block its export.';
    return { message, findings: strings(parsed.findings) };
  } catch {
    return { message: 'The app has problems that block its export.', findings: [] };
  }
}

/**
 * The export on a `biorouter serve` browser: the daemon's zip, saved through the browser's own
 * download. A path on the server is never asked for and nothing is written on the server: the
 * old shim prompted for a folder "on the Ubuntu server" and wrote the export onto the host.
 *
 * The request goes to this page's own daemon with its secret, and the served document's cookie
 * rides along, so a knowledge base reaches the zip exactly when this browser could open it.
 */
export async function exportAppInThisBrowser(
  apiBaseUrl: string,
  secretKey: string,
  appId: unknown,
  raw: unknown,
  extraHeaders: Record<string, string> = {}
): Promise<AppExportResult> {
  if (typeof appId !== 'string' || !APP_ID_PATTERN.test(appId)) {
    return { status: 'failed', message: 'That is not an app name.' };
  }
  const request = parseExportRequest(raw);
  if (typeof request === 'string') return { status: 'failed', message: request };
  // A daemon binary built for the server is no use to whoever downloads the zip.
  const body = exportRequestBody({ ...request, bundleDaemon: false });
  let response: Response;
  try {
    response = await fetch(
      `${apiBaseUrl.replace(/\/+$/, '')}/apps/${encodeURIComponent(appId)}/export`,
      {
        method: 'POST',
        credentials: 'same-origin',
        headers: { ...extraHeaders, 'Content-Type': 'application/json', 'X-Secret-Key': secretKey },
        body,
      }
    );
  } catch (error) {
    return {
      status: 'failed',
      message: `Could not reach Biorouter: ${error instanceof Error ? error.message : String(error)}`,
    };
  }
  if (response.status === 422) {
    const { message, findings } = await lintRefusal(response);
    return { status: 'lint', message, findings, retry: '' };
  }
  if (!response.ok) return { status: 'failed', message: await refusalText(response) };
  const summary = decodeExportReport(response.headers.get(EXPORT_REPORT_HEADER), request.mode);
  const blob = await response.blob();
  const name = `${appId}.zip`;
  const url = URL.createObjectURL(blob);
  try {
    const link = document.createElement('a');
    link.href = url;
    link.download = name;
    link.rel = 'noopener';
    document.body.appendChild(link);
    link.click();
    link.remove();
  } finally {
    // The click has handed the blob to the download; the URL is not needed after this turn.
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
  }
  return { status: 'exported', target: name, summary, legacy: false };
}
