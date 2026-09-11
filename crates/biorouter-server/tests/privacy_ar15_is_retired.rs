//! AR-15 is retired, and this is the gate that every document says so.
//!
//! Issue #56. [`AR-15`] recorded that a caller holding the daemon secret could
//! raise its own session's capability with no credentials. Task 18A closed it
//! (commit `0757823f`): `POST /agent/update_provider` now 409s an upward bind
//! that carries no `X-User-Action`. For a while afterwards three documents went
//! on describing the risk as accepted — including the *user-facing*
//! `privacy-tiers.md` — and one of them under the heading "why it is accepted
//! rather than closed".
//!
//! ⚠ **A security document that overstates a weakness is wrong.** It is not the
//! safe direction to err in: it spends the reader's trust in the entries that
//! are accurate, and the next reader who checks one claim and finds it stale has
//! no reason to believe the rest. So this is a test and not a convention.
//!
//! **What each assertion would fail**, because a doc gate that only greps the
//! happy path is worth nothing:
//!
//! - `the_marker_matches_the_convention_its_siblings_use` — a marker invented
//!   for AR-15 alone (`### AR-15 (closed)`), or none at all. The convention is
//!   **read off AR-1/6/8/9/10/11**, not hardcoded, so it also fails if the
//!   siblings' shape changes and AR-15 is not brought along.
//! - `the_banner_names_what_closed_it` — a heading marked RETIRED over a body
//!   that never says which ruling, commit or gate did it.
//! - `the_section_no_longer_argues_for_accepting_it` — the marker added and the
//!   body's verdict left standing underneath, which is the failure this gate
//!   exists for.
//! - `every_reference_resolves_to_the_retired_heading` — the heading renamed and
//!   the six in-document links left pointing at the old slug. A reader who
//!   follows one lands nowhere and the retirement is invisible. The slug
//!   function is **self-checked against AR-11's live anchor**, so it cannot pass
//!   by computing a slug the tree does not use.
//! - `no_document_still_describes_it_as_open` — the plan fixed and the
//!   user-facing doc or the brief forgotten.
//! - `the_documented_closure_is_the_one_the_code_performs` — the deepest one:
//!   the docs claim retired while `routes/agent.rs` has stopped enforcing it.
//!   Deleting `!is_user_action(&headers)` from the gate makes every document in
//!   this repository a lie, and nothing else in the tree notices, because the
//!   route's own tests assert the *refusal*, not the *reason*.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
// The lib's copy is `#[cfg(test)]` and is NOT compiled into an integration
// binary — every one of these files declares its own. Nothing here builds an
// `AppState` today, so this is a floor rather than a fix; the guard exists
// because the next test added here would not have one.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

/// The commit that implemented DR-16 and retired AR-15.
const CLOSING_COMMIT: &str = "0757823f";

const PLAN: &str = include_str!("../../../docs/security/privacy-tiers-execution-plan.md");
const USER_DOC: &str = include_str!("../../../docs/security/privacy-tiers.md");
const BRIEF: &str = include_str!("../../../docs/security/privacy-tiers-implementation-brief.md");
const AGENT_ROUTE: &str = include_str!("../src/routes/agent.rs");

/// The accepted risks that already carry a marker when this test was written.
/// The convention is derived from them; AR-15 is then held to it.
const MARKED_SIBLINGS: [&str; 5] = ["AR-6", "AR-8", "AR-9", "AR-10", "AR-11"];

/// `s[a..b]`, boundary-checked. This workspace denies clippy's `string_slice`,
/// and the deny earns its keep here: these documents are dense with em-dashes
/// and `⚠`, so a byte offset that lands mid-character would panic with a message
/// about UTF-8 rather than about the claim under test.
fn cut(s: &str, a: usize, b: usize) -> &str {
    s.get(a..b).unwrap_or_else(|| {
        panic!(
            "{a}..{b} is not a char boundary in a {}-byte document",
            s.len()
        )
    })
}

fn cut_from(s: &str, a: usize) -> &str {
    cut(s, a, s.len())
}

