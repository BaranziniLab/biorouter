/**
 * The App SDK safety frameworks: four opt-in switches, each backed by a config key that is false
 * on the server until a person turns it on. Tests import these instead of retyping them.
 *
 * The prose says "Agent Drafter" (no hyphen) and "stop hook" (lower case), the way the rest of the
 * product does. `help` is InfoTip text: two sentences at most.
 */
export type BrsdkKey =
  | 'brsdk_pii_guardrail'
  | 'brsdk_llm_guardrails'
  | 'brsdk_encryption'
  | 'brsdk_tracing';

export interface BrsdkToggleMeta {
  key: BrsdkKey;
  label: string;
  help: string;
}

export const BRSDK_TOGGLES: readonly BrsdkToggleMeta[] = [
  {
    key: 'brsdk_pii_guardrail',
    label: 'PII / PHI guardrail',
    help: 'Mask personal and health information before an app message reaches the model or chat.',
  },
  {
    key: 'brsdk_llm_guardrails',
    label: 'Goal stop-hook guardrail',
    help: 'Run the goal stop hook judge for Agent Drafter apps that declare a goal.',
  },
  {
    key: 'brsdk_encryption',
    label: 'Encrypted vault',
    help: "Store each Agent Drafter app's data in a per-app encrypted vault.",
  },
  {
    key: 'brsdk_tracing',
    label: 'Agent tracing',
    help: 'Permit trace timeline support for Agent Drafter apps that declare tracing.',
  },
];
