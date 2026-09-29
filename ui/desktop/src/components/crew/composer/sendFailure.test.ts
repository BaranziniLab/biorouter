import { describe, expect, it } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { refusalCopy } from '../dialogs/copy';
import { crewActionCopy } from '../state/copy';
import { composerCopy } from './copy';
import {
  messageTooLong,
  MESSAGE_MAX_BYTES,
  sendFailure,
  type SendFailureContext,
} from './sendFailure';

/**
 * QA M1, M5, R-2 and FILES-F9: the composer printed the broker's own `code: text` after
 * "Couldn't send." for every refused post. The texts below are the broker's and the daemon's own
 * (`broker.rs`, `routes/crew.rs`, `crew/transport.rs`), so a change there shows up here.
 */

const context: SendFailureContext = {
  host: 'Iris Wong (@crew_iris)',
  isHost: false,
  fileName: null,
  fileChannel: null,
};

/** A broker refusal as the daemon forwards it: 400 `crew_request_refused`, its code beside. */
function brokerRefusal(text: string): CrewHttpError {
  const code = text.slice(0, text.indexOf(':'));
  return new CrewHttpError(text, 400, 'crew_request_refused', undefined, code);
}

describe('a refused post in words for a person', () => {
  it.each([
    ['the message size limit', 'invalid_params: message too long', composerCopy.tooLong],
    [
      'a full workspace',
      'quota_exceeded: workspace logical state exceeds 16 MiB; reads remain available but further mutations require a new workspace or a supported retention upgrade; in-place pruning is not supported',
      refusalCopy.storageFull,
    ],
    ['the message count limit', 'quota_exceeded: message limit', refusalCopy.storageFull],
    [
      'a member’s share, which the broker words for a person',
      "quota_exceeded: You have used your share of this workspace's storage. Reading still works; ask the workspace host about starting a new workspace.",
      "You have used your share of this workspace's storage. Reading still works; ask the workspace host about starting a new workspace.",
    ],
    [
      'a storage failure, addressed to the host',
      'storage_failed: restart and recover before further mutations',
      composerCopy.storageFailed('Iris Wong (@crew_iris)'),
    ],
    [
      'a file shared in another channel',
      'forbidden: attachment provenance cannot be dropped',
      composerCopy.fileElsewhere(null, null),
    ],
    [
      'a server path shared in another channel',
      'forbidden: reference provenance cannot be dropped',
      composerCopy.pathElsewhere,
    ],
    ['an archived channel', 'channel_archived: channel is read-only', composerCopy.archivedRefusal],
  ])('says %s plainly', (_label, text, words) => {
    const failure = sendFailure(brokerRefusal(text), context);
    expect(failure.text).toBe(words);
    expect(failure.text).not.toMatch(/^[a-z_]+: /);
    expect(failure.transport).toBe(false);
  });

  it('tells the host to restart the server, rather than to ask themselves', () => {
    const failure = sendFailure(
      brokerRefusal('storage_failed: restart and recover before further mutations'),
      { ...context, isHost: true }
    );
    expect(failure.text).toBe(composerCopy.storageFailedHost);
  });

  /**
   * MSG2-N6: the host of a full workspace was told to "ask the host", and the host of a server out
   * of disk space to restart Crew, which stalls again without space.
   */
  it.each([
    [
      'a full workspace',
      'quota_exceeded: workspace logical state is full; reads remain available and the host can still remove members and change policy, but further changes require a new workspace; in-place pruning of history is not supported',
      refusalCopy.fullButHostCanAdministerHost,
    ],
    [
      'a workspace past its supported size',
      'quota_exceeded: workspace logical state exceeds 16 MiB; reads remain available but further mutations require a new workspace or a supported retention upgrade; in-place pruning is not supported',
      refusalCopy.storageFullHost,
    ],
    ['the message count limit', 'quota_exceeded: message limit', refusalCopy.storageFullHost],
    [
      'a server out of disk space',
      'storage_full: The workspace server is out of disk space, so this change was not saved. Reading still works. Ask the host to free space on the server and restart Crew.',
      composerCopy.diskFullHost,
    ],
  ])('tells the host what the host can do about %s', (_label, text, words) => {
    expect(sendFailure(brokerRefusal(text), { ...context, isHost: true }).text).toBe(words);
    expect(words).not.toMatch(/ask the host/i);
    // A member is still told to ask the host.
    expect(sendFailure(brokerRefusal(text), context).text).not.toBe(words);
  });

  it('names the host generically when the viewer cannot see who that is', () => {
    const failure = sendFailure(
      brokerRefusal('storage_failed: restart and recover before further mutations'),
      { ...context, host: null }
    );
    expect(failure.text).toBe(composerCopy.storageFailed(null));
    expect(failure.text).toContain('the workspace host');
  });

  it('says a full disk the same way as a storage failure (QA R-2)', () => {
    const failure = sendFailure(
      new CrewHttpError(
        'request_denied: No space left on device (os error 28)',
        400,
        'crew_request_refused',
        undefined,
        'request_denied'
      ),
      context
    );
    expect(failure.text).toBe(composerCopy.storageFailed('Iris Wong (@crew_iris)'));
  });

  it('names the file and the channel it was shared in when this computer knows them', () => {
    const failure = sendFailure(
      brokerRefusal('forbidden: attachment provenance cannot be dropped'),
      {
        ...context,
        fileName: 'counts.csv',
        fileChannel: '#methods',
      }
    );
    expect(failure.text).toBe(
      'counts.csv was shared in #methods. Share it there, or upload it again here.'
    );
  });

  it('keeps a code it does not know as sent, never guessed at', () => {
    expect(sendFailure(brokerRefusal('forbidden: channel unavailable'), context).text).toBe(
      'forbidden: channel unavailable'
    );
    expect(sendFailure(new CrewHttpError('Slow down.', 429), context).text).toBe('Slow down.');
  });

  it('marks a failure of the link, and words it', () => {
    const notSent = sendFailure(
      new CrewHttpError('The bridge was lost.', 503, 'crew_not_sent'),
      context
    );
    expect(notSent).toMatchObject({
      text: composerCopy.notSent,
      transport: true,
      code: 'crew_not_sent',
    });
    const legacy = sendFailure(
      new CrewHttpError(
        'Crew connection is disconnected; authenticate and connect in Crew',
        400,
        'crew_request_refused'
      ),
      context
    );
    expect(legacy).toMatchObject({ text: composerCopy.notReached, transport: true });
    const timeout = sendFailure(
      new CrewHttpError('The computer did not answer in time', 504),
      context
    );
    expect(timeout).toMatchObject({ text: 'The computer did not answer in time', transport: true });
  });

  it('falls back for anything that is not an error', () => {
    expect(sendFailure('nope', context)).toEqual({
      text: crewActionCopy.actionFallback,
      transport: false,
    });
  });
});

describe('messageTooLong', () => {
  it('counts UTF-8 bytes, as the broker does, not characters', () => {
    expect(messageTooLong('x'.repeat(MESSAGE_MAX_BYTES))).toBe(false);
    expect(messageTooLong('x'.repeat(MESSAGE_MAX_BYTES + 1))).toBe(true);
    // 22,000 characters of Chinese are 66,000 bytes.
    expect(messageTooLong('字'.repeat(22_000))).toBe(true);
  });

  it('counts the escaped size too: control characters grow sixfold', () => {
    const controls = String.fromCharCode(1).repeat(30_000);
    expect(new TextEncoder().encode(controls).length).toBeLessThan(MESSAGE_MAX_BYTES);
    expect(messageTooLong(controls)).toBe(true);
  });
});
