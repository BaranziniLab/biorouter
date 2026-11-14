import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import bundledExtensions from '../extensions/bundled-extensions.json';
import { CAPABILITIES, isCapabilityDefaultEnabled, isCapabilityExtension } from './capabilities';

const expectedDefaults = {
  developer: true,
  computercontroller: true,
  webdocuments: true,
  autovisualiser: true,
  code_execution: true,
  extensionmanager: true,
  skills: true,
  todo: true,
  memory: true,
  knowledge: true,
  agent_drafter: true,
  chatrecall: false,
  dictionary: false,
  workspace: true,
  crew: true,
  decisions: true,
};

describe('capabilities', () => {
  it('classifies every shipped built-in and platform extension as a capability', () => {
    expect(
      Object.fromEntries(CAPABILITIES.map(({ key, defaultEnabled }) => [key, defaultEnabled]))
    ).toEqual(expectedDefaults);

    for (const extension of bundledExtensions) {
      expect(isCapabilityExtension(extension), extension.name).toBe(true);
      expect(isCapabilityDefaultEnabled(extension), extension.name).toBe(extension.enabled);
    }

    for (const name of [
      'todo',
      'chatrecall',
      'extensionmanager',
      'skills',
      'code_execution',
      'dictionary',
      'crew',
    ]) {
      expect(isCapabilityExtension({ name }), name).toBe(true);
    }
  });

  it('is every platform extension the daemon registers', () => {
    // A platform extension missing here is listed among the person's installed extensions.
    const agents = join(process.cwd(), '..', '..', 'crates', 'biorouter', 'src', 'agents');
    const registry = readFileSync(join(agents, 'extension.rs'), 'utf8');
    const block = registry.slice(registry.indexOf('pub static PLATFORM_EXTENSIONS'));
    const modules = [
      ...block
        .slice(0, block.indexOf('\n);'))
        .matchAll(/name: (?:crate::agents::)?(\w+)::EXTENSION_NAME/g),
    ].map((match) => match[1]);
    expect(modules).toContain('decisions_extension');
    expect(modules.length).toBeGreaterThanOrEqual(8);
    for (const module of modules) {
      const source = readFileSync(join(agents, `${module}.rs`), 'utf8');
      const name = source.match(/pub (?:static|const) EXTENSION_NAME: &str = "([^"]+)";/)?.[1];
      expect(name, module).toBeTruthy();
      expect(isCapabilityExtension({ name: name! }), name).toBe(true);
    }
  });

  it('the extension manager description names installing and deleting', () => {
    // The natural half-fix mentions installation and leaves out deletion — the
    // irreversible half, and the only reason this consent copy matters. Not an
    // equality check: the Rust-side description is a different sentence in a
    // different register, deliberately.
    const description = CAPABILITIES.find((c) => c.key === 'extensionmanager')?.description ?? '';
    expect(description).toMatch(/install/i);
    expect(description).toMatch(/delet/i);
  });
});
