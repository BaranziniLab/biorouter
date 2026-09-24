import { describe, expect, it } from 'vitest';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import type { Session } from 'electron';
import {
  isAllowedArtifactFrameNavigation,
  isAllowedRendererPermission,
  isAppOrigin,
  shouldOpenExternalNavigation,
} from './permissionPolicy';

type CheckHandler = NonNullable<Parameters<Session['setPermissionCheckHandler']>[0]>;
type RequestHandler = NonNullable<Parameters<Session['setPermissionRequestHandler']>[0]>;
/** Every permission name either of the renderer partition's handlers can be asked about. */
type ElectronPermission = Parameters<CheckHandler>[1] | Parameters<RequestHandler>[1];

/**
 * Every permission Electron can hand the two handlers, spelled out so the test
 * can iterate them at runtime. `listIsExhaustive` below stops compiling, and
 * names the newcomer, when an Electron upgrade adds one: the policy already
 * denies it, and this makes someone look before a test says so.
 */
const EVERY_ELECTRON_PERMISSION = [
  'clipboard-read',
  'clipboard-sanitized-write',
  'deprecated-sync-clipboard-read',
  'display-capture',
  'fileSystem',
  'fullscreen',
  'geolocation',
  'hid',
  'idle-detection',
  'keyboardLock',
  'media',
  'mediaKeySystem',
  'midi',
  'midiSysex',
  'notifications',
  'openExternal',
  'pointerLock',
  'serial',
  'speaker-selection',
  'storage-access',
  'top-level-storage-access',
  'unknown',
  'usb',
  'window-management',
] as const satisfies ReadonlyArray<ElectronPermission>;
type Unlisted = Exclude<ElectronPermission, (typeof EVERY_ELECTRON_PERMISSION)[number]>;
const listIsExhaustive: [Unlisted] extends [never] ? true : Unlisted = true;

/** The packaged renderer's entry, in the shape `rendererEntryUrl()` builds it. */
const packagedRendererDir = path.resolve(
  'test-fixtures',
  'packaged-app',
  'renderer',
  'main_window'
);
const packagedEntry = pathToFileURL(path.join(packagedRendererDir, 'index.html'));
const devEntry = new URL('http://localhost:5173/');

describe('permissionPolicy', () => {
  it('matches only the configured development renderer origin', () => {
    const appUrl = new URL('http://localhost:5173/');
    expect(isAppOrigin('http://localhost:5173/pair', appUrl)).toBe(true);
    expect(isAppOrigin('http://127.0.0.1:5173/', appUrl)).toBe(false);
    expect(isAppOrigin('https://example.com/', appUrl)).toBe(false);
  });

  it('keeps packaged artifact files outside the renderer directory', () => {
    const rendererDir = path.resolve('test-fixtures', 'packaged-app', 'renderer');
    const appUrl = pathToFileURL(path.join(rendererDir, 'index.html'));
    expect(
      isAppOrigin(pathToFileURL(path.join(rendererDir, 'assets', 'app.js')).href, appUrl)
    ).toBe(true);
    expect(
      isAppOrigin(pathToFileURL(path.resolve('test-fixtures', 'artifact-1.html')).href, appUrl)
    ).toBe(false);
  });

  it('never hands Biorouter itself to the external browser', () => {
    const appUrl = new URL('http://localhost:5174/');
    expect(shouldOpenExternalNavigation('http://localhost:5174/', appUrl)).toBe(false);
    expect(shouldOpenExternalNavigation('http://localhost:5174/pair', appUrl)).toBe(false);
    expect(shouldOpenExternalNavigation('https://example.com/report', appUrl)).toBe(true);
    expect(shouldOpenExternalNavigation('https://user:secret@example.com/', appUrl)).toBe(false);
    expect(
      shouldOpenExternalNavigation(`https://example.com/${'x'.repeat(9 * 1024)}`, appUrl)
    ).toBe(false);
    expect(shouldOpenExternalNavigation('file:///tmp/report.pdf', appUrl)).toBe(false);
    expect(shouldOpenExternalNavigation('not a URL', appUrl)).toBe(false);
  });

  it('pins artifact frames to srcdoc, with no configurable escape', () => {
    expect(isAllowedArtifactFrameNavigation('about:srcdoc')).toBe(true);
    expect(isAllowedArtifactFrameNavigation('about:blank')).toBe(true);
    expect(isAllowedArtifactFrameNavigation('data:text/html,escape')).toBe(false);
    expect(isAllowedArtifactFrameNavigation('blob:https://example.test/id')).toBe(false);
    expect(isAllowedArtifactFrameNavigation('https://example.test/exfiltrate')).toBe(false);
    expect(isAllowedArtifactFrameNavigation('file:///etc/passwd')).toBe(false);
    // The daemon's own origin is no longer special. A figure used to be served
    // to an inline transcript iframe through `/mcp-ui-proxy`, which this policy
    // had to whitelist; the artifact panel builds its own srcdoc instead, so
    // there is nothing left on the daemon an artifact frame may reach. If this
    // ever goes green again, a second display surface has come back.
    expect(
      isAllowedArtifactFrameNavigation(
        'http://127.0.0.1:8765/mcp-ui-proxy?contentType=rawhtml&waitForRenderData=true'
      )
    ).toBe(false);
    expect(isAllowedArtifactFrameNavigation('http://127.0.0.1:8765/apps/escape/')).toBe(false);
  });

  it('allows only audio capture requested by Biorouter itself', () => {
    const appUrl = new URL('http://localhost:5173/');
    expect(
      isAllowedRendererPermission('media', 'http://localhost:5173/pair', appUrl, ['audio'])
    ).toBe(true);
    expect(
      isAllowedRendererPermission('media', 'http://localhost:5173/pair', appUrl, ['video'])
    ).toBe(false);
    expect(
      isAllowedRendererPermission('media', 'http://localhost:5173/pair', appUrl, ['audio', 'video'])
    ).toBe(false);
    expect(isAllowedRendererPermission('geolocation', 'http://localhost:5173/', appUrl, [])).toBe(
      false
    );
    expect(
      isAllowedRendererPermission('media', 'file:///tmp/biorouter-artifacts/evil.html', appUrl, [
        'audio',
      ])
    ).toBe(false);
  });
});

