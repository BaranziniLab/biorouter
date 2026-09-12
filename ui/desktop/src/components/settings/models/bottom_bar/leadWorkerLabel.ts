/**
 * What the composer's model chip may say about a lead/worker pair.
 *
 * # The defect (D7 of the 2026-09-12 model-controls run)
 *
 * The chip's one job is to say where the next message goes. With a pair
 * configured it named the WORKER and labelled it `(worker)` — on Home, where the
 * next message starts a new chat and the **lead** answers it.
 *
 * Measured live (dev GUI, sandboxed config, 2026-09-12): lead
 * `claude_code / claude-opus-5`, worker `versa_azure / gpt-4.1-mini-2025-04-14`,
 * `BIOROUTER_LEAD_TURNS: 3`. Home's chip read
 * `Current model: gpt-4.1-mini-2025-04-14 (worker) (Public model)` — the model
 * that does not get the first three turns, under the one word that says so.
 *
 * # Why the role was always `worker`
 *
 * `BIOROUTER_MODEL` **is** the worker while a pair is on (`LeadWorkerSettings`
 * writes the worker to it; `providers::factory::create_lead_worker_from_env`
 * reads it as the worker's model). The role was derived by comparing that key
 * against `BIOROUTER_LEAD_MODEL`, so it could only ever read `lead` for a
 * degenerate pair whose two halves name the same model. Every real pair read
 * `worker`, in every chat and on Home.
 *
 * ⚠ The `useCurrentModelInfo()` branch that looked like it rescued this is dead:
 * `CurrentModelContext` in `BaseChat.tsx` is created and read and **never
 * provided**, so the hook returns `null` on every surface. Do not reason from it.
 *
 * # What the renderer can actually know
 *
 * `LeadWorkerProvider::get_active_provider` routes to the lead while
 * `turn_count < lead_turns`, and `turn_count` is daemon state the renderer is
 * never served. So there are exactly two answers:
 *
 * - **No chat yet** (Home, a chat not started). `/agent/start` opens a session at
 *   `LeadWorkerRoutingState::default()` — `turn_count: 0` — so with `lead_turns`
 *   at 1 or more the lead answers the next message, with certainty. The chip
 *   names the lead and labels it `(lead)`.
 * - **Inside a chat.** Which half is live depends on that chat's turn count, and
 *   nothing tells this component what it is. So the chip claims **no role at
 *   all** rather than asserting the one that is false for every chat's opening
 *   turns. It still names what `BIOROUTER_MODEL` holds, which is what it named
 *   before.
 *
 * ⚠ **`lead_turns` of 0 is a real configuration, and it inverts the first
 * answer.** `turn_count < 0` never holds, so the WORKER takes turn 1 and the lead
 * is only ever reached through `handle_completion_result`'s failure fallback. It
 * is not merely hand-editable: `configure.rs`'s `prompt_turns` validator accepts
 * any `u32::parse`, and `"0"` parses — so the shipped CLI will write it while
 * telling the user it wants a positive number. Claiming `(lead)` there would be
 * D7 again, with the halves swapped.
 *
 * Dropping a claim is the point, not a loss: the pair's whole truth goes to
 * {@link leadWorkerHandoverNote}, in the dropdown, which has room for a sentence
 * — the same split this chip already uses for the tier word, the affiliation
 * word and `CHAT_KEEPS_ITS_MODEL_NOTE`.
 *
 * A chat with its OWN binding is not this module's business: it runs the single
 * provider its session row names, so no half of a globally configured pair is in
 * play and `ModelsBottomBar` never asks.
 */

/**
 * `DEFAULT_LEAD_TURNS` in `crates/biorouter/src/providers/factory.rs`. A copy,
 * because the renderer reads the config key itself and the daemon's fallback is
 * not served anywhere; {@link readLeadTurns} is the one place it is applied.
 */
export const DEFAULT_LEAD_TURNS = 3;

/** The app-wide lead/worker selection, as its config keys state it. */
export interface LeadWorkerPair {
  /** `BIOROUTER_LEAD_MODEL`. Empty when no pair is configured. */
  leadModel: string;
  /**
   * `BIOROUTER_LEAD_PROVIDER`, falling back to `BIOROUTER_PROVIDER` — which is
   * exactly what `create_lead_worker_from_env` does with an unset key. Resolve
   * the fallback before building this, so nothing downstream has to.
   */
  leadProvider: string;
  /** `BIOROUTER_MODEL` — the WORKER's model while a pair is on. */
  workerModel: string;
  /** `BIOROUTER_LEAD_TURNS`, already defaulted by {@link readLeadTurns}. */
  leadTurns: number;
}

