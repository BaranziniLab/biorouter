//! Free-text search over a catalog of named entries — the ONE matcher behind
//! `skills__searchMarketplaceSkills`,
//! `extensionmanager__search_marketplace_extensions` and the installed-skill
//! `skills__searchSkills`. A new catalog search should call [`rank`] with its
//! own fields rather than grow a matcher of its own: every copy of this logic
//! so far has drifted into the failure below.
//!
//! ⚠ **A query is a set of words, not a substring.** The matcher this replaced
//! asked whether the WHOLE lowercased query occurred inside a single field, so a
//! one-word query worked and every phrase failed. Measured in the 2026-09-10
//! composer QA run (finding F5), one chat, one live registry:
//! `R scripting ggplot visualization` → `total: 0`, while `ggplot` → 2 and
//! `r-scripting` → 1. A model composes exactly that phrase on a user's behalf,
//! so the shape that failed was the common one, and the model went on to tell
//! the user the marketplace had nothing. The installed-skill search failed the
//! same phrase through code of its own — it kept a skill only when EVERY word
//! was in it — and answered `total: 0` with a ggplot skill and an R-scripting
//! skill installed.
//!
//! So a query is split into terms and an entry is a hit when it matches ANY of
//! them. The union is deliberate: no single entry has to contain every word a
//! user happened to say, and an AND over a phrase is the same empty answer with
//! a different cause. Precision comes from the ranking instead, best first:
//!
//! 1. an entry holding the query **as written** — its words, in that order, as
//!    whole words — which no scatter of the same words outranks;
//! 2. then by **how many terms** it matched, so an entry matching every term
//!    precedes one matching some;
//! 3. then by **where** each term matched — the id or name outweighs a tag,
//!    which outweighs the description — and how exactly (the whole word, the
//!    start of one, or inside one);
//! 4. then the order the entries were given in — the registry's is by id, the
//!    installed skills' by name — so a result never reshuffles.
//!
//! Three rules keep the union from drowning the useful hits, and the first two
//! were needed by the measured query itself. The third — an unanchored match has
//! to be worth something, see [`substantial_infix`] — closes the same failure one
//! step further in: a query that finds everything says nothing, whether it got
//! there through a repeated field or through a three-letter morpheme.
//!
//! * **A term under three characters matches whole words only.** `r` has to
//!   find the R language; as a substring it matched nearly every entry. The
//!   query as written is held to the same edges — it counts only where it
//!   starts and ends at a word boundary. Tested as a plain substring it let a
//!   query that IS one short term back in through every word containing it:
//!   `R` alone still returned all five fixture skills of the installed-skill
//!   search, three of them with no matched term at all, and `R scripting`
//!   ranked "for scripting" above an entry that said both words.
//! * **Filler words are dropped** ("a skill about R" is `r`), because in a union
//!   a word like `for` or `about` inflates the term count of every entry whose
//!   prose happens to use it, which ranked noise above the real hit.

/// How much a match in one field says about an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Weight {
    /// Free prose: a description.
    Prose = 1,
    /// Curated labels: tags, keywords, an organization, a bundle.
    ///
    /// ⚠ **Not a curation bucket a surface also offers as a filter control.** A
    /// skill's `category` was here, and it is the licence's defect again: `Core`
    /// names 57 of the shipped registry's 129 skills and `Biomedical` 63, so
    /// `core` returned 59 and `biomedical` 65 — half the catalog, ranked by a
    /// word the user did not mean. Every surface that shows the category answers
    /// it with a control instead — the desktop Browse-skills modal's own category
    /// filter (`All` / `Core skills` / `Developer & authoring` /
    /// `Biomedical analysis`, measured showing exactly the 9 Developer rows that
    /// `developer` used to return), and three `data-facet="category"` chips on the
    /// website shelf — and the website never searched it, so dropping it is also
    /// what brings the three matchers into step. See `MarketplaceCatalog::search_skills`.
    Label = 2,
    /// What the entry is called: its id and names.
    Name = 3,
}