fn cut_to(s: &str, b: usize) -> &str {
    cut(s, 0, b)
}

/// The heading line for `ar`, matched on the exact `### AR-N — ` prefix so
/// `AR-1` cannot match `AR-10`.
fn heading(ar: &str) -> &'static str {
    let prefix = format!("### {ar} \u{2014} ");
    PLAN.lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{prefix}…` heading in the execution plan"))
}

/// Everything from `ar`'s heading to the next `### ` heading.
fn section(ar: &str) -> &'static str {
    let h = heading(ar);
    let start = PLAN.find(h).expect("the heading came from PLAN");
    let rest = cut_from(PLAN, start + h.len());
    let end = rest.find("\n### ").unwrap_or(rest.len());
    cut(PLAN, start, start + h.len() + end)
}

/// The blockquote immediately under a heading: the consecutive `>` lines that
/// follow it, which is where every marked sibling puts its banner.
fn banner(ar: &str) -> String {
    section(ar)
        .lines()
        .skip(1)
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| l.starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The marker field of a heading: `### AR-9 — RETIRED by DR-17 — Layer A …`
/// yields `RETIRED by DR-17`. `None` when the heading has only two fields, i.e.
/// carries no marker at all.
fn marker(ar: &str) -> Option<String> {
    let parts: Vec<&str> = heading(ar).split(" \u{2014} ").collect();
    (parts.len() >= 3).then(|| parts[1].trim().to_string())
}

/// GitHub's heading slug: lowercase, drop everything that is not alphanumeric,
/// a space or a hyphen, then spaces to hyphens. Self-checked in
/// [`every_reference_resolves_to_the_retired_heading`] against an anchor the
/// tree already uses, so a wrong rule here cannot silently pass the test.
fn slug(heading_line: &str) -> String {
    heading_line
        .trim_start_matches('#')
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Nearest char boundary at or below `i`. These documents are full of em-dashes
/// and `⚠`, so a byte-offset window has to be nudged onto a boundary or the
/// slice panics and the test fails for a reason that is not the one it is about.
fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[test]
fn the_marker_matches_the_convention_its_siblings_use() {
    // Derive the convention rather than asserting a remembered one: collect the
    // marker verbs the already-marked risks use, and require AR-15 to use one
    // of them in the same `<VERB> by DR-<n>` shape.
    let mut verbs = Vec::new();
    for ar in MARKED_SIBLINGS {
        let m = marker(ar).unwrap_or_else(|| {
            panic!("{ar} lost its marker; the convention this test derives is gone")
        });
        let (verb, dr) = m
            .split_once(" by ")
            .unwrap_or_else(|| panic!("{ar}'s marker `{m}` is not `<VERB> by DR-<n>`"));
        assert!(
            verb.chars().all(|c| c.is_ascii_uppercase()),
            "{ar}'s marker verb `{verb}` is not upper case"
        );
        assert!(
            dr.strip_prefix("DR-")
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())),
            "{ar}'s marker does not name a decision of record: `{m}`"
        );
        verbs.push(verb.to_string());
    }
    assert!(
        verbs.len() == MARKED_SIBLINGS.len(),
        "the sibling scan is vacuous"
    );

    let m = marker("AR-15").expect(
        "AR-15's heading carries no marker. Task 18A closed it in commit 0757823f; a risk that \
         is closed and still reads as open is a security document overstating a weakness, which \
         costs the trust the accurate entries need. Mark it the way AR-6/8/9/10/11 are marked.",
    );
    let (verb, dr) = m
        .split_once(" by ")
        .unwrap_or_else(|| panic!("AR-15's marker `{m}` is not the siblings' `<VERB> by DR-<n>`"));
    assert!(
        verbs.iter().any(|v| v == verb),
        "AR-15 is marked `{verb}`, which no sibling uses ({verbs:?}). One convention, not two."
    );
    assert_eq!(
        dr, "DR-16",
        "AR-15 was closed by DR-16 (Task 18A), so that is the ruling its marker must name"
    );
}

