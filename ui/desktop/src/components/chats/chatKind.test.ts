import { describe, expect, it } from 'vitest';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { chatIconFor, chatKindOf, chatPrivacyMark, isCrewTask, type ChatKind } from './chatKind';

const ALL_KINDS: ChatKind[] = [
  'chat',
  'crew',
  'branch',
  'subagent',
  'app',
  'scheduled',
  'workflow',
  'terminal',
];
const NON_CHAT_KINDS = ALL_KINDS.filter((k) => k !== 'chat');

describe('chatKindOf', () => {
  it('reads the durable lineage fields, not the title', () => {
    expect(chatKindOf({ name: 'Cohort query', diverged_from: 'session-0' })).toBe('branch');
    expect(chatKindOf({ name: 'Cohort query', parent_session_id: 'session-0' })).toBe('subagent');
    expect(chatKindOf({ name: 'Cohort query', session_type: 'sub_agent' })).toBe('subagent');
    expect(chatKindOf({ name: 'Nightly triage', session_type: 'scheduled' })).toBe('scheduled');
    expect(chatKindOf({ name: 'zsh', session_type: 'terminal' })).toBe('terminal');
    expect(chatKindOf({ name: 'app:spec-002' })).toBe('app');
    expect(chatKindOf({ name: 'Cohort query' })).toBe('chat');
  });

  /**
   * ⚠ **A renamed branch is still a branch.** The title regex was the only
   * signal before BR-45 recorded lineage, and it is defeated by anyone who
   * renames the chat — which is exactly why it stopped being the primary test.
   * It survives only as the fallback for rows written before the field existed.
   */
  it('still recognises a branch whose name was changed', () => {
    expect(chatKindOf({ name: 'Second attempt at the cohort', diverged_from: 'session-0' })).toBe(
      'branch'
    );
    // …and the legacy naming, for rows that predate `diverged_from`.
    expect(chatKindOf({ name: 'Greeting 2 (branch 1)' })).toBe('branch');
  });

  /**
   * ⚠ The regex must not fire on a chat that merely talks about branches. This
   * is the failure the sidebar's original resolver was already guarding, and
   * moving the resolver must not drop the guard.
   */
  it('does not mistake prose for a branch or an app', () => {
    expect(chatKindOf({ name: 'Which git branch 2 use?' })).toBe('chat');
    expect(chatKindOf({ name: 'Refactor the app: rename it' })).toBe('chat');
  });

  /**
   * ⚠ **Order is load-bearing.** A sub-agent that was itself diverged carries
   * BOTH fields; the delegation is the more consequential fact (it is not a
   * chat the user is holding), so it must win. Written down because the two
   * arms are adjacent and swapping them fails nothing else.
   */
  it('prefers delegation over divergence when a session is both', () => {
    expect(chatKindOf({ name: 'Worker', diverged_from: 'a', parent_session_id: 'b' })).toBe(
      'subagent'
    );
  });

  it('treats a missing name as a plain chat rather than throwing', () => {
    expect(chatKindOf({})).toBe('chat');
    expect(chatKindOf({ name: null })).toBe('chat');
  });
});

describe('chatIconFor', () => {
  /**
   * ⚠ **Privacy is a SHAPE difference, not a hue** (the lock badge). The dense
   * dot it replaced was a separate mark; folding the tier into a colour alone
   * would have made a safety-relevant marking invisible to anyone who cannot
   * separate the two inks.
   */
  it('gives a private chat a different glyph from a public one', () => {
    const priv = chatIconFor('chat', 'private');
    const pub = chatIconFor('chat', 'public');
    expect(priv.Icon).not.toBe(pub.Icon);
    expect(priv.label).toBe('Private chat');
    expect(pub.label).toBe('Chat');
  });

  it('says the word "private" for every kind, not just plain chats', () => {
    for (const kind of NON_CHAT_KINDS) {
      expect(chatIconFor(kind, 'private').label).toMatch(/private/i);
      expect(chatIconFor(kind, 'public').label).not.toMatch(/private/i);
    }
  });

  /**
   * An unknown tier reads as neither private NOR public. It keeps the unmarked
   * shape (a padlock is a claim) and says "not yet known" in its name; this
   * test used to assert it read as Public, which pinned the 2026-09-14 defect.
   */
  it('gives an unknown tier the unmarked shape and says it is not yet known', () => {
    expect(chatIconFor('chat', 'unknown').Icon).toBe(chatIconFor('chat', 'public').Icon);
    expect(chatIconFor('chat', 'unknown').Icon).not.toBe(chatIconFor('chat', 'private').Icon);
    expect(chatIconFor('chat', 'unknown').label).toBe('Chat, privacy not yet known');
    for (const kind of NON_CHAT_KINDS) {
      expect(chatIconFor(kind, 'unknown').label).toMatch(/not yet known/);
      expect(chatIconFor(kind, 'unknown').label).not.toMatch(/private/i);
    }
  });

  it('says nothing about privacy when the master switch is off', () => {
    expect(chatIconFor('chat', 'off')).toEqual(chatIconFor('chat', 'public'));
    expect(chatIconFor('subagent', 'off').label).toBe('Sub-agent');
  });

  it('gives every kind a distinct glyph, public and private', () => {
    for (const mark of ['public', 'private'] as const) {
      const icons = ALL_KINDS.map((k) => chatIconFor(k, mark).Icon);
      expect(new Set(icons).size).toBe(icons.length);
    }
  });

  it('gives every kind a glyph, a private glyph and a name', () => {
    for (const kind of ALL_KINDS) {
      const pub = chatIconFor(kind, 'public');
      const priv = chatIconFor(kind, 'private');
      expect(pub.Icon, kind).toBeTruthy();
      expect(priv.Icon, kind).toBeTruthy();
      expect(pub.label.length, kind).toBeGreaterThan(0);
      // Privacy is a shape on EVERY kind now, not only on a plain chat.
      expect(priv.Icon, kind).not.toBe(pub.Icon);
      expect(chatIconFor(kind, 'unknown').Icon, kind).toBe(pub.Icon);
      expect(chatIconFor(kind, 'off').Icon, kind).toBe(pub.Icon);
    }
  });

  it('names the two new kinds', () => {
    expect(chatIconFor('crew', 'public').label).toBe('Crew task');
    expect(chatIconFor('crew', 'private').label).toBe('Crew task, private');
    expect(chatIconFor('workflow', 'unknown').label).toBe('Workflow run, privacy not yet known');
  });

  /** One glyph, one meaning: a kind that is an entity draws the entity glyph. */
  it('draws the entity glyph for a kind that is an entity', () => {
    expect(chatIconFor('chat', 'public').Icon).toBe(ENTITY_ICONS.chat);
    expect(chatIconFor('subagent', 'public').Icon).toBe(ENTITY_ICONS.agent);
    expect(chatIconFor('app', 'public').Icon).toBe(ENTITY_ICONS.application);
    expect(chatIconFor('scheduled', 'public').Icon).toBe(ENTITY_ICONS.schedule);
    expect(chatIconFor('workflow', 'public').Icon).toBe(ENTITY_ICONS.workflow);
  });
});

