import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * The producer half of the approval card's one rule, asserted from the consumer's
 * side because this is where the consequence shows up.
 *
 * `ToolCallConfirmation` reads a card's `prompt` as a SECURITY FINDING: it paints
 * a warning banner and withholds "Always Allow", on the grounds that a permanent
 * grant is not a thing to decide from a card that exists because an inspector
 * objected. That is only sound while `prompt` carries nothing but
 * `approval_prompt_for_request` — the inspectors' own reasons.
 *
 * The coding-agent bridge broke it: every bridged call parked a card whose prompt
 * was the bridge's own framing ("<child> asked to run this through Biorouter"), so
 * every one of them arrived under a security banner, with no way to grant a
 * lasting permission, for a verdict no inspector had reached.
 *
 * A TypeScript test reaching into Rust is deliberate. The rule spans the two
 * languages, the renderer half is pinned next door in
 * `ToolCallConfirmation.test.tsx`, and a rule that is only checked on one side of
 * a boundary is how this drifted in the first place. (`autovis_cdn_desktop_contract.rs`
 * does the same thing in the other direction.)
 */
const here = dirname(fileURLToPath(import.meta.url));
const BRIDGE = resolve(here, '../../../..', 'crates/biorouter/src/providers/coding_agent/bridge.rs');

const bridge = readFileSync(BRIDGE, 'utf-8');

/** The body of one `fn` in the bridge, up to the next item at the same depth. */
function functionBody(name: string): string {
  const start = bridge.indexOf(`async fn ${name}(`);
  expect(start, `${name} should exist in bridge.rs`).toBeGreaterThan(-1);
  const rest = bridge.slice(start + 1);
  const end = rest.search(/\n {4}(?:pub )?(?:async )?fn |\n {4}\/\/\/ /);
  return end === -1 ? rest : rest.slice(0, end);
}

describe('the bridge and the approval card agree on what `prompt` means', () => {
  const awaitApproval = functionBody('await_approval');

  it('parks its card without inventing a security finding', () => {
    expect(awaitApproval).toContain('prompt: None');
    expect(awaitApproval).not.toMatch(/prompt:\s*Some\(/);
  });

  it('does not dress its own framing up as an inspector verdict', () => {
    // The attribution is worth showing the user — it just needs a field of its
    // own rather than the one that means "an inspector objected".
    expect(bridge).not.toContain('asked to run this through Biorouter');
  });

  it('records an approval the user meant to last, so the card is not lying', () => {
    // Offering "Always Allow" is only honest if the answer survives the turn.
    // `handle_approved_and_denied_tools` does this on the agent's own path; the
    // bridge only logged the permission, so the next identical call asked again.
    const calls = awaitApproval.match(/record_lasting_decision/g) ?? [];
    expect(calls.length, 'both the approved and the denied arm must record').toBe(2);

    const record = functionBody('record_lasting_decision');
    expect(record).toContain('Permission::AlwaysAllow => PermissionLevel::AlwaysAllow');
    expect(record).toContain('Permission::AlwaysDeny => PermissionLevel::NeverAllow');
    expect(record).toContain('update_permission_manager');
    // A one-off is an answer about this call, not a rule.
    expect(record).toMatch(/_ => return/);
  });
});