/// Words that say how a request is phrased, not what it is for. Dropped from a
/// query unless nothing else is left, so a query made only of them (`agent`,
/// `or`) still searches for what it says.
const FILLER: &[&str] = &[
    "a",
    "about",
    "an",
    "and",
    "any",
    "are",
    "baam",
    "be",
    "by",
    "can",
    "do",
    "does",
    "find",
    "for",
    "from",
    "help",
    "how",
    "i",
    "in",
    "into",
    "is",
    "it",
    "its",
    "looking",
    "marketplace",
    "me",
    "my",
    "need",
    "of",
    "on",
    "or",
    "please",
    "search",
    "some",
    "that",
    "the",
    "this",
    "to",
    "use",
    "using",
    "via",
    "want",
    "what",
    "which",
    "with",
];

/// Filler specific to the skills catalog: every entry in it is a skill.
pub(crate) const SKILL_NOISE: &[&str] = &["skill", "skills"];

/// Filler specific to the extensions catalog: every entry in it is an extension.
pub(crate) const EXTENSION_NOISE: &[&str] = &["extension", "extensions"];

/// Below this many characters a term matches whole words only.
const MIN_PARTIAL_CHARS: usize = 3;

/// At or above this many characters a term may match anywhere inside a word,
/// however long the word. Below it, [`substantial_infix`] asks for half.
const MIN_INFIX_CHARS: usize = 4;

/// One entry a search returned, with the query terms it matched.
#[derive(Debug)]
pub struct CatalogSearchHit<'a, T> {
    pub entry: &'a T,
    /// The terms this entry matched, in query order. Empty only when the query
    /// holds no word at all (`++`), so that nothing but the query as written
    /// could have found the entry.
    pub matched_terms: Vec<String>,
}

/// A ranked search: the terms the query was split into, and every entry that
/// matched at least one of them (or holds the whole query as written), best
/// first.
#[derive(Debug)]
pub struct CatalogSearch<'a, T> {
    /// What the query was read as, after filler words were dropped. Reported
    /// to the model so a surprising result can be traced to its input.
    pub terms: Vec<String>,
    pub hits: Vec<CatalogSearchHit<'a, T>>,
}

impl<T> CatalogSearch<'_, T> {
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// What was searched for, as a sentence fragment for an empty result:
    /// "any of the terms `r`, `ggplot`", or "the query `…`" when the query
    /// held no word at all.
    pub fn describe_query(&self, query: &str) -> String {
        if self.terms.is_empty() {
            return format!("the query `{query}`");
        }
        let terms = self
            .terms
            .iter()
            .map(|term| format!("`{term}`"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("any of the terms {terms}")
    }
}

/// Lowercase words, split at every character that is not a letter or digit —
/// whitespace and punctuation alike, so `r-scripting` is `r` + `scripting` and
/// `ggplot2` stays one word.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// Does `label` say nothing that `license` does not — is every word of it a word
/// of the licence? A catalog whose entries carry a licence drops such a label
/// when it assembles an entry's searchable text.
///
/// The licence itself is not a searchable field: every entry in the BAAM
/// registry is Apache-2.0, so it separates nothing, and both catalog searches
/// leave the field out for that reason.
///
/// ⚠ **Leaving the FIELD out was not enough.** A registry republishes the licence
/// as one of the entry's own tag chips — and, for a skill, again among its
/// keywords — and labels are searched, rightly: `MCP`, `ELN` and `Imaging` are
/// exactly what a tag is for. Measured in the Browse-extensions modal on
/// 2026-09-12 against the live 37-entry registry, with the field already gone:
/// `PACS` → 31 of 37, `pac` → 31, `apache` → 31, and not one of the 31 about
/// PACS. The three counts agreeing is the identification — `PACS` reaches `pac`
/// through the plural fallback in [`term_strength`], `pac` is inside `apache`,
/// and 31 rows wear an `Apache-2.0` chip. Removing the field had moved the defect
/// one field over, where a test asserting "the licence is not searched" still
/// passed.
///
/// Compared by WORDS rather than by equality, because the second spelling is not
/// the first: the tag is `Apache-2.0` and the keyword is `apache`. An equality
/// test drops the tag and keeps the keyword, which is the same half-fix again.
///
/// What this deliberately does not do: drop every label (`MCP`, `Imaging`, `ELN`,
/// `Registry` are real search value), or name a licence in the matcher
/// (`Apache`, `MIT` — the next licence reopens the hole). The cost of the word
/// test is a licence id built from a topical word — `Python-2.0`, `Ruby`,
/// `PostgreSQL` — on an entry that also tags itself with that word; the tag is
/// then dropped for saying only what the licence says. No entry in the shipped
/// registry is such a case (measured over all 166: the rule drops the 129 licence
/// labels and nothing else), and an equality test pays a smaller version of the
/// same cost.
pub(crate) fn names_only_the_license(label: &str, license: &str) -> bool {
    let license_words: Vec<String> = words(license).collect();
    let mut label_words = words(label);
    match label_words.next() {
        // An empty label says nothing at all, which is not the same as saying
        // only the licence: leave it, so the rule stays about the licence.
        None => false,
        Some(first) => {
            license_words.contains(&first) && label_words.all(|word| license_words.contains(&word))
        }
    }
}

/// The distinct terms of `query`, in the order written, without filler.
fn terms(query: &str, noise: &[&str]) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    for word in words(query) {
        if !all.contains(&word) {
            all.push(word);
        }
    }
    let meaningful: Vec<String> = all
        .iter()
        .filter(|term| !FILLER.contains(&term.as_str()) && !noise.contains(&term.as_str()))
        .cloned()
        .collect();
    if meaningful.is_empty() {
        all
    } else {
        meaningful
    }
}