#[test]
fn the_banner_names_what_closed_it() {
    // Every marked sibling opens its section with a blockquote carrying its own
    // marker verb. Establish that first, so the AR-15 assertion is a convention
    // check and not a lone rule.
    for ar in MARKED_SIBLINGS {
        let b = banner(ar);
        let verb = marker(ar).unwrap();
        let verb = verb.split_once(" by ").unwrap().0.to_string();
        assert!(
            b.starts_with('>') && b.contains(&verb),
            "{ar} has no `> …{verb}…` banner under its heading"
        );
    }

    let b = banner("AR-15");
    for needed in [
        "RETIRED",
        "DR-16",
        "Task 18A",
        CLOSING_COMMIT,
        "2026-08-02",
        // The gate itself, quoted, so the banner and the code can be compared —
        // see `the_documented_closure_is_the_one_the_code_performs`.
        "raise_needs_user_action",
        "is_user_action",
        "TierRaiseNeedsUser",
    ] {
        assert!(
            b.contains(needed),
            "AR-15's retirement banner does not mention `{needed}`. A marker that does not name \
             the ruling, the task, the commit and the gate is not a record of the closure: the \
             next reader cannot check it, and an unverifiable claim is how this entry went stale \
             in the first place.\nBanner reads:\n{b}"
        );
    }
}

#[test]
fn the_section_no_longer_argues_for_accepting_it() {
    let body = squash(section("AR-15"));
    // The exact heading the section used to carry. This is the claim the gate
    // exists to kill: a marker on the heading with the old verdict still
    // standing underneath it reads as a contradiction, and a reader who skims
    // takes the body.
    for stale in [
        "Why it is accepted rather than closed.",
        "This plan does not close it",
    ] {
        assert!(
            !body.contains(stale),
            "AR-15's section still contains `{stale}`, which asserts the risk is open"
        );
    }
    // And the section must say, in its own body and not only in the banner,
    // what the daemon now requires.
    assert!(
        body.contains("X-User-Action"),
        "AR-15's body never names the proof that closed it"
    );
}

#[test]
fn every_reference_resolves_to_the_retired_heading() {
    // Self-check the slug rule against an anchor the tree already relies on. If
    // this fails, every assertion below is comparing against a slug GitHub does
    // not generate, and the whole test is theatre.
    let ar11 = slug(heading("AR-11"));
    assert_eq!(
        ar11, "ar-11--amended-by-dr-17--the-daemons-own-api-secret-is-recoverable",
        "the slug rule no longer reproduces the anchor the plan itself links to"
    );

    let want = slug(heading("AR-15"));
    assert!(
        want.contains("retired-by-dr-16"),
        "AR-15's slug does not carry its marker: `{want}`"
    );

    let mut total = 0usize;
    for (name, doc) in [
        ("privacy-tiers-execution-plan.md", PLAN),
        ("privacy-tiers.md", USER_DOC),
        ("privacy-tiers-implementation-brief.md", BRIEF),
    ] {
        let mut seen = 0usize;
        for (i, _) in doc.match_indices("#ar-15-") {
            let rest = cut_from(doc, i + 1);
            let end = rest.find(')').expect("an anchor inside a markdown link");
            assert_eq!(
                cut_to(rest, end),
                want,
                "{name} links to a stale AR-15 anchor. Renaming the heading without updating the \
                 links leaves a reader who follows one on a page that does not scroll, so the \
                 retirement is invisible exactly where someone went looking for it."
            );
            seen += 1;
        }
        assert!(
            seen > 0,
            "{name} does not link AR-15 at all, so it cannot be saying what became of it"
        );
        total += seen;
    }
    assert!(total >= 8, "only {total} AR-15 links across the three docs");
}

