import { useEffect, useState } from 'react';
import { readSetupArtifact } from '../onboarding/setup';

/**
 * The version of the Crew server program this build carries, as `GET /crew/setup/artifact`
 * describes it (the daemon refuses a program whose version is not its own), for the connection
 * bar's version notice (account ownership AO-4, "Desktop"). It is the program Update Crew
 * installs, so the notice and the Open dialog's Update Crew agree on what "older" means.
 *
 * Read at most once per window once it is known: the daemon reads and hashes the whole program to
 * answer, and the program does not change while it runs. A read that fails (a build that carries
 * no program, an older daemon, a refusal) answers `null` and is not kept, so the next notice that
 * needs it asks again.
 */
let known: string | null = null;
let pending: Promise<string | null> | null = null;

export function readBuildBrokerVersion(): Promise<string | null> {
  if (known !== null) return Promise.resolve(known);
  pending ??= readSetupArtifact().then(
    (artifact) => {
      known = artifact.version;
      pending = null;
      return known;
    },
    () => {
      pending = null;
      return null;
    }
  );
  return pending;
}

/** {@link readBuildBrokerVersion} for a component: asked only while `active`; `null` until known. */
export function useBuildBrokerVersion(active: boolean): string | null {
  const [version, setVersion] = useState<string | null>(known);
  useEffect(() => {
    if (!active || version !== null) return;
    let live = true;
    void readBuildBrokerVersion().then((read) => {
      if (live && read !== null) setVersion(read);
    });
    return () => {
      live = false;
    };
  }, [active, version]);
  return version;
}

export function resetBuildBrokerVersionForTests(): void {
  known = null;
  pending = null;
}
