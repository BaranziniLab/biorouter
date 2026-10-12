import type { FC } from 'react';
import type { LucideProps } from 'lucide-react';
// The entity glyphs are imported by name, not through ENTITY_ICONS: its
// `EntityIcon` type is deliberately narrower than `FC<LucideProps>`. That they
// ARE the entity glyphs is asserted in chatKind.test.ts.
import {
  Agent,
  AgentPrivate,
  AppWindow,
  ChatBubble,
  ChatBubblePrivate,
  Clock,
  CrewChat,
  CrewChatPrivate,
  GitBranch,
  Pipeline,
  Terminal,
  withPrivateBadge,
} from '../icons/app-icons';
import type { SessionClassification } from '../../api/types.gen';

/**
 * What KIND of chat a row represents. One axis, decided from the session record.
 *
 * ⚠ **Kind is not privacy.** A chat has both — a delegated sub-agent can be
 * private, a branch can be public — so the two are resolved separately and
 * composed by {@link chatIconFor}. Collapsing them into one enum is how you end
 * up unable to say "private branch".
 */
export type ChatKind =
  | 'chat'
  | 'crew'
  | 'branch'
  | 'subagent'
  | 'app'
  | 'scheduled'
  | 'workflow'
  | 'terminal';

/**
 * The fields any chat-listing surface has. Deliberately structural rather than
 * the generated `Session`: the sidebar's `SessionSummary`, History's rows and
 * the tab strip's tabs are three different shapes that all carry these, and a
 * surface that only knows a name still gets a correct answer for `app` and the
 * legacy branch case.
 */
export interface ChatKindSource {
  name?: string | null;
  /** BR-45: set by `diverge_session`. The durable mark of a user fork. */
  diverged_from?: string | null;
  /** BR-71: the delegating parent, set when this session is a sub-agent. */
  parent_session_id?: string | null;
  /** As stored: `user` / `scheduled` / `sub_agent` / `hidden` / `terminal`. */
  session_type?: string | null;
  /**
   * Where the chat came from, when the daemon says: `crew` for a Crew task
   * (`routes/crew.rs` `create_run_session`), `workflow` for a chat a workflow
   * started (`sessions.workflow_json` is set). `SessionSummary` does not carry
   * it today (owner decision 6.10: no backend change), so `workflow` is
   * dormant and Crew falls back to `working_dir`, then the title.
   */
  origin?: string | null;
  /** The chat's working folder. Every Crew task runs in `<data>/crew/tasks`. */
  working_dir?: string | null;
}

/** `create_run_session` gives every Crew task this folder. */
const CREW_TASK_DIR = /[\\/]crew[\\/]tasks[\\/]?$/;
/** `task_title` (`routes/crew.rs`): "Crew task" until admission names the
 * channel, then "Crew · #channel" or "Crew · #channel · excerpt". */
const CREW_TASK_NAME = /^Crew(?: task$| · )/;

/**
 * Is this a Crew task chat (the owned agent's run for a channel)?
 *
 * Strongest signal first: the daemon's `origin`, then the folder the daemon
 * puts every task in (it survives a rename), then the title, the fallback for
 * surfaces that only know a name (the tab strip). The title test is the
 * weakest, exactly as the branch regex is: a person can rename a task away
 * from it, and can name an ordinary chat into it.
 */
export function isCrewTask(session: ChatKindSource): boolean {
  if (session.origin === 'crew') return true;
  if (session.working_dir && CREW_TASK_DIR.test(session.working_dir)) return true;
  return CREW_TASK_NAME.test((session.name ?? '').trim());
}

/**
 * ⚠ **Order matters, and it is not arbitrary.** A sub-agent that was itself
 * diverged carries both `parent_session_id` and `diverged_from`; the delegation
 * is the more consequential fact about it (it is not a chat the user is holding),
 * so it wins. `app` is checked first because an app's chat is not a chat the
 * user opens at all. A fork of a Crew task reads as a branch: it is the
 * person's own chat and no longer posts to the channel. A scheduled workflow
 * run reads as scheduled.
 */
export function chatKindOf(session: ChatKindSource): ChatKind {
  const name = (session.name ?? '').trim();
  if (name.startsWith('app:')) return 'app';

  const type = session.session_type ?? null;
  if (type === 'terminal') return 'terminal';
  if (type === 'sub_agent' || session.parent_session_id) return 'subagent';
  if (type === 'scheduled') return 'scheduled';

  // `diverged_from` is the durable signal. The name regex behind it is the
  // legacy fallback and nothing more: it was the ONLY signal before BR-45
  // recorded lineage, so rows written then still need to read as branches — but
  // it is also defeated by anyone who renames a branch, which is precisely why
  // it stopped being the primary test.
  if (session.diverged_from) return 'branch';
  if (isCrewTask(session)) return 'crew';
  if (session.origin === 'workflow') return 'workflow';
  if (/\(branch \d+\)$/i.test(name)) return 'branch';

  return 'chat';
}