/// Is `term`, found inside `word` without touching its start, enough of that
/// word to be a search rather than a morpheme?
///
/// The matcher already grades an anchored match above an unanchored one — a
/// prefix scores 2, an infix 1 — and [`MIN_PARTIAL_CHARS`] was the only
/// admission gate, so three characters bought a match anywhere inside any word.
/// Measured on the 37-entry extension shelf of the shipped registry, that is
/// what made three-letter queries return it whole: `lab` → **37 of 37**, `gen` →
/// 36, `age` → 36. Per hit, 32 of `lab`'s were an infix of `baranzinilab` — the
/// organization — and 33 each of `gen`'s and `age`'s an infix of `…Agent` in the
/// extension's own NAME. So this is not a field that can be dropped, the way the
/// licence and the version were: `lab` alone would be fixed by dropping
/// `organization`, and nothing can drop a name. What all three share is a
/// three-letter term with no boundary on either side. On the skills shelf the
/// same rule had `ing` matching 88 of 129 skills, `ica` 85, `ion` 84, `cal` 80
/// and `tio` 77 — every one of them a morpheme.
///
/// So an unanchored match needs either [`MIN_INFIX_CHARS`] characters, or half
/// the word it sits in. Two arms rather than one number, because each closes a
/// case the other gets wrong, and each was checked against the shipped
/// registry's own vocabulary — every word a visitor could be echoing back.
///
/// ⚠ The counts that used to sit here were **not reproducible** and are gone.
/// Measured 2026-09-12 from `landing/registry.json`, over exactly the fields
/// these matchers search: **1,445** distinct words with the prose descriptions,
/// **773** without them. No combination of fields yields the 807 this comment
/// claimed, and this change's own siblings say 795 (`catalog_search_mirrors.rs`,
/// `landing/baam.html`) — three figures for one corpus is how a number nobody
/// re-measures drifts. Quote a corpus size only together with the field set that
/// produces it.
///
/// * A flat four-character floor drops the hits a short term earns inside a
///   SHORT word: `rna` in `scRNA`, `rRNA`, `miRNA`, `piRNA` and `sem` in `RSEM`
///   are the search, not a morpheme. It cost `rna` `single-cell` and
///   `microbiome`, and `sem` `rna-quantification`.
/// * A flat half-the-word ratio drops the hits a LONG term earns inside a longer
///   compound, which is most of a biomedical vocabulary: `omics` stopped finding
///   `transcriptomics`, `metabolomics` and `epigenomics`, and `flow` stopped
///   finding `workflows`.
///
/// What the two arms remove together is dominated by the `lab` flood; the rest
/// is `pro` inside "reproducible"/"improving", `logs` inside "pathology", `end`
/// inside "frontend"/"appendix". Half is also the proportion
/// this rule's own documented case sits at — `heatmap` is 7 of
/// `complexheatmap`'s 14 — so the arm that admits a short term is calibrated to
/// the example the infix rule exists for, rather than to the queries it refuses.
fn substantial_infix(term: &str, word: &str) -> bool {
    let term_chars = term.chars().count();
    term_chars >= MIN_INFIX_CHARS || term_chars * 2 >= word.chars().count()
}

