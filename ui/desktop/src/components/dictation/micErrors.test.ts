import { describe, expect, it } from 'vitest';
import { InsecureContextError } from './capture';
import { micErrorNotice, type MicErrorContext } from './micErrors';

const err = (name: string) => new DOMException('x', name);
const mac = (osStatus: MicErrorContext['osStatus']): MicErrorContext => ({
  platform: 'darwin',
  surface: 'desktop',
  osStatus,
});

describe('micErrorNotice: the error map of docs/dictation/surfaces.md', () => {
  it.each([
    [
      'NotAllowedError, macOS denied',
      err('NotAllowedError'),
      mac('denied'),
      'MSG-MIC-DENIED-MAC',
      'open-settings',
    ],
    [
      'NotAllowedError, macOS not determined',
      err('NotAllowedError'),
      mac('not-determined'),
      'MSG-MIC-DENIED-MAC',
      'open-settings',
    ],
    [
      'NotAllowedError, macOS restricted',
      err('NotAllowedError'),
      mac('restricted'),
      'MSG-MIC-RESTRICTED',
      null,
    ],
    [
      'NotAllowedError, macOS granted',
      err('NotAllowedError'),
      mac('granted'),
      'MSG-MIC-GRANTED-FAILED',
      null,
    ],
    [
      'NotAllowedError, Windows, any status',
      err('NotAllowedError'),
      { platform: 'win32', surface: 'desktop', osStatus: 'granted' },
      'MSG-MIC-DENIED-WIN',
      'open-settings',
    ],
    [
      'NotAllowedError, Linux desktop',
      err('NotAllowedError'),
      { platform: 'linux', surface: 'desktop', osStatus: 'unknown' },
      'MSG-MIC-BUSY',
      null,
    ],
    [
      'NotAllowedError, a browser',
      err('NotAllowedError'),
      { platform: 'darwin', surface: 'browser', osStatus: 'unknown' },
      'MSG-MIC-BROWSER',
      null,
    ],
    ['NotFoundError', err('NotFoundError'), mac('granted'), 'MSG-NO-DEVICE', null],
    [
      'OverconstrainedError after both retries',
      err('OverconstrainedError'),
      mac('granted'),
      'MSG-NO-DEVICE',
      null,
    ],
    ['NotReadableError', err('NotReadableError'), mac('granted'), 'MSG-MIC-BUSY', null],
    ['AbortError', err('AbortError'), mac('granted'), 'MSG-MIC-BUSY', null],
    ['something unexpected', new Error('boom'), mac('granted'), 'MSG-MIC-BUSY', null],
  ] as const)('%s', (_name, error, context, id, action) => {
    const notice = micErrorNotice(error, context as MicErrorContext);
    expect(notice.id).toBe(id);
    expect(notice.action).toBe(action);
    expect(notice.text.length).toBeGreaterThan(10);
  });

  it('an insecure context names the host', () => {
    const notice = micErrorNotice(new InsecureContextError(), {
      platform: 'linux',
      surface: 'browser',
      osStatus: 'unknown',
      host: '10.0.0.5:8765',
    });
    expect(notice.id).toBe('MSG-INSECURE');
    expect(notice.text).toContain('http://10.0.0.5:8765');
  });
});
