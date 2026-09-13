// Install a skill from a registry download URL.
//
// ⚠ **Through the one import pipeline** (`/skills/packages/install`), not
// through a renderer-side unzip-and-write. This function used to fetch the
// asset, extract it with the daemon's depth-counting ZIP parser, and write the
// text files itself — so a marketplace asset that happened to be a
// multi-skill package got the same flattening a pasted repository URL did
// (#115), and a partial write left a half-installed skill with no way to tell.
//
// The asset is downloaded to a temporary file first, exactly as before, because
// the importer takes a path or a URL and the registry's asset URLs are already
// on the allowed-host list either way — but the *interpretation* of what comes
// back is now the daemon's single one.

import { installSkillPackage } from '../../api';
import type { ImportKind, ImportResult } from '../../api';
import type { RegistrySkill } from './registry';

/** One unit the daemon installed. */
export interface InstalledUnit {
  /** The name the Skills list shows it under. */
  name: string;
  kind: ImportKind;
  /** Its component skill names, as installed. */
  skills: string[];
}

export interface InstallResult {
  ok: boolean;
  name: string;
  error?: string;
  /**
   * What landed, as the daemon reports it — set when `ok`. A marketplace
   * package is one unit holding several skills, which is the count the success
   * toast needs and the registry row cannot be trusted to give.
   */
  installed?: InstalledUnit[];
  /** Set when the source was ambiguous and nobody has answered yet. */
  needsChoice?: { planId: string; reason: string; components: string[] };
}

export async function installRegistrySkill(skill: RegistrySkill): Promise<InstallResult> {
  const dl = await window.electron.downloadRegistryAsset(skill.download);
  if ('error' in dl) return { ok: false, name: skill.name, error: dl.error };

  try {
    const response = await installSkillPackage<true>({
      body: { filePath: dl.path },
      throwOnError: true,
    });
    const result = response.data as ImportResult;
    if (result.status === 'needsChoice') {
      // A catalogued marketplace asset should never reach here — its layout is
      // known. Reported rather than resolved, because picking one on the user's
      // behalf is the behaviour this replaced.
      return {
        ok: false,
        name: skill.name,
        needsChoice: {
          planId: result.planId,
          reason: result.preview.ambiguity?.reason ?? 'This package needs a choice.',
          components: result.preview.components.map((component) => component.name),
        },
        error: result.preview.ambiguity?.reason,
      };
    }
    return {
      ok: true,
      name: skill.name,
      installed: (result.installed ?? []).map((unit) => ({
        name: unit.displayName,
        kind: unit.kind,
        skills: unit.skills ?? [],
      })),
    };
  } catch (err) {
    return {
      ok: false,
      name: skill.name,
      error: err instanceof Error ? err.message : `Could not install ${skill.name}`,
    };
  }
}