/// How well `term` matches one field word: 3 for the whole word, 2 for its
/// start, 1 for anywhere inside it (`heatmap` in `complexheatmap`), 0 for no
/// match. A short term matches whole words only, and a term that is short
/// relative to the word matches only at its start — see [`substantial_infix`].
fn strength(term: &str, word: &str) -> u32 {
    if word == term {
        3
    } else if term.chars().count() < MIN_PARTIAL_CHARS {
        0
    } else if word.starts_with(term) {
        2
    } else if word.contains(term) && substantial_infix(term, word) {
        1
    } else {
        0
    }
}

/// `term`'s strength against `word`, falling back to its singular so
/// `visualizations` finds `visualization` and `heatmaps` finds `heatmap`.
/// `class` and `gis` are left alone.
fn term_strength(term: &str, word: &str) -> u32 {
    let direct = strength(term, word);
    if direct > 0 {
        return direct;
    }
    match term.strip_suffix('s') {
        Some(stem) if stem.chars().count() >= MIN_PARTIAL_CHARS && !stem.ends_with('s') => {
            strength(stem, word)
        }
        _ => 0,
    }
}

/// Does `text` hold `phrase` as written — starting and ending at a word
/// boundary, not inside a longer word? `r scripting` is in "R scripting" but
/// not in "for scripting", where its `r` is the tail of `for`. Both are
/// lowercase already.
///
/// An edge of `phrase` that is not a letter or digit needs no boundary: it is
/// one, so `++` is written in "c++".
///
/// Every character position is tried, not only the occurrences `find` would
/// step through, because a refused occurrence can overlap an accepted one:
/// `a a` in "ba a a" is written only from the second `a`.
fn written_in(text: &str, phrase: &str) -> bool {
    let starts_word = phrase.chars().next().is_some_and(char::is_alphanumeric);
    let ends_word = phrase
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric);
    let mut before = None;
    for (start, current) in text.char_indices() {
        if let Some(rest) = text.get(start..).filter(|rest| rest.starts_with(phrase)) {
            let after = rest
                .get(phrase.len()..)
                .and_then(|tail| tail.chars().next());
            let opens = !starts_word || !before.is_some_and(char::is_alphanumeric);
            let closes = !ends_word || !after.is_some_and(char::is_alphanumeric);
            if opens && closes {
                return true;
            }
        }
        before = Some(current);
    }
    false
}

