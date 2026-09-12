//! The marketplace matcher exists three times, and this is where the three are
//! held to one rule.
//!
//! `crates/biorouter/src/catalog_search.rs` is canonical. It is ported to
//! `ui/desktop/src/components/baam/search.ts` (the Browse modals) and to
//! `landing/marketplace-search.js` (the BAAM shelves at biorouter.ucsf.edu), and
//! each port has its own tests — but nothing could see the three at once, so what
//! drifted was not a rule but the FIELD LIST each caller hands the rule. Measured
//! on 2026-09-12 against the shipped registry: the Rust catalog and the desktop
//! modal searched a skill's `category` and the website never did, so `core`
//! answered 59 of 129 skills in the app and 2 of 132 cards on the site, and
//! `registry.ts` claimed the fields were the same.
//!
//! This is the only test that reads all three files, so it is deliberately about
//! text rather than behaviour: the numeric rules, and the one field list whose
//! divergence was invisible. Behaviour is pinned where each copy lives —
//! `catalog_search::tests` and `marketplace::tests` here,
//! `search.test.ts` / `registry.test.ts` in the desktop app, and
//! `landing/scripts/baam-search.test.mjs`, whose differential runs the website's
//! real page against the canonical field list over 795 catalog queries.

use std::fs;
use std::path::PathBuf;

fn repo(path: &str) -> String {
    let mut file = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    file.push("../..");
    file.push(path);
    fs::read_to_string(&file).unwrap_or_else(|error| panic!("reading {}: {error}", file.display()))
}

/// The integer a `const NAME = 4;` / `const NAME: usize = 4;` line declares.
/// Deliberately strict: a constant that has become an expression is a change this
/// test should notice rather than skip.
fn declared_number(source: &str, name: &str) -> u32 {
    let line = source
        .lines()
        .find(|line| line.contains(name) && line.contains('=') && line.trim_end().ends_with(';'))
        .unwrap_or_else(|| panic!("no `{name} = …;` declaration"));
    let digits: String = line
        .rsplit('=')
        .next()
        .unwrap()
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("`{name}` is declared as `{}`, not a number", line.trim()))
}

/// The body of the function `name` opens, up to the first `}` that closes a line
/// at the indent a function is declared at in one of these three files — column
/// zero (Rust module level, an exported TS function), two (inside the website's
/// IIFE) or four (a Rust `impl`). Whichever closes EARLIEST is the function's own,
/// which is enough to read one declaration without parsing three languages.
fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
    let (_, rest) = source
        .split_once(name)
        .unwrap_or_else(|| panic!("no function `{name}`"));
    ["\n}", "\n  }", "\n    }"]
        .iter()
        .filter_map(|close| rest.split_once(close).map(|(body, _)| body))
        .min_by_key(|body| body.len())
        .unwrap_or(rest)
}

const RUST: &str = "crates/biorouter/src/catalog_search.rs";
const DESKTOP: &str = "ui/desktop/src/components/baam/search.ts";
const WEBSITE: &str = "landing/marketplace-search.js";

/// One number in three files. `MIN_INFIX_CHARS` is the newer of the two and the
/// reason this test exists: a port that took the rule's shape and not its
/// threshold would pass its own tests and answer a different catalog.
#[test]
fn the_three_matchers_declare_the_same_thresholds() {
    for constant in ["MIN_PARTIAL_CHARS", "MIN_INFIX_CHARS"] {
        let rust = declared_number(&repo(RUST), constant);
        for port in [DESKTOP, WEBSITE] {
            assert_eq!(
                declared_number(&repo(port), constant),
                rust,
                "{port} declares a different {constant} from {RUST}"
            );
        }
    }
    // And they are not the same number, which is the whole rule: three characters
    // is enough to search from the start of a word and not from inside one.
    assert!(
        declared_number(&repo(RUST), "MIN_INFIX_CHARS")
            > declared_number(&repo(RUST), "MIN_PARTIAL_CHARS")
    );
}

/// Both arms, in all three. A port that kept only the threshold arm loses `rna`
/// inside `scRNA`; one that kept only the ratio arm loses `omics` inside
/// `transcriptomics`. Either way its own tests still pass.
#[test]
fn the_three_matchers_admit_an_unanchored_match_on_the_same_two_arms() {
    for (file, name) in [
        (RUST, "fn substantial_infix"),
        (DESKTOP, "export function substantialInfix"),
        (WEBSITE, "function substantialInfix"),
    ] {
        let source = repo(file);
        let body = function_body(&source, name);
        assert!(
            body.contains("MIN_INFIX_CHARS"),
            "{file}: `{name}` does not read MIN_INFIX_CHARS"
        );
        assert!(
            body.contains("* 2 >="),
            "{file}: `{name}` has no half-the-word arm: {body}"
        );
    }
}

/// The field list, which is what actually drifted. A skill's `category` is a
/// filter control on every surface that shows it — three chips in the desktop
/// modal, three `data-facet="category"` chips on the website shelf — and naming
/// 57 and 63 of 129 rows, searching it returned half the catalog.
///
/// Asserted as an absence, so each half also asserts the function was really
/// found and read: a body that no longer mentions `keywords` has not been parsed.
#[test]
fn neither_skill_search_reads_the_category_a_facet_already_answers() {
    for (file, name) in [
        (
            "crates/biorouter/src/marketplace.rs",
            "pub fn search_skills",
        ),
        (
            "ui/desktop/src/components/baam/registry.ts",
            "export function rankSkills",
        ),
    ] {
        let source = repo(file);
        let body = function_body(&source, name);
        assert!(
            body.contains("keywords"),
            "{file}: `{name}` was not read — its body mentions no keywords field: {body}"
        );
        assert!(
            !body.contains("category"),
            "{file}: `{name}` searches a skill's category again: {body}"
        );
    }
    // The organization is NOT the same case and stays searched: people browse the
    // extensions shelf by the lab that publishes it, and no surface offers the
    // full organization as a control.
    for (file, name) in [
        (
            "crates/biorouter/src/marketplace.rs",
            "pub fn search_extensions",
        ),
        (
            "ui/desktop/src/components/baam/registry.ts",
            "export function rankExtensions",
        ),
    ] {
        let source = repo(file);
        assert!(
            function_body(&source, name).contains("organization"),
            "{file}: `{name}` stopped searching the organization"
        );
    }
}
