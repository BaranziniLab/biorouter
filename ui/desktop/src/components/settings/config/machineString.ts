/**
 * The attributes of a text field that holds a MACHINE string: a command (`claude`), a URL, a model
 * id (`gpt-5.5-2026-04-24`), a region (`us-west-2`), a profile name, a config value. Vision
 * V4-R7-5.
 *
 * The definition lives with the field primitive, `ui/input.tsx`, where `<Input machine />` is the
 * way to ask for it; this module keeps the name the Configure dialogs, the Add provider form and
 * the Configuration editor already import, so the three attributes have exactly one spelling.
 * Spread it where a props object is built before the element exists
 * (`DefaultProviderSetupForm`'s field table); everywhere else write `<Input machine />`.
 *
 * Do not put it on a field that holds a person's own words (a chat message, a description): that
 * one keeps its spellcheck, which the Chat tab's switch controls. Secret fields need nothing,
 * `SecretInput` already switches spellcheck off.
 *
 * The tests that hold it in place are behavioural, on each form: `DefaultProviderSetupForm.test`,
 * `ConfigSettings.privacy.test`, and the Chromium cases in `customProviderForm.browser.test` and
 * `capabilities/settingsBFill.browser.test`, which read the engine's `input.spellcheck`. The
 * primitive's own cases are in `ui/input.test.tsx`.
 */
export { machineStringProps } from '../../ui/input';
