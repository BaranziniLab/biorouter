/**
 * The attributes of a text field that holds a MACHINE string: a command (`claude`), a URL, a model
 * id (`gpt-5.5-2026-04-24`), a region (`us-west-2`), a profile name, a config value. Vision
 * V4-R7-5.
 *
 * The browser's spellchecker knows nothing about those, so on a field left as it ships Chromium
 * drew a red dotted misspelling line under `claude` the moment Configure Claude Code opened with
 * its command field focused, and under every value in the Configuration editor (`versa_azure`,
 * `auto`, the Bedrock URL). A squiggle on a correct value tells a person something is wrong with
 * it. The same trio keeps the platform from "correcting" the value (`autoCorrect`) or turning the
 * first letter of a command into a capital (`autoCapitalize`) where a platform does that.
 *
 * Spread it on the field: `<Input {...machineStringProps} />`. A secret field needs nothing
 * extra, `SecretInput` already switches spellcheck off whatever a caller passes; this is for the
 * plain ones beside it. Do not put it on a field that holds a person's own words (a chat
 * message, a description): that one should keep its spellcheck, which the Chat tab's switch
 * controls.
 *
 * `machineString.test.ts` fails on any `<Input` in this tree and in the provider dialogs that does
 * not spread it, so a field added later does not bring the underline back.
 */
export const machineStringProps = {
  spellCheck: false,
  autoCorrect: 'off',
  autoCapitalize: 'off',
} as const;
