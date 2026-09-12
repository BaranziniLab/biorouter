/* UCSF Biorouter — free-text search over the BAAM shelves.
 *
 * ⚠ **A query is a set of words, not a substring.** This is a rule-for-rule port
 * of `crates/biorouter/src/catalog_search.rs` — `marketplace/search.rs` until
 * PR #266 moved it — which is the canonical
 * matcher behind `skills__searchMarketplaceSkills` and
 * `extensionmanager__search_marketplace_extensions`. Read that module for the
 * reasoning; what follows is only what a JavaScript reader needs.
 *
 * The defect this closes is the one the Rust matcher closed, and this page
 * carried the last two copies of it. `filterExtensions`, `filterSkills` and
 * `filterWorkflows` each asked `hay.indexOf(q) !== -1` — whether the WHOLE
 * lowercased query occurred verbatim inside one card's text. A one-word query
 * worked and every phrase failed, so the natural-language queries a visitor
 * actually types returned the empty shelf:
 *
 *     "R scripting ggplot visualization"  ->  0 cards
 *     "SPOKE knowledge graph"             ->  0 cards
 *
 * So a query is split into terms and a card is a hit when it matches ANY of
 * them. Two rules keep that union from returning the whole catalog, and both
 * were needed by the measured queries themselves:
 *
 *   * A term under three characters matches whole words only. `r` has to find
 *     the R language; as a substring it matched nearly every card.
 *   * Filler words are dropped ("a skill about R" is `r`), because in a union a
 *     word like `for` or `about` inflates the term count of every card whose
 *     prose happens to use it.
 *   * An unanchored match has to be worth something — see `substantialInfix`.
 *     Three characters is enough to search FROM THE START of a word and not
 *     enough to search inside a long one: measured on this page against the live
 *     registry, `lab` showed 37 of 37 extension cards through `BaranziniLab` and
 *     `gen` and `age` 36 each through `…Agent`.
 *
 * **License is deliberately NOT searched**, matching the Rust matcher's field
 * list. That is why the callers build a weighted field list out of the card's
 * heading, prose and curated tags rather than handing over `textContent`, which
 * sweeps up the licence chip and every button label with it.
 *
 * **Ranking is computed but the shelves are not reordered**, and that is not a
 * shortcut. The Rust matcher ranks because it returns a truncated list to a
 * model; this page shows every hit, inside curated sections (Featured /
 * integrations, and the skill categories) whose order is editorial. Reordering
 * would destroy that curation to no visible end, so DOM order — which is
 * registry order, the matcher's own final tiebreak — is kept. `rank()` still
 * returns the score, so a future ranked surface on this site has it.
 */