/**
 * `BIOROUTER_LEAD_TURNS` as the count the daemon will use.
 *
 * ⚠ **This mirrors `get_param::<usize>(…).unwrap_or(DEFAULT_LEAD_TURNS)` and
 * nothing else.** `/config/read` answers an unset key with `null`, a saved number
 * as a number, and a config file can hold anything; what the daemon's `usize`
 * parse REJECTS falls back to its default, and what it accepts is kept as-is.
 * `0` is accepted there, so it is kept here — see the header. Treating it as
 * "unusable, so default to 3" was the first draft, and it would have put `(lead)`
 * on a pair whose lead does not answer the first turn.
 */
export function readLeadTurns(raw: unknown): number {
  if (raw === null || raw === undefined || raw === '') return DEFAULT_LEAD_TURNS;
  const parsed = typeof raw === 'number' ? raw : Number(raw);
  // A `usize` parse takes neither a fraction nor a negative nor a non-number.
  if (!Number.isInteger(parsed) || parsed < 0) return DEFAULT_LEAD_TURNS;
  return parsed;
}

/** Is a lead/worker pair configured at all? */
export function leadWorkerActive(pair: LeadWorkerPair): boolean {
  return !!pair.leadModel;
}

/** What the chip says about the pair. */
export interface LeadWorkerChip {
  /**
   * The model the chip must name, or `undefined` to keep naming the app-wide
   * selection (which is the worker, and is what the chip named before).
   */
  model?: string;
  /**
   * The provider that model belongs to, and it is set exactly when `model` is.
   *
   * ⚠ **The two travel together or the chip lies about a third thing.** Every
   * other fact the chip states — the tier padlock, the affiliation glyph, the
   * disclosure line — is read off ONE catalog row for the provider named here. A
   * `model` without its provider would hang the worker's provider's tier on the
   * lead's name, which is the cross-provider pairing `ModelsBottomBar`'s own
   * comments warn about at length.
   */
  provider?: string;
  /**
   * The role parenthetical, or `undefined` when no role may be claimed — either
   * because no pair is configured, or because the chip is inside a chat and the
   * live half is not knowable here.
   */
  role?: 'lead';
}

/**
 * The chip's name, provider and role for a pair.
 *
 * `hasChat` is the whole of the distinction: with no chat the next message opens
 * a new session and the lead answers it; inside a chat the turn count decides
 * and the renderer is not told it.
 */
export function leadWorkerChip(pair: LeadWorkerPair, hasChat: boolean): LeadWorkerChip {
  if (!leadWorkerActive(pair)) return {};
  if (hasChat) return {};
  // `lead_turns: 0` means the worker takes turn 1 and the lead is fallback-only,
  // so the app-wide selection is already the right name and no role is claimable.
  if (pair.leadTurns < 1) return {};
  return { model: pair.leadModel, provider: pair.leadProvider, role: 'lead' };
}

/**
 * The dropdown's one line about the pair, or `null` when none is configured.
 *
 * True of every chat and every surface, which is why it can stand where the
 * chip's role cannot: it states the handover rather than claiming a side of it.
 * The worker is omitted when the key is unset — a sentence that names only what
 * it knows, rather than one with a hole in it.
 */
export function leadWorkerHandoverNote(pair: LeadWorkerPair): string | null {
  if (!leadWorkerActive(pair)) return null;
  // At 0 lead turns there is no handover to describe: the worker answers
  // everything and the lead is reached only by the failure fallback.
  if (pair.leadTurns < 1) {
    const worker = pair.workerModel ? `${pair.workerModel} answers every turn; ` : '';
    return `Lead/worker mode. ${worker}${pair.leadModel} is the fallback model.`;
  }
  const turns = pair.leadTurns === 1 ? 'the first turn' : `the first ${pair.leadTurns} turns`;
  const opening = `Lead/worker mode. ${pair.leadModel} answers ${turns} of a chat`;
  return pair.workerModel ? `${opening}; ${pair.workerModel} takes the rest.` : `${opening}.`;
}
