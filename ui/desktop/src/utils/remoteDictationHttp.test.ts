import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { webcrypto } from 'node:crypto';
import {
  addRemoteHttpAcknowledgement,
  hasRemoteHttpAcknowledgement,
  remoteHttpBackendUrl,
  remoteConsentKey,
} from './remoteDictationHttp';

describe('remote dictation acknowledgement', () => {
  beforeEach(() => vi.stubGlobal('crypto', webcrypto));
  afterEach(() => vi.unstubAllGlobals());
  it('warns for a non loopback HTTP backend and scopes the key to its URL', () => {
    expect(remoteHttpBackendUrl('http://LAB.example:8765/a/')).toEqual({
      url: 'http://lab.example:8765/a/',
      host: 'lab.example:8765',
    });
    expect(remoteHttpBackendUrl('http://lab.example:8765/b')?.url).not.toBe(
      remoteHttpBackendUrl('http://lab.example:8765/a')?.url
    );
  });

  it('does not warn for loopback HTTP or HTTPS', () => {
    expect(remoteHttpBackendUrl('http://127.0.0.1:8765')).toBeNull();
    expect(remoteHttpBackendUrl('http://127.0.0.2:8765')).toBeNull();
    expect(remoteHttpBackendUrl('http://localhost:8765')).toBeNull();
    expect(remoteHttpBackendUrl('http://[::1]:8765')).toBeNull();
    expect(remoteHttpBackendUrl('https://lab.example')).toBeNull();
  });

  it('hashes query and userinfo without storing their raw text', async () => {
    const first = await remoteConsentKey('http://user:password@lab.example/?token=alpha');
    const second = await remoteConsentKey('http://user:password@lab.example/?token=beta');
    expect(first).not.toBe(second);
    expect(first).toMatch(/^remote-http:sha256:[0-9a-f]{64}$/);
    expect(first).not.toContain('password');
    expect(first).not.toContain('token');
  });

  it('keeps other viewer settings and one acknowledgement per URL', () => {
    const first = addRemoteHttpAcknowledgement([], `remote-http:sha256:${'a'.repeat(64)}`);
    const second = addRemoteHttpAcknowledgement(first, `remote-http:sha256:${'b'.repeat(64)}`);
    const third = addRemoteHttpAcknowledgement(second, `remote-http:sha256:${'a'.repeat(64)}`);
    expect(third).toEqual([
      `remote-http:sha256:${'b'.repeat(64)}`,
      `remote-http:sha256:${'a'.repeat(64)}`,
    ]);
    expect(
      addRemoteHttpAcknowledgement(third, 'http://user:password@lab.example/?token=x')
    ).toEqual([]);
    expect(hasRemoteHttpAcknowledgement(third, `remote-http:sha256:${'a'.repeat(64)}`)).toBe(true);
    expect(hasRemoteHttpAcknowledgement(third, `remote-http:sha256:${'c'.repeat(64)}`)).toBe(false);
  });
});
