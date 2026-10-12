import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  EXPORT_REPORT_HEADER,
  decodeExportReport,
  exportAppInThisBrowser,
  exportRequestBody,
  parseExportRequest,
} from './appExportRequest';

const encode = (value: unknown) =>
  btoa(JSON.stringify(value)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

describe('the export request', () => {
  it('keeps "none of these" apart from "everything referenced" all the way to the body', () => {
    const request = parseExportRequest({
      mode: 'full',
      bundleDaemon: false,
      allowLintErrors: false,
      include: { knowledge_bases: [], skills: [], extensions: [] },
    });
    expect(typeof request).toBe('object');
    expect(JSON.parse(exportRequestBody(request as never))).toEqual({
      mode: 'full',
      include: { knowledge_bases: [], skills: [], extensions: [] },
      bundle_daemon: 'none',
      allow_lint_errors: false,
    });
  });

  it('sends no selection in launcher mode', () => {
    expect(
      JSON.parse(
        exportRequestBody({ mode: 'launcher', bundleDaemon: false, allowLintErrors: false })
      )
    ).toEqual({ mode: 'launcher', bundle_daemon: 'none', allow_lint_errors: false });
  });

  it('refuses a full request that leaves a list out, and anything that is not a request', () => {
    expect(
      parseExportRequest({ mode: 'full', include: { knowledge_bases: [], skills: [] } })
    ).toMatch(/extensions/);
    expect(parseExportRequest({ mode: 'full', include: { knowledge_bases: [1] } })).toMatch(
      /names/
    );
    expect(parseExportRequest({ mode: 'everything' })).toMatch(/launcher or full/);
    expect(parseExportRequest(null)).toMatch(/not readable/);
    // Anything but `true` is false: a renderer cannot turn on the lint waiver by accident.
    expect(parseExportRequest({ mode: 'launcher', allowLintErrors: 'yes' })).toMatchObject({
      allowLintErrors: false,
    });
  });

  it('reads the report header, and an unreadable one as an empty report of the asked mode', () => {
    const summary = decodeExportReport(
      encode({
        mode: 'full',
        files: 12,
        kbs: ['lab-notes'],
        notes: ["skipped knowledge base 'cohort': private"],
        lint_warnings: ['ERROR: x'],
      }),
      'launcher'
    );
    expect(summary).toMatchObject({
      mode: 'full',
      files: 12,
      kbs: ['lab-notes'],
      skills: [],
      notes: ["skipped knowledge base 'cohort': private"],
      lintWarnings: ['ERROR: x'],
    });
    expect(decodeExportReport('%%%', 'full')).toMatchObject({ mode: 'full', files: 0, notes: [] });
    expect(decodeExportReport(null, 'launcher').mode).toBe('launcher');
  });
});

describe('exportAppInThisBrowser', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  let clicked: { href: string; download: string }[];
  const realCreate = URL.createObjectURL;
  const realRevoke = URL.revokeObjectURL;

  beforeEach(() => {
    clicked = [];
    fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    URL.createObjectURL = vi.fn(() => 'blob:export') as typeof URL.createObjectURL;
    URL.revokeObjectURL = vi.fn();
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement
    ) {
      clicked.push({ href: this.href, download: this.download });
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    URL.createObjectURL = realCreate;
    URL.revokeObjectURL = realRevoke;
  });

  it('downloads the daemon zip and never names a path on the server', async () => {
    fetchMock.mockResolvedValue(
      new Response(new Blob(['PK']), {
        status: 200,
        headers: {
          'Content-Type': 'application/zip',
          [EXPORT_REPORT_HEADER]: encode({ mode: 'launcher', files: 9 }),
        },
      })
    );
    const result = await exportAppInThisBrowser('http://host:3000/', 'secret', 'demo', {
      mode: 'launcher',
      bundleDaemon: true,
      allowLintErrors: false,
    });
    expect(result).toMatchObject({ status: 'exported', target: 'demo.zip' });
    expect(clicked).toEqual([{ href: 'blob:export', download: 'demo.zip' }]);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('http://host:3000/apps/demo/export');
    expect(init.method).toBe('POST');
    expect(init.headers['X-Secret-Key']).toBe('secret');
    // The server's own binary is no use to whoever downloads the zip.
    expect(JSON.parse(init.body).bundle_daemon).toBe('none');
  });

  it('answers harness errors as lint, so the dialog can offer to export anyway', async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ error: 'harness ERRORs', findings: ['#missing'] }), {
        status: 422,
      })
    );
    expect(
      await exportAppInThisBrowser('http://host', 's', 'demo', {
        mode: 'launcher',
        bundleDaemon: false,
        allowLintErrors: false,
      })
    ).toEqual({ status: 'lint', message: 'harness ERRORs', findings: ['#missing'], retry: '' });
    expect(clicked).toEqual([]);
  });

  it('says what the daemon said when it refuses', async () => {
    fetchMock.mockResolvedValue(new Response("There is no app 'demo'.", { status: 404 }));
    expect(
      await exportAppInThisBrowser('http://host', 's', 'demo', {
        mode: 'launcher',
        bundleDaemon: false,
        allowLintErrors: false,
      })
    ).toEqual({ status: 'failed', message: "There is no app 'demo'." });
  });
});
