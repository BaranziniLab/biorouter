import { nameToKey } from '../extensions/utils';
import type { ExtensionConfig } from '../../../api/types.gen';

/**
 * Built-in capabilities of Biorouter.
 *
 * These ship with Biorouter and are surfaced as first-class "Capabilities" in
 * Settings → Chat instead of being mixed with user-installed extensions.
 *
 * Keys are the normalized (whitespace-stripped, lower-cased) extension names,
 * matching `nameToKey(extension.name)`.
 */
export interface CapabilityMeta {
  /** Normalized extension key — see nameToKey(). */
  key: string;
  label: string;
  description: string;
  defaultEnabled: boolean;
}

export const CAPABILITIES: CapabilityMeta[] = [
  {
    key: 'developer',
    label: 'Developer',
    description: 'Read, write and run code, and run shell commands.',
    defaultEnabled: true,
  },
  {
    key: 'computercontroller',
    label: 'Computer Use',
    description: 'View and control desktop apps for an approved task.',
    defaultEnabled: true,
  },
  {
    key: 'webdocuments',
    label: 'Web & Documents',
    description: 'Read web pages and work with spreadsheets, documents, and PDFs.',
    defaultEnabled: true,
  },
  {
    key: 'autovisualiser',
    label: 'Auto Visualiser',
    description:
      'Interactive charts, diagrams, networks, maps and scientific plots, rendered inline.',
    defaultEnabled: true,
  },
  {
    key: 'code_execution',
    label: 'Code Execution',
    description: 'Execute JavaScript in a sandboxed environment for programmatic tool use.',
    defaultEnabled: true,
  },
  {
    key: 'extensionmanager',
    label: 'Extension Manager',
    description:
      'Lets Biorouter find and install extensions from the marketplace, attach and detach them on its own, and permanently delete an installed package once you approve it.',
    defaultEnabled: true,
  },
  {
    key: 'skills',
    label: 'Skills',
    description: 'Load and use reusable skills from your skill directories.',
    defaultEnabled: true,
  },
  {
    key: 'todo',
    label: 'Todo',
    description: 'Keep track of multi-step tasks with a working to-do list.',
    defaultEnabled: true,
  },
  {
    key: 'memory',
    label: 'Memory',
    description: 'Teach Biorouter your preferences so it remembers them as you go.',
    defaultEnabled: true,
  },
  {
    key: 'knowledge',
    label: 'Knowledge',
    description:
      'Personal knowledge bases Biorouter builds and maintains for you, stored as markdown with full history.',
    defaultEnabled: true,
  },
  {
    key: 'agent_drafter',
    label: 'Agent Drafter',
    description:
      'Build interactive artifacts, static pages or apps with an embedded Biorouter agent, and export them as standalone projects.',
    defaultEnabled: true,
  },
  {
    key: 'chatrecall',
    label: 'Chat Recall',
    description: 'Search past chats and load their summaries for context.',
    defaultEnabled: false,
  },
  {
    /*
     * ⚠ Workspace is a capability, not an extension (#76).
     *
     * This one entry is the whole UI half of that change. The Extensions
     * tab and the composer popup both filter on `isCapabilityExtension`,
     * and the chip count, the bulk Enable/Disable counts and the load
     * toast's denominator all derive from the same predicate, so adding
     * the key here removes it from every one of them at once.
     *
     * Lowercase `workspace`: the daemon sends `name: "Workspace"` and the
     * match runs through `nameToKey`.
     */
    key: 'workspace',
    label: 'Workspace Control',
    description:
      'Work across several chats: open, read and steer other chats, and run subagents in visible tabs.',
    defaultEnabled: true,
  },
];

/** Set of normalized capability keys, for quick membership checks. */
export const CAPABILITY_KEYS: ReadonlySet<string> = new Set(CAPABILITIES.map((c) => c.key));

const CAPABILITIES_BY_KEY: ReadonlyMap<string, CapabilityMeta> = new Map(
  CAPABILITIES.map((capability) => [capability.key, capability])
);

/** True when the given extension is one of the shipped capabilities. */
export function isCapabilityExtension(extension: { name: string } | ExtensionConfig): boolean {
  return CAPABILITY_KEYS.has(nameToKey(extension.name));
}

export function isCapabilityDefaultEnabled(extension: { name: string } | ExtensionConfig): boolean {
  return CAPABILITIES_BY_KEY.get(nameToKey(extension.name))?.defaultEnabled ?? false;
}