/// Rank `entries` against `query`. `fields` names the text of one entry that
/// is searched, and how much a match there counts.
///
/// An empty (or all-whitespace) query is the browse case: every entry, in the
/// order given.
pub(crate) fn rank<'a, T>(
    query: &str,
    noise: &[&str],
    entries: impl IntoIterator<Item = &'a T>,
    fields: impl Fn(&'a T) -> Vec<(&'a str, Weight)>,
) -> CatalogSearch<'a, T> {
    let phrase = query.trim().to_lowercase();
    if phrase.is_empty() {
        return CatalogSearch {
            terms: Vec::new(),
            hits: entries
                .into_iter()
                .map(|entry| CatalogSearchHit {
                    entry,
                    matched_terms: Vec::new(),
                })
                .collect(),
        };
    }
    let terms = terms(query, noise);

    let mut ranked = Vec::new();
    for entry in entries {
        let fields = fields(entry);
        let written = fields
            .iter()
            .any(|(text, _)| written_in(&text.to_lowercase(), &phrase));
        let entry_words: Vec<(String, u32)> = fields
            .iter()
            .flat_map(|(text, weight)| words(text).map(move |word| (word, *weight as u32)))
            .collect();

        let mut matched_terms = Vec::new();
        let mut score = 0;
        for term in &terms {
            let best = entry_words
                .iter()
                .map(|(word, weight)| term_strength(term, word) * weight)
                .max()
                .unwrap_or(0);
            if best > 0 {
                matched_terms.push(term.clone());
                score += best;
            }
        }
        if written || !matched_terms.is_empty() {
            ranked.push((
                (written, matched_terms.len(), score),
                CatalogSearchHit {
                    entry,
                    matched_terms,
                },
            ));
        }
    }
    // Stable, and descending on the key: equal ranks keep the order given.
    ranked.sort_by(|(left, _), (right, _)| right.cmp(left));
    CatalogSearch {
        terms,
        hits: ranked.into_iter().map(|(_, hit)| hit).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Entry {
        id: &'static str,
        name: &'static str,
        description: &'static str,
        tags: &'static [&'static str],
    }

    fn fields(entry: &Entry) -> Vec<(&str, Weight)> {
        let mut fields = vec![
            (entry.id, Weight::Name),
            (entry.name, Weight::Name),
            (entry.description, Weight::Prose),
        ];
        fields.extend(entry.tags.iter().map(|tag| (*tag, Weight::Label)));
        fields
    }

    fn ids<'a>(search: &CatalogSearch<'a, Entry>) -> Vec<&'a str> {
        search.hits.iter().map(|hit| hit.entry.id).collect()
    }

    const ENTRIES: &[Entry] = &[
        Entry {
            id: "complex-plots",
            name: "Complex Plots",
            description: "Draws annotated heat maps with the ComplexHeatmap package.",
            tags: &["ComplexHeatmap"],
        },
        Entry {
            id: "prose-only",
            name: "Prose Only",
            description: "Mentions scripting in passing.",
            tags: &[],
        },
        Entry {
            id: "r-scripting",
            name: "R Scripting",
            description: "Tidyverse conventions for R code.",
            tags: &["R"],
        },
    ];

    #[test]
    fn a_query_is_split_at_whitespace_and_punctuation_and_lowercased() {
        assert_eq!(
            terms("R scripting, ggplot/Visualization", &[]),
            ["r", "scripting", "ggplot", "visualization"]
        );
        assert_eq!(terms("r-scripting", &[]), ["r", "scripting"]);
        assert_eq!(
            terms("ggplot2 ggplot2", &[]),
            ["ggplot2"],
            "terms are distinct"
        );
    }

    #[test]
    fn filler_is_dropped_unless_it_is_all_there_is() {
        assert_eq!(
            terms("a skill about R scripting or ggplot", SKILL_NOISE),
            ["r", "scripting", "ggplot"]
        );
        assert_eq!(terms("skills", SKILL_NOISE), ["skills"]);
        assert_eq!(
            terms("skills", EXTENSION_NOISE),
            ["skills"],
            "a catalog's own noise words are its own"
        );
    }

    /// `r` as a substring is in nearly every word of prose; as a term it must
    /// mean the R language.
    #[test]
    fn a_short_term_matches_whole_words_only() {
        let search = rank("R scripting", &[], ENTRIES, fields);
        let prose = search
            .hits
            .iter()
            .find(|hit| hit.entry.id == "prose-only")
            .expect("`scripting` is in its description");
        assert_eq!(
            prose.matched_terms,
            ["scripting"],
            "the `r` inside `scripting` is not the R language"
        );
        assert!(
            !ids(&search).contains(&"complex-plots"),
            "nor is the `r` inside `Draws`"
        );
    }

    /// The same rule for a query that IS one short term. The whole-query check
    /// used to be a plain substring test, so `r` alone found every entry with
    /// the letter anywhere — `complex-plots` through "Draws", `prose-only`
    /// through its own name — and the rule above held only inside a longer
    /// phrase.
    #[test]
    fn a_one_letter_query_matches_whole_words_only() {
        let search = rank("R", &[], ENTRIES, fields);
        assert_eq!(ids(&search), ["r-scripting"]);
        assert_eq!(search.hits[0].matched_terms, ["r"]);
    }

    /// The query as written outranks any count of separate words, so it has to
    /// be written there: `r scripting` inside "for scripting" is the tail of
    /// `for` and then a word. Read as a substring it ranked an entry matching
    /// one of the two words above one matching both.
    #[test]
    fn a_phrase_found_only_inside_other_words_is_not_the_query_as_written() {
        let entries = [
            Entry {
                id: "shell-snippets",
                name: "Shell Snippets",
                description: "Snippets for scripting the shell.",
                tags: &[],
            },
            Entry {
                id: "tidy-style",
                name: "Tidy Style",
                description: "Scripting conventions for R.",
                tags: &["R"],
            },
        ];
        let search = rank("R scripting", &[], &entries, fields);
        assert_eq!(ids(&search), ["tidy-style", "shell-snippets"]);
        assert_eq!(search.hits[0].matched_terms, ["r", "scripting"]);
        assert_eq!(search.hits[1].matched_terms, ["scripting"]);
    }

    /// An unanchored match has to be worth something. Three characters is enough
    /// to search from the START of a word — `gen` really does find `genomics` —
    /// and, inside a long one, is a morpheme: `lab` inside `BaranziniLab` and
    /// `gen` inside `…Agent` returned the whole 37-entry extension shelf.
    ///
    /// Asserted against [`strength`] directly, one word at a time, because at
    /// catalog level the same query reaches the same entry through several words
    /// and a count hides which rule admitted it.
    #[test]
    fn a_short_term_matches_inside_a_word_only_when_it_is_half_of_it() {
        // The three measured floods, at the word each of them came through.
        assert_eq!(strength("lab", "baranzinilab"), 0, "3 of 12");
        assert_eq!(strength("gen", "cdwagent"), 0, "3 of 8");
        assert_eq!(strength("age", "language"), 0, "3 of 8");
        // Unanchored is the only thing refused. The start of a word still counts
        // at three characters, and the whole word always counts.
        assert_eq!(strength("gen", "genomics"), 2);
        assert_eq!(strength("lab", "labarchives"), 2);
        assert_eq!(strength("lab", "lab"), 3);
        // A short term inside a SHORT word is the search, not a morpheme — and
        // these are the hits a flat four-character floor would have cost.
        assert_eq!(strength("rna", "scrna"), 1, "3 of 5");
        assert_eq!(strength("rna", "rrna"), 1, "3 of 4");
        assert_eq!(strength("sem", "rsem"), 1, "3 of 4");
        assert_eq!(strength("age", "image"), 1, "3 of 5");
        // At four characters a term is unanchored anywhere, however long the
        // word — which is what keeps a compound biomedical vocabulary findable.
        assert_eq!(strength("omics", "transcriptomics"), 1);
        assert_eq!(strength("flow", "workflows"), 1);
        // The case the infix rule was written for sits exactly on the boundary
        // the short arm draws, so it would pass on either arm.
        assert_eq!(strength("heatmap", "complexheatmap"), 1, "7 of 14");
        assert!(substantial_infix("heatmap", "complexheatmap"));
    }

    #[test]
    fn a_long_term_matches_inside_a_word_and_a_plural_finds_its_singular() {
        assert_eq!(
            ids(&rank("heatmap", &[], ENTRIES, fields)),
            ["complex-plots"],
            "`heatmap` inside `complexheatmap`"
        );
        assert_eq!(
            ids(&rank("heatmaps", &[], ENTRIES, fields)),
            ["complex-plots"],
            "no field says `heatmaps`; its singular is inside `complexheatmap`"
        );
        assert_eq!(
            ids(&rank("scripts", &[], ENTRIES, fields)),
            ["r-scripting", "prose-only"],
            "`scripts` is not in `scripting`, but `script` starts it"
        );
    }

    /// The union, ranked: every entry matching a term is returned, the one
    /// matching more terms first, and a name match ahead of a prose match.
    #[test]
    fn hits_are_the_union_ranked_by_terms_matched_then_by_where() {
        let search = rank("R scripting", &[], ENTRIES, fields);
        assert_eq!(ids(&search), ["r-scripting", "prose-only"]);
        assert_eq!(search.hits[0].matched_terms, ["r", "scripting"]);
        assert_eq!(search.hits[1].matched_terms, ["scripting"]);

        let by_place = rank("scripting", &[], ENTRIES, fields);
        assert_eq!(
            ids(&by_place),
            ["r-scripting", "prose-only"],
            "a match in the name outranks the same match in the description"
        );
    }

    #[test]
    fn a_phrase_is_written_only_between_word_boundaries() {
        assert!(written_in("r scripting", "r scripting"));
        assert!(written_in("tidy code for r.", "r"), "the second `r`");
        assert!(!written_in("snippets for scripting", "r scripting"));
        assert!(!written_in("tidyverse", "dy"));
        assert!(
            written_in("ba a a", "a a"),
            "written at the second `a`, which overlaps the refused first occurrence"
        );
        assert!(
            written_in("c++ code", "++"),
            "an edge that is not a letter or digit is a boundary itself"
        );
    }

    /// The query as written — its words, in order, as words — outranks any
    /// scatter of the same words, even one in a weightier field. A fragment of
    /// a word is not the query as written, though: `dy` inside `Tidyverse` is
    /// exactly the substring the short-term rule refuses.
    #[test]
    fn the_query_as_written_ranks_first_but_only_as_whole_words() {
        let entries = [
            Entry {
                id: "code-tidy",
                name: "Code Tidy",
                description: "Formatting rules.",
                tags: &[],
            },
            Entry {
                id: "styler",
                name: "Styler",
                description: "Writes tidy code.",
                tags: &[],
            },
        ];
        assert_eq!(
            ids(&rank("tidy code", &[], &entries, fields)),
            ["styler", "code-tidy"],
            "both hold both words, in the name or the prose; only one says `tidy code`"
        );

        assert!(
            rank("dy", &[], ENTRIES, fields).is_empty(),
            "`dy` is inside `Tidyverse`, not a word of it"
        );
        assert!(
            rank("s p", &[], ENTRIES, fields).is_empty(),
            "neither `s` nor `p` is a whole word"
        );
    }

    #[test]
    fn an_empty_query_browses_in_registry_order() {
        let search = rank("   ", &[], ENTRIES, fields);
        assert!(search.terms.is_empty());
        assert_eq!(ids(&search), ["complex-plots", "prose-only", "r-scripting"]);
    }

    /// The rule a catalog applies to its own labels. Both spellings the BAAM
    /// registry publishes go, which is the whole point — `Apache-2.0` is the tag
    /// and `apache` is the keyword, and an equality test would keep the second
    /// and leave `PACS` matching 49 skills through it.
    #[test]
    fn a_label_naming_only_the_licence_is_recognised_in_either_spelling() {
        for label in [
            "Apache-2.0",
            "apache",
            "APACHE",
            "apache 2.0",
            "2.0",
            "Apache/2.0",
        ] {
            assert!(
                names_only_the_license(label, "Apache-2.0"),
                "`{label}` says nothing `Apache-2.0` does not"
            );
        }
        // A label that says anything else stays searchable, including one that
        // merely contains a word of the licence.
        for label in [
            "Apache Spark",
            "MCP",
            "ELN",
            "Imaging",
            "Registry",
            "apachex",
        ] {
            assert!(
                !names_only_the_license(label, "Apache-2.0"),
                "`{label}` says more than the licence"
            );
        }
        // An empty label says nothing at all, which is not the same as saying
        // only the licence; and an entry with no licence has none to drop.
        assert!(!names_only_the_license("", "Apache-2.0"));
        assert!(!names_only_the_license("  -  ", "Apache-2.0"));
        assert!(!names_only_the_license("Apache-2.0", ""));
    }
}