/**
 * What a chat glyph may SAY about privacy — one more state than the tier has.
 *
 * - `private` / `public` — a source has read this chat's row and said so.
 * - `unknown` — nobody has yet. A tab whose chat the session list leaves out (a
 *   delegated subagent's) until its own row is read; every tab after a reload
 *   until the list lands; a row whose read was refused.
 * - `off` — the master privacy switch is off, so nothing enforces a tier and
 *   the glyph makes no privacy statement either way (DR-15).
 *
 * ⚠ **`unknown` exists because rendering it as `public` was a false statement
 * on a privacy indicator.** Measured 2026-09-14 on 1.90.4: a private chat's
 * subagent tabs drew `data-privacy="public"` for ~0.5–3.2 s after a Settings
 * round-trip and ~1.5 s after a reload, until their `metadata_only` reads
 * answered. The tier maps already kept "no opinion" apart from Public
 * (`privacy/sessionTier.ts`); the glyph then folded the two back together with
 * `tier ?? 'public'`.
 *
 * ⚠ **Why not "private until confirmed" instead.** It is the other lie. Every
 * public chat would flash a padlock after every reload, and a mark that fires on
 * chats that are not private is a mark people learn to look past — the reason
 * the tier badge keeps Public quiet. It would also make this glyph the one place
 * in the renderer that INVENTS `private`, which every tier source is careful
 * never to do, and a read that fails for good (a refused or deleted chat) would
 * leave the invented padlock up indefinitely. No gate reads this glyph — the
 * daemon enforces — so over-marking protects nothing; the glyph's only job is
 * to be true, and "not yet known" is.
 */
export type ChatPrivacyMark = SessionClassification | 'unknown' | 'off';

/**
 * Resolve the mark for a glyph from what its surface knows.
 *
 * `null`/`undefined` is "no source has an opinion", never Public. With the
 * switch off the answer is `off` whatever the tier, so a chat the daemon has
 * classified private does not claim a protection nothing is applying.
 */
export function chatPrivacyMark(
  tier: SessionClassification | null | undefined,
  tiersEnabled: boolean
): ChatPrivacyMark {
  if (!tiersEnabled) return 'off';
  return tier ?? 'unknown';
}

/**
 * The words each mark adds to a glyph's accessible name. A `Record` over the
 * union, so a new mark — or a third tier on the wire — is a compile error here
 * rather than a state that silently reads as one of the others.
 */
const PRIVACY_SUFFIX: Record<ChatPrivacyMark, string> = {
  private: ', private',
  public: '',
  unknown: ', privacy not yet known',
  off: '',
};

interface KindGlyph {
  label: string;
  Icon: FC<LucideProps>;
  /** The same figure wearing the lock badge. Built once, at module scope. */
  Private: FC<LucideProps>;
}

/**
 * One row per kind, so a new kind is a compile error until it has a glyph, a
 * private glyph and a name. A kind that is an entity draws that entity's glyph
 * (a scheduled run is the Scheduler's Clock, an app chat is Built apps'
 * AppWindow, a workflow run is the Workflows mark).
 */
const KIND_GLYPHS: Record<ChatKind, KindGlyph> = {
  chat: { label: 'Chat', Icon: ChatBubble, Private: ChatBubblePrivate },
  crew: { label: 'Crew task', Icon: CrewChat, Private: CrewChatPrivate },
  branch: {
    label: 'Diverged chat',
    Icon: GitBranch,
    Private: withPrivateBadge(GitBranch, 'branch'),
  },
  subagent: { label: 'Sub-agent', Icon: Agent, Private: AgentPrivate },
  app: { label: 'App', Icon: AppWindow, Private: withPrivateBadge(AppWindow, 'app') },
  scheduled: {
    label: 'Scheduled run',
    Icon: Clock,
    Private: withPrivateBadge(Clock, 'scheduled'),
  },
  workflow: {
    label: 'Workflow run',
    Icon: Pipeline,
    Private: withPrivateBadge(Pipeline, 'workflow'),
  },
  terminal: {
    label: 'Terminal',
    Icon: Terminal,
    Private: withPrivateBadge(Terminal, 'terminal'),
  },
};

interface ChatIcon {
  Icon: FC<LucideProps>;
  /** Screen-reader text. States the kind AND, when private or unknown, that. */
  label: string;
}

/**
 * The glyph for one chat, from its kind and its privacy mark.
 *
 * ⚠ **Privacy is a SHAPE on every kind**: the lock badge, bottom right. It
 * used to be a shape only on a plain chat (a padlocked bubble), and the other
 * kinds relied on a coral ink plus the label, which made an all-private list a
 * wall of coral glyphs and left a private branch distinguishable from a public
 * one by hue alone. The label still says the word for every kind.
 *
 * `unknown` keeps the unmarked SHAPE (a padlock is a claim, and this state
 * makes none) and says so in the label. What sets it apart from Public on
 * screen is the dimming `main.css` authors against `data-privacy="unknown"`,
 * which `ChatKindIcon` stamps. `off` makes no statement at all.
 */
export function chatIconFor(kind: ChatKind, mark: ChatPrivacyMark): ChatIcon {
  const glyph = KIND_GLYPHS[kind] ?? KIND_GLYPHS.chat;
  if (mark === 'private') {
    return {
      Icon: glyph.Private,
      label: kind === 'chat' ? 'Private chat' : `${glyph.label}${PRIVACY_SUFFIX.private}`,
    };
  }
  return { Icon: glyph.Icon, label: `${glyph.label}${PRIVACY_SUFFIX[mark]}` };
}
