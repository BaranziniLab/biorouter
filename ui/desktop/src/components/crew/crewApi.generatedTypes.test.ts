import { describe, expect, expectTypeOf, it } from 'vitest';
import type * as Api from '../../api/types.gen';
import {
  crewHttp,
  type CrewConnection,
  type CrewErrorCode,
  type CrewPath,
  type ObservedRun,
} from './crewApi';
import type { CrewGrantKind, CrewGrantRevocation, CrewSessionGrant } from './api/grants';
import type { CrewInvitationMissing, CrewJoinState } from './api/join';
import type { CrewSelectorKind } from './api/names';
import type { CrewTransfer, TransferDirection } from './crewTransfers';
import type { HostStartState } from './onboarding/hostStart';
import type { Wire } from './api/parse';
import * as errors from './api/errors';

/**
 * CROSSCUT-6: the renderer reads the daemon's Crew routes through the generated client's types, so
 * `npm run generate-api` (which CI runs and diffs) turns a daemon-side rename into a compile error
 * here rather than a field that silently reads as absent. These are type assertions: `npm run
 * typecheck` fails on a broken one, and a route, field or code that stops existing stops compiling.
 */
describe('Crew answers on the generated types', () => {
  it('sends a path only when it fills a route the daemon declares for that method', () => {
    expectTypeOf<'/connections'>().toMatchTypeOf<CrewPath<'GET'>>();
    expectTypeOf<'/connections/c-1/runs/r-1/cancel'>().toMatchTypeOf<CrewPath<'POST'>>();
    expectTypeOf<'/connections/c-1/sessions/s-1/revoke'>().toMatchTypeOf<CrewPath<'POST'>>();
    expectTypeOf<'/transfers?connection_id=c-1'>().toMatchTypeOf<CrewPath<'GET'>>();
    expectTypeOf<'/connections/c-1/invitation?invitee=bob'>().toMatchTypeOf<CrewPath<'GET'>>();
    // A route the daemon does not have, and one called with another method, do not compile.
    expectTypeOf<'/connections/c-1/dial'>().not.toMatchTypeOf<CrewPath<'POST'>>();
    expectTypeOf<'/connections/c-1/grant'>().not.toMatchTypeOf<CrewPath<'POST'>>();
    expectTypeOf<'/connections/c-1/connect'>().not.toMatchTypeOf<CrewPath<'GET'>>();
    expectTypeOf<'/credentials/lock'>().not.toMatchTypeOf<CrewPath<'GET'>>();
    expectTypeOf<'/credentials/lock'>().not.toMatchTypeOf<CrewPath<'DELETE'>>();
    // The limit of a type: a filled segment is any text, `/` included, so a route that ends in
    // one (`DELETE /connections/{id}`) takes any longer path under it. Encoding every segment is
    // what keeps a value to one segment (`crewHttpPaths.sourceGuard.test.ts`).
    expectTypeOf<'/connections/c-1/extra'>().toMatchTypeOf<CrewPath<'DELETE'>>();
  });

  it('refuses a call whose path and method name no route', () => {
    // Compiled, never run: each `@ts-expect-error` fails the typecheck if its call compiles.
    const calls = () => {
      // @ts-expect-error: the daemon has no such route
      void crewHttp('/connections/c-1/dial', 'POST', {});
      // @ts-expect-error: connect is a POST
      void crewHttp('/connections/c-1/connect');
      void crewHttp('/connections/c-1/connect', 'POST', {});
      void crewHttp(
        `/connections/${encodeURIComponent('c 1')}/runs/${encodeURIComponent('r-1')}/cancel`,
        'POST',
        {}
      );
      void crewHttp<{ runs: ObservedRun[] }>(`/connections/${encodeURIComponent('c-1')}/runs`);
    };
    expect(typeof calls).toBe('function');
  });

  it('reads a saved connection, a task and a grant as the daemon declares them', () => {
    expectTypeOf<CrewConnection['last_error_code']>().toEqualTypeOf<
      Api.CrewConnectionView['last_error_code']
    >();
    expectTypeOf<CrewConnection['port']>().toEqualTypeOf<Api.Connection['port']>();
    expectTypeOf<CrewConnection['mode']>().toEqualTypeOf<Api.ClusterMode>();
    expectTypeOf<ObservedRun['started_at']>().toEqualTypeOf<Api.RunView['started_at']>();
    expectTypeOf<CrewSessionGrant['policy_epoch']>().toEqualTypeOf<Api.GrantRow['policy_epoch']>();
    expectTypeOf<CrewGrantRevocation>().toEqualTypeOf<Api.Revocation>();
    expectTypeOf<CrewGrantKind>().toEqualTypeOf<'chat' | 'task'>();
    expectTypeOf<CrewJoinState>().toEqualTypeOf<Api.JoinState>();
    expectTypeOf<CrewInvitationMissing>().toEqualTypeOf<Api.InvitationMissing>();
    expectTypeOf<CrewSelectorKind>().toEqualTypeOf<
      'person' | 'former_person' | 'team' | 'channel' | 'connection'
    >();
    expectTypeOf<CrewTransfer['blob_id']>().toEqualTypeOf<string | null>();
    expectTypeOf<CrewTransfer['destination_identity']>().toEqualTypeOf<
      Api.Receipt['destination_identity']
    >();
    expectTypeOf<TransferDirection>().toEqualTypeOf<Api.Direction>();
    expectTypeOf<HostStartState>().toEqualTypeOf<Api.HostStartState>();
  });

  it('names only the fields a daemon answer declares', () => {
    expectTypeOf<keyof Wire<Api.CrewError>>().toEqualTypeOf<keyof Api.CrewError>();
    // A field the daemon renamed or dropped is not a key a reader can name.
    expectTypeOf<'brokerCode'>().not.toMatchTypeOf<keyof Wire<Api.CrewError>>();
    // Every member of a tagged answer contributes its fields.
    expectTypeOf<'candidates'>().toMatchTypeOf<
      keyof Wire<Api.ResolveResponse['results'][number]>
    >();
  });

  it('keeps every daemon code the renderer branches on among the codes the daemon declares', () => {
    const daemonCodes = Object.entries(errors)
      .filter(([name, value]) => /^CREW_[A-Z_]+$/.test(name) && typeof value === 'string')
      .map(([, value]) => value as string)
      .filter(
        (code) => code !== errors.CREW_UNEXPECTED_RESPONSE && code !== errors.CREW_DAEMON_OUTDATED
      );
    expect(daemonCodes.length).toBeGreaterThan(15);
    expectTypeOf<typeof errors.CREW_NOT_SENT>().toMatchTypeOf<CrewErrorCode>();
    expectTypeOf<typeof errors.CREW_REVOCATION_UNCONFIRMED>().toMatchTypeOf<CrewErrorCode>();
    // A code the daemon does not answer is not one.
    expectTypeOf<'crew_not_a_code'>().not.toMatchTypeOf<CrewErrorCode>();
    for (const code of daemonCodes) expect(code).toMatch(/^crew_[a-z_]+$/);
  });
});
