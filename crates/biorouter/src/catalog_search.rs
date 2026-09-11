//! Free-text search over a catalog of named entries — the ONE matcher behind
//! `skills__searchMarketplaceSkills` and
//! `extensionmanager__search_marketplace_extensions`. A new catalog search
//! should call [`rank`] with its own fields rather than grow a matcher of its
//! own: every copy of this logic so far has drifted into the failure below.
//!
//! ⚠ **A query is a set of words, not a substring.** The matcher this replaced
//! asked whether the WHOLE lowercased query occurred inside a single field, so a
//! one-word query worked and every phrase failed. Measured in the 2026-09-10
//! composer QA run (finding F5), one chat, one live registry:
//! `R scripting ggplot visualization` → `total: 0`, while `ggplot` → 2 and
//! `r-scripting` → 1. A model composes exactly that phrase on a user's behalf,
//! so the shape that failed was the common one, and the model went on to tell
//! the user the marketplace had nothing.
//!
//! So a query is split into terms and an entry is a hit when it matches ANY of
//! them. The union is deliberate: no single entry has to contain every word a
//! user happened to say, and an AND over a phrase is the same empty answer with
//! a different cause. Precision comes from the ranking instead, best first:
//!
//! 1. an entry containing the query **verbatim** — what the old matcher found,
//!    so nothing it returned is lost;
//! 2. then by **how many terms** it matched, so an entry matching every term
//!    precedes one matching some;
//! 3. then by **where** each term matched — the id or name outweighs a tag,
//!    which outweighs the description — and how exactly (the whole word, the
//!    start of one, or inside one);
//! 4. then registry order, which is by id, so a result never reshuffles.
//!
//! Two rules keep the union from drowning the useful hits, and both were needed
//! by the measured query itself:
//!
//! * **A term under three characters matches whole words only.** `r` has to
//!   find the R language; as a substring it matched nearly every entry.
//! * **Filler words are dropped** ("a skill about R" is `r`), because in a union
//!   a word like `for` or `about` inflates the term count of every entry whose
//!   prose happens to use it, which ranked noise above the real hit.

/// How much a match in one field says about an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Weight {
    /// Free prose: a description.
    Prose = 1,
    /// Curated labels: tags, keywords, a category, an organization.
    Label = 2,
    /// What the entry is called: its registry id and names.
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

/// One entry a search returned, with the query terms it matched.
#[derive(Debug)]
pub struct CatalogSearchHit<'a, T> {
    pub entry: &'a T,
    /// The terms this entry matched, in query order. Empty only for an entry
    /// that nothing but the verbatim query found.
    pub matched_terms: Vec<String>,
}

/// A ranked search: the terms the query was split into, and every entry that
/// matched at least one of them (or the whole query verbatim), best first.
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

/// How well `term` matches one field word: 3 for the whole word, 2 for its
/// start, 1 for anywhere inside it (`heatmap` in `complexheatmap`), 0 for no
/// match. A short term matches whole words only.
fn strength(term: &str, word: &str) -> u32 {
    if word == term {
        3
    } else if term.chars().count() < MIN_PARTIAL_CHARS {
        0
    } else if word.starts_with(term) {
        2
    } else if word.contains(term) {
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

/// Rank `entries` against `query`. `fields` names the text of one entry that
/// is searched, and how much a match there counts.
///
/// An empty (or all-whitespace) query is the browse case: every entry, in
/// registry order.
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
        let verbatim = fields
            .iter()
            .any(|(text, _)| text.to_lowercase().contains(&phrase));
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
        if verbatim || !matched_terms.is_empty() {
            ranked.push((
                (verbatim, matched_terms.len(), score),
                CatalogSearchHit {
                    entry,
                    matched_terms,
                },
            ));
        }
    }
    // Stable, and descending on the key: equal ranks keep registry order.
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

    /// Everything the substring matcher found is still found: a query that
    /// occurs verbatim in a field is a hit even when its terms are too short
    /// to match on their own.
    #[test]
    fn a_verbatim_occurrence_is_still_a_hit() {
        let search = rank("s p", &[], ENTRIES, fields);
        assert!(search.is_empty(), "neither `s` nor `p` is a whole word");

        let search = rank("dy", &[], ENTRIES, fields);
        assert_eq!(ids(&search), ["r-scripting"], "`dy` inside `Tidyverse`");
        assert!(search.hits[0].matched_terms.is_empty());
    }

    #[test]
    fn an_empty_query_browses_in_registry_order() {
        let search = rank("   ", &[], ENTRIES, fields);
        assert!(search.terms.is_empty());
        assert_eq!(ids(&search), ["complex-plots", "prose-only", "r-scripting"]);
    }
}