describe('Crew task chats', () => {
  it('reads the folder every Crew task runs in, so a renamed task stays Crew', () => {
    expect(
      chatKindOf({
        name: 'Statin cohort',
        working_dir: '/Users/a/.local/share/biorouter/crew/tasks',
      })
    ).toBe('crew');
    expect(chatKindOf({ name: 'Statin cohort', working_dir: '/x/crew/tasks/' })).toBe('crew');
    expect(isCrewTask({ working_dir: 'C:\\Users\\a\\AppData\\Biorouter\\crew\\tasks' })).toBe(true);
  });

  it('takes the daemon origin when it is there', () => {
    expect(chatKindOf({ name: 'Anything', origin: 'crew' })).toBe('crew');
  });

  it('falls back to the title the daemon writes', () => {
    expect(chatKindOf({ name: 'Crew · #methods · Summarize the counts' })).toBe('crew');
    expect(chatKindOf({ name: 'Crew · #methods' })).toBe('crew');
    expect(chatKindOf({ name: 'Crew task' })).toBe('crew');
  });

  it('does not mistake a chat about crews for a Crew task', () => {
    expect(chatKindOf({ name: 'Crew planning notes' })).toBe('chat');
    expect(chatKindOf({ name: 'Crew tasks for Monday' })).toBe('chat');
    expect(chatKindOf({ name: 'crew · lowercase' })).toBe('chat');
    expect(chatKindOf({ name: 'Notes', working_dir: '/Users/a/crew/tasks-archive' })).toBe('chat');
  });

  /** A fork of a Crew task is the person's own chat; it no longer posts to the channel. */
  it('lets a fork of a Crew task read as a branch', () => {
    expect(
      chatKindOf({
        name: 'Crew · #methods · Plot (branch 1)',
        working_dir: '/x/crew/tasks',
        diverged_from: 's1',
      })
    ).toBe('branch');
  });

  it('keeps a Crew sub-agent a sub-agent', () => {
    expect(
      chatKindOf({
        name: 'Crew · #methods · x',
        session_type: 'sub_agent',
        working_dir: '/x/crew/tasks',
      })
    ).toBe('subagent');
  });
});

/**
 * The workflow kind ships dormant: `SessionSummary` carries no `origin` until
 * the daemon field lands (owner decision 6.10), so no real row reaches it yet.
 */
describe('workflow runs', () => {
  it('reads the daemon origin', () => {
    expect(chatKindOf({ name: 'Weekly cohort refresh', origin: 'workflow' })).toBe('workflow');
  });

  it('reads as a plain chat without it', () => {
    expect(chatKindOf({ name: 'Weekly cohort refresh' })).toBe('chat');
  });

  it('lets the scheduler own a scheduled workflow run', () => {
    expect(chatKindOf({ name: 'Nightly', origin: 'workflow', session_type: 'scheduled' })).toBe(
      'scheduled'
    );
  });
});

describe('chatPrivacyMark', () => {
  it('never turns an absent tier into public', () => {
    expect(chatPrivacyMark(undefined, true)).toBe('unknown');
    expect(chatPrivacyMark(null, true)).toBe('unknown');
    expect(chatPrivacyMark('public', true)).toBe('public');
    expect(chatPrivacyMark('private', true)).toBe('private');
  });

  it('stands down to off, whatever the tier, when nothing enforces tiers', () => {
    for (const tier of ['private', 'public', null, undefined] as const) {
      expect(chatPrivacyMark(tier, false)).toBe('off');
    }
  });
});
