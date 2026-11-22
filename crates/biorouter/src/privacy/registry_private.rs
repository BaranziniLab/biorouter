//! **Generated file — do not edit by hand.**
//!
//! `landing/scripts/build-registry.mjs` writes this in the same run as
//! `landing/registry.json` and the desktop fallback snapshot, from the
//! `data-privacy` / `data-extension-name` / `data-affiliation` annotations on
//! the extension cards in `landing/baam.html`. Regenerate all three with:
//!
//! ```text
//! node landing/scripts/build-registry.mjs
//! ```
//!
//! The set has to live in Rust: there is no network path to the registry from
//! here (the only fetch is the Electron `main.ts` `registry:fetch` handler), so
//! without this file the CLI and the daemon can enforce nothing.
//!
//! Drift between the three is detectable, not merely discouraged:
//!
//! ```text
//! node landing/scripts/build-registry.mjs --check
//! ```
//!
//! regenerates all three in memory and fails if any committed copy differs. It
//! runs in CI (the Frontend workflow) and in `just check-everything`, so a hand
//! edit here — or an interrupted run that updated only some of the three — is
//! caught rather than trusted not to happen.
//!
//! Each const carries `#[rustfmt::skip]` so that `--check` above is the only
//! authority on its layout. rustfmt wraps a slice literal by `array_width`
//! (60 columns between the brackets), not by `max_width`, and a generator that
//! predicted the wrong one produced text `cargo fmt --check` rejected and
//! `--check` demanded — two gates in `just check-everything` with no state
//! satisfying both, over the file whose header forbids the obvious way out.

/// The BAAM extensions whose cards declare `data-privacy="private"`, and which
/// so must never be admitted to a public session.
///
/// Values are `name_to_key` **keys** — whitespace-stripped and lowercased —
/// which is the form `classify_extension` reduces its argument to before the
/// lookup. That makes the entry match either spelling the registry publishes:
/// the id (`cdwagent`) or the bundle `manifest.name` (`CDWAgent`).
#[rustfmt::skip]
pub const PRIVATE_EXTENSIONS: &[&str] = &["cdwagent", "gptragent", "ucsfomopagent"];

/// Whose agreements each private extension's data is under — DR-26's third
/// axis, keyed by the same `name_to_key` key as the set above.
///
/// A private extension with NO institutional constraint has **no row here**.
/// Absent means unconstrained (`ExtensionAffiliation::Any`, reachable from any
/// private model); an empty allowlist would mean the opposite — permits nothing
/// — so the generator refuses `data-affiliation=""` rather than emitting one.
#[rustfmt::skip]
pub const EXTENSION_AFFILIATIONS: &[(&str, &[&str])] =
    &[("cdwagent", &["ucsf"]), ("ucsfomopagent", &["ucsf"])];

/// Institution id → display name, for the warning copy.
///
/// DR-26 requires a cross-affiliation warning to NAME both institutions:
/// "this may be a compliance risk" is a shrug, not a warning a user can act on.
/// An id absent from this map has no display name, so the warning surfaces the
/// raw id — and, being absent from every allowlist that does not literally spell
/// it, it is a mismatch rather than a silent pass.
#[rustfmt::skip]
pub const INSTITUTIONS: &[(&str, &str)] = &[("ucsf", "UCSF")];