/**
 * P0-3: every Copy in the app failed with `NotAllowedError`, because the
 * renderer partition's handlers granted nothing but audio capture and
 * `navigator.clipboard.writeText` needs `clipboard-sanitized-write`.
 *
 * The URL shapes below are the ones Electron 39.8.10 was measured passing for
 * that permission: the check handler's `details.requestingUrl` is the
 * document's full committed URL (hash included) and its `requestingOrigin` is
 * a bare `file:///` for every packaged document; the request handler's
 * `requestingUrl` is the same full URL.
 */
describe('renderer clipboard permissions', () => {
  const WRITE = 'clipboard-sanitized-write';
  // The check handler passes `[details.mediaType ?? 'unknown']`, the request
  // handler `details.mediaTypes ?? []`; a clipboard decision ignores both.
  const handlerMediaShapes: ReadonlyArray<ReadonlyArray<string>> = [[], ['unknown']];

  it('lets the dev renderer write the clipboard', () => {
    for (const mediaTypes of handlerMediaShapes) {
      for (const url of ['http://localhost:5173/', 'http://localhost:5173/#/crew/general']) {
        expect(isAllowedRendererPermission(WRITE, url, devEntry, mediaTypes)).toBe(true);
      }
    }
  });

  it('lets the packaged file: entry write the clipboard', () => {
    const entry = packagedEntry.href;
    for (const mediaTypes of handlerMediaShapes) {
      for (const url of [entry, `${entry}#/crew/general`, `${entry}?tab=public#/`]) {
        expect(isAllowedRendererPermission(WRITE, url, packagedEntry, mediaTypes)).toBe(true);
      }
    }
  });

  it('refuses a clipboard write from any other origin', () => {
    for (const url of [
      'http://127.0.0.1:5173/',
      'http://localhost:5174/',
      'https://localhost:5173/',
      'https://example.com/',
      'about:srcdoc',
      'about:blank',
      'data:text/html,<p>x</p>',
      'null',
      '',
      packagedEntry.href,
    ]) {
      expect(isAllowedRendererPermission(WRITE, url, devEntry, [])).toBe(false);
    }
    for (const url of ['http://localhost:5173/', 'about:srcdoc', 'null', '']) {
      expect(isAllowedRendererPermission(WRITE, url, packagedEntry, [])).toBe(false);
    }
  });

  it('refuses a clipboard write from a local file outside the renderer directory', () => {
    const outside = [
      pathToFileURL(path.resolve('test-fixtures', 'artifact-1.html')).href,
      pathToFileURL(path.resolve('test-fixtures', 'packaged-app', 'index.html')).href,
      // Shares the renderer directory's name as a prefix, and nothing else.
      pathToFileURL(
        path.resolve('test-fixtures', 'packaged-app', 'renderer', 'main_window_x', 'i.html')
      ).href,
      `${pathToFileURL(packagedRendererDir).href}/../../evil.html`,
      'file:///etc/passwd',
    ];
    for (const url of outside) {
      expect(isAllowedRendererPermission(WRITE, url, packagedEntry, [])).toBe(false);
    }
  });

  it('refuses the bare file: origin, which names every local file at once', () => {
    // What the check handler's `requestingOrigin` holds for ANY packaged
    // document. Only `details.requestingUrl` says which file asked, so the
    // origin alone must never be enough.
    for (const origin of ['file:///', 'file://']) {
      expect(isAllowedRendererPermission(WRITE, origin, packagedEntry, [])).toBe(false);
    }
  });

  it('keeps every clipboard READ denied, even to the app itself', () => {
    for (const permission of ['clipboard-read', 'deprecated-sync-clipboard-read']) {
      for (const [url, appUrl] of [
        ['http://localhost:5173/', devEntry],
        [packagedEntry.href, packagedEntry],
      ] as const) {
        expect(isAllowedRendererPermission(permission, url, appUrl, [])).toBe(false);
        expect(isAllowedRendererPermission(permission, url, appUrl, ['audio'])).toBe(false);
      }
    }
  });

  it('grants the app itself nothing beyond the clipboard write and audio capture', () => {
    expect(listIsExhaustive).toBe(true);
    for (const [url, appUrl] of [
      ['http://localhost:5173/#/', devEntry],
      [packagedEntry.href, packagedEntry],
    ] as const) {
      for (const permission of EVERY_ELECTRON_PERMISSION) {
        for (const mediaTypes of handlerMediaShapes) {
          expect(
            isAllowedRendererPermission(permission, url, appUrl, mediaTypes),
            `${permission} ${JSON.stringify(mediaTypes)} from ${url}`
          ).toBe(permission === WRITE);
        }
      }
      // The audio rule is the only other grant, and it is unchanged.
      expect(isAllowedRendererPermission('media', url, appUrl, ['audio'])).toBe(true);
      expect(isAllowedRendererPermission('media', url, appUrl, ['audio', 'video'])).toBe(false);
    }
  });
});