#[test]
fn no_document_still_describes_it_as_open() {
    // Every mention of AR-15 outside its own section must be either a link to
    // the retired anchor, part of an `AR-1…AR-15` range, or sitting next to a
    // word that says what happened to it.
    let own_section = section("AR-15");
    for (name, doc) in [
        ("privacy-tiers-execution-plan.md", PLAN),
        ("privacy-tiers.md", USER_DOC),
        ("privacy-tiers-implementation-brief.md", BRIEF),
    ] {
        for (i, _) in doc.match_indices("AR-15") {
            let after = cut_from(doc, i + 5);
            if after.starts_with("](") {
                continue; // a link; the anchor is checked above
            }
            let before = cut_to(doc, i);
            if before.ends_with("AR-1 through ") || before.ends_with("AR-1\u{2013}") {
                continue; // `AR-1–AR-15`, a range over the whole list
            }
            let lo = floor_boundary(doc, i.saturating_sub(250));
            let hi = floor_boundary(doc, (i + 250).min(doc.len()));
            let window = cut(doc, lo, hi);
            let disclosed = ["retired", "RETIRED", "withdrawn", "no longer"]
                .iter()
                .any(|w| window.contains(w));
            let own_start = PLAN.find(own_section).unwrap();
            let in_own_section = name == "privacy-tiers-execution-plan.md"
                && i >= own_start
                && i < own_start + own_section.len();
            assert!(
                disclosed || in_own_section,
                "{name} mentions AR-15 at byte {i} with nothing nearby saying it was retired. \
                 A bare citation of a closed risk reads as a live one.\n…{window}…"
            );
        }
    }
}

#[test]
fn the_documented_closure_is_the_one_the_code_performs() {
    // The docs now assert a specific gate. If the gate goes, the docs are
    // wrong in the *dangerous* direction — claiming a hole is closed when it is
    // open — and no other test in this tree ties the two together.
    //
    // ⚠ The scan starts at `update_agent_provider`, the handler AR-15 is about.
    // It used to take the FIRST refusal in the file, which stopped being this
    // handler's when `eb594ded` put the new-chat gate above it: from then on the
    // scan read that gate, and deleting the proof from this one left it green.
    // The new-chat gate is SD-9's, a different rule with its own tests in
    // `routes/agent.rs`.
    let handler = AGENT_ROUTE
        .find("async fn update_agent_provider")
        .expect("update_agent_provider moved; AR-15's gate is the one inside it");
    let body = cut_from(AGENT_ROUTE, handler);
    // Bounded by the next route, so a refusal that left this handler cannot be
    // found in the one after it.
    let body = cut_to(body, body.find("\n#[utoipa::path(").unwrap_or(body.len()));
    let refusal = handler
        + body
            .find("PrivacyRefusal::TierRaiseNeedsUser")
            .expect("update_agent_provider no longer refuses an unproven tier raise at all");
    let guard = cut_to(AGENT_ROUTE, refusal)
        .rfind("    if ")
        .expect("the tier-raise refusal is not inside an `if`");
    let condition = cut(AGENT_ROUTE, guard, refusal);
    assert!(
        condition.len() < 400,
        "the condition scan ran past its `if` and is reading unrelated code ({} bytes)",
        condition.len()
    );
    for token in [
        "privacy_tiers_enabled()",
        "raise_needs_user_action(",
        "!is_user_action(",
    ] {
        assert!(
            condition.contains(token),
            "the tier-raise refusal no longer branches on `{token}`, so AR-15 is open again, and \
             three documents, including the user-facing one, now say it is closed. Either restore \
             the gate or un-retire AR-15 in all of them.\nGuard reads: {condition}"
        );
    }

    // A negative control, so the extractor is provably not matching anything it
    // is handed: the same file's `update_working_dir` is not a raise channel.
    let elsewhere = AGENT_ROUTE
        .find("async fn update_working_dir")
        .expect("update_working_dir moved; pick another non-raise handler");
    assert!(
        !cut(
            AGENT_ROUTE,
            elsewhere,
            (elsewhere + 800).min(AGENT_ROUTE.len())
        )
        .contains("raise_needs_user_action("),
        "the scan is over-reading: a handler that is not a raise channel reported the guard"
    );
}