(function (root, factory) {
  'use strict';
  var api = factory();
  if (typeof module === 'object' && module && module.exports) module.exports = api;
  else root.MarketplaceSearch = api;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  /* How much a match in one field says about a card. Mirrors `search::Weight`. */
  var Weight = { Prose: 1, Label: 2, Name: 3 };

  /* Words that say how a request is phrased, not what it is for. Dropped from a
     query unless nothing else is left, so a query made only of them still
     searches for what it says. Same list as `search::FILLER`. */
  var FILLER = [
    'a', 'about', 'an', 'and', 'any', 'are', 'baam', 'be', 'by', 'can', 'do',
    'does', 'find', 'for', 'from', 'help', 'how', 'i', 'in', 'into', 'is', 'it',
    'its', 'looking', 'marketplace', 'me', 'my', 'need', 'of', 'on', 'or',
    'please', 'search', 'some', 'that', 'the', 'this', 'to', 'use', 'using',
    'via', 'want', 'what', 'which', 'with'
  ];

  /* Per-catalog filler: every entry in the shelf already is one of these.
     SKILL_NOISE and EXTENSION_NOISE are `search.rs`'s own; WORKFLOW_NOISE is
     the identical rule applied to the third shelf, which the Rust matcher has
     no catalog for. */
  var SKILL_NOISE = ['skill', 'skills'];
  var EXTENSION_NOISE = ['extension', 'extensions'];
  var WORKFLOW_NOISE = ['workflow', 'workflows'];

  /* Below this many characters a term matches whole words only. */
  var MIN_PARTIAL_CHARS = 3;

  /* At or above this many characters a term may match anywhere inside a word,
     however long the word. Below it, `substantialInfix` asks for half. */
  var MIN_INFIX_CHARS = 4;

  /* Split at every character that is not a letter or digit — whitespace and
     punctuation alike, so `r-scripting` is `r` + `scripting` and `ggplot2`
     stays one word. The Unicode classes stand in for Rust's
     `char::is_alphanumeric`. */
  var NON_WORD = /[^\p{L}\p{N}]+/u;
  var WORD_CHAR = /[\p{L}\p{N}]/u;

  function words(text) {
    if (!text) return [];
    var out = [];
    var parts = String(text).split(NON_WORD);
    for (var i = 0; i < parts.length; i++) {
      if (parts[i]) out.push(parts[i].toLowerCase());
    }
    return out;
  }

  /* The distinct terms of `query`, in the order written, without filler. */
  function terms(query, noise) {
    var all = [];
    var every = words(query);
    for (var i = 0; i < every.length; i++) {
      if (all.indexOf(every[i]) === -1) all.push(every[i]);
    }
    var meaningful = all.filter(function (term) {
      return FILLER.indexOf(term) === -1 && (noise || []).indexOf(term) === -1;
    });
    return meaningful.length > 0 ? meaningful : all;
  }

  /* Is `term`, found inside `word` without touching its start, enough of that
     word to be a search rather than a morpheme?

     The matcher already grades an anchored match above an unanchored one — a
     prefix scores 2, an infix 1 — and MIN_PARTIAL_CHARS was the only admission
     gate, so three characters bought a match anywhere inside any word. Measured
     on this page against the live 37-entry extensions shelf: `lab` -> 37 of 37,
     `gen` -> 36, `age` -> 36, through an infix of `baranzinilab` in the org line
     and of `…Agent` in each card's own heading. So it is a rule, not a field:
     dropping the org line fixes one of the three and nothing can drop a heading.

     An unanchored match therefore needs either MIN_INFIX_CHARS characters or half
     the word it sits in. Two arms, because each closes a case the other gets
     wrong, and each was checked against the registry's own vocabulary. (The
     "807 words" this comment used to claim is not reproducible: measured
     2026-09-12 over exactly the searched fields, the catalog has 1,445 distinct
     words with the prose descriptions and 773 without. Quote a corpus size only
     with the field set that produces it.) A flat four-character floor loses `rna` inside `scRNA`/`rRNA`/`miRNA` and
     `sem` inside `RSEM`; a flat half-the-word ratio loses `omics` inside
     `transcriptomics` and `flow` inside `workflows`. Half is the proportion the
     infix rule's own documented case sits at — `heatmap` is 7 of
     `complexheatmap`'s 14.

     `substantial_infix` in catalog_search.rs; a change here is a change there and
     in ui/desktop/src/components/baam/search.ts. */
  function substantialInfix(term, word) {
    var termChars = Array.from(term).length;
    return termChars >= MIN_INFIX_CHARS || termChars * 2 >= Array.from(word).length;
  }

  /* How well `term` matches one field word: 3 for the whole word, 2 for its
     start, 1 for anywhere inside it (`heatmap` in `complexheatmap`), 0 for no
     match. A short term matches whole words only, and a term that is short
     relative to the word matches only at its start — see `substantialInfix`. */
  function strength(term, word) {
    if (word === term) return 3;
    if (Array.from(term).length < MIN_PARTIAL_CHARS) return 0;
    if (word.indexOf(term) === 0) return 2;
    if (word.indexOf(term) !== -1) return substantialInfix(term, word) ? 1 : 0;
    return 0;
  }

  /* `term`'s strength against `word`, falling back to its singular so
     `visualizations` finds `visualization`. `class` and `gis` are left alone. */
  function termStrength(term, word) {
    var direct = strength(term, word);
    if (direct > 0) return direct;
    if (term.charAt(term.length - 1) !== 's') return 0;
    var stem = term.slice(0, -1);
    if (Array.from(stem).length < MIN_PARTIAL_CHARS) return 0;
    if (stem.charAt(stem.length - 1) === 's') return 0;
    return strength(stem, word);
  }

  /* Whether `phrase` is WRITTEN IN `text`: present as a substring, and not
     buried inside a longer word at either end. The boundary is imposed only by
     an end that is itself a word character, so `++` is written in `c++ code`
     while `dy` is not written in `tidyverse`.

     ⚠ **Every character position is tried**, not just the first occurrence,
     because a refused occurrence can overlap an accepted one: `a a` is written
     in `ba a a` only from the second `a`.

     This is `written_in` in catalog_search.rs, which PR #266 introduced to
     replace the plain substring test the verbatim bonus used to do. Ported so
     `rank` keeps scoring the way the canonical matcher does; the shelves call
     `matching`, which discards rank, so nothing on the page moves either way. */
  function writtenIn(text, phrase) {
    var chars = Array.from(String(text == null ? '' : text).toLowerCase());
    var needle = Array.from(String(phrase == null ? '' : phrase));
    if (needle.length === 0) return false;
    var startsWord = WORD_CHAR.test(needle[0]);
    var endsWord = WORD_CHAR.test(needle[needle.length - 1]);
    for (var i = 0; i + needle.length <= chars.length; i++) {
      var hit = true;
      for (var j = 0; j < needle.length; j++) {
        if (chars[i + j] !== needle[j]) {
          hit = false;
          break;
        }
      }
      if (!hit) continue;
      var before = i > 0 ? chars[i - 1] : null;
      var after = i + needle.length < chars.length ? chars[i + needle.length] : null;
      var opens = !startsWord || !(before !== null && WORD_CHAR.test(before));
      var closes = !endsWord || !(after !== null && WORD_CHAR.test(after));
      if (opens && closes) return true;
    }
    return false;
  }

  /**
   * Rank `entries` against `query`.
   *
   * `fields(entry)` returns `[[text, weight], …]` — the text of one entry that
   * is searched, and how much a match there counts. An empty (or all-whitespace)
   * query is the browse case: every entry, in the order given.
   *
   * Returns `{ terms, hits: [{ entry, matchedTerms, score, verbatim }] }`, best
   * first; equal ranks keep the order they were given in.
   */
  function rank(query, noise, entries, fields) {
    var list = Array.prototype.slice.call(entries);
    var phrase = String(query == null ? '' : query).trim().toLowerCase();
    if (!phrase) {
      return {
        terms: [],
        hits: list.map(function (entry) {
          return { entry: entry, matchedTerms: [], score: 0, verbatim: false };
        })
      };
    }
    var queryTerms = terms(query, noise);
    var ranked = [];
    list.forEach(function (entry, index) {
      var entryFields = fields(entry) || [];
      var verbatim = entryFields.some(function (field) {
        return writtenIn(field[0], phrase);
      });
      var entryWords = [];
      entryFields.forEach(function (field) {
        var weight = field[1];
        words(field[0]).forEach(function (word) {
          entryWords.push([word, weight]);
        });
      });
      var matchedTerms = [];
      var score = 0;
      queryTerms.forEach(function (term) {
        var best = 0;
        for (var i = 0; i < entryWords.length; i++) {
          var value = termStrength(term, entryWords[i][0]) * entryWords[i][1];
          if (value > best) best = value;
        }
        if (best > 0) {
          matchedTerms.push(term);
          score += best;
        }
      });
      if (verbatim || matchedTerms.length > 0) {
        ranked.push({
          entry: entry,
          matchedTerms: matchedTerms,
          score: score,
          verbatim: verbatim,
          index: index
        });
      }
    });
    ranked.sort(function (left, right) {
      if (left.verbatim !== right.verbatim) return left.verbatim ? -1 : 1;
      if (left.matchedTerms.length !== right.matchedTerms.length) {
        return right.matchedTerms.length - left.matchedTerms.length;
      }
      if (left.score !== right.score) return right.score - left.score;
      return left.index - right.index;
    });
    return {
      terms: queryTerms,
      hits: ranked.map(function (hit) {
        return {
          entry: hit.entry,
          matchedTerms: hit.matchedTerms,
          score: hit.score,
          verbatim: hit.verbatim
        };
      })
    };
  }

  /**
   * The set of entries `query` matches — what a show/hide filter needs. `rank`
   * is the thing to reach for when the order matters.
   */
  function matching(query, noise, entries, fields) {
    var found = rank(query, noise, entries, fields);
    var set = new Set();
    found.hits.forEach(function (hit) {
      set.add(hit.entry);
    });
    return set;
  }

  return {
    Weight: Weight,
    FILLER: FILLER,
    SKILL_NOISE: SKILL_NOISE,
    EXTENSION_NOISE: EXTENSION_NOISE,
    WORKFLOW_NOISE: WORKFLOW_NOISE,
    words: words,
    terms: terms,
    writtenIn: writtenIn,
    substantialInfix: substantialInfix,
    rank: rank,
    matching: matching
  };
});
