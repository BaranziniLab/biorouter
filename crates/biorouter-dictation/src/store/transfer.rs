//! Import and export ("Import and export formats", DD-43).
//!
//! Export carries no ids, session ids or usage: written forms, spoken forms,
//! misrecognitions, kind, `match` and the note, and on a private row `"tier": "private"`
//! and its `affiliation`, so an import elsewhere restamps it private with the same
//! institutions. Only active rows are exported: a suggestion is waiting for the person,
//! and an import would make it active.
//!
//! Import reads one term per line (`EPAS1 = epas one, e pass one`, a `#` at the start
//! or after a space begins a comment), CSV
//! or TSV with the header `written,spoken_forms,note` (spoken forms separated by `;`),
//! and the export JSON. It merges and never deletes; each row's plan is `new`,
//! `merged`, `duplicate` or `invalid` with its code.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};

use super::ops::{AddOutcome, NewTerm, Reach};
use super::schema::{
    normalize_institution_id, EntrySource, EntryStatus, EntryTier, MatchPolicy, RowAffiliation,
    StoreFile, TermKind, EXPORT_FORMAT, EXPORT_VERSION, MAX_ROW_INSTITUTIONS,
};
use super::validate::{self, ValidationCode};
use super::StoreError;

/// The import formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    /// One term per line (`--format txt`).
    Lines,
    Csv,
    Tsv,
    /// The export JSON.
    Json,
}

impl ImportFormat {
    /// `txt` (or `lines`), `csv`, `tsv` or `json`.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "txt" | "lines" => Some(Self::Lines),
            "csv" => Some(Self::Csv),
            "tsv" => Some(Self::Tsv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The format a file's extension names.
    pub fn from_extension(path: &Path) -> Option<Self> {
        Self::parse(path.extension()?.to_str()?)
    }
}

/// A row's `affiliation` as the import file carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedAffiliation {
    /// `null`, or no key at all (a text or CSV line, a JSON row without it): no
    /// institution claims the row (O-36).
    Unclaimed,
    /// `{"institutions": [...]}` with 1 to 8 ids of 1 to 64 characters and no control
    /// character, normalized.
    Institutions(Vec<String>),
    /// Any other value: present but unreadable, stored as `{"institutions": []}`, which
    /// only local models clear (O-32), because `null` would reach every private target.
    Unreadable,
}

impl ImportedAffiliation {
    fn read(value: Option<&Value>) -> Self {
        let Some(value) = value else {
            return Self::Unclaimed;
        };
        if value.is_null() {
            return Self::Unclaimed;
        }
        let Some(object) = value.as_object() else {
            return Self::Unreadable;
        };
        let (Some(institutions), 1) = (object.get("institutions"), object.len()) else {
            return Self::Unreadable;
        };
        let Some(raw) = institutions.as_array() else {
            return Self::Unreadable;
        };
        let mut ids: Vec<String> = Vec::with_capacity(raw.len());
        for item in raw {
            let Some(id) = item.as_str().and_then(normalize_institution_id) else {
                return Self::Unreadable;
            };
            ids.push(id);
        }
        ids.sort();
        ids.dedup();
        if ids.is_empty() || ids.len() > MAX_ROW_INSTITUTIONS {
            return Self::Unreadable;
        }
        Self::Institutions(ids)
    }

    fn stored(&self) -> Option<RowAffiliation> {
        match self {
            Self::Unclaimed => None,
            Self::Institutions(ids) => Some(RowAffiliation {
                institutions: ids.clone(),
            }),
            Self::Unreadable => Some(RowAffiliation::unknown()),
        }
    }
}

/// One row of an import file.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRow {
    /// The 1 based line of a text, CSV or TSV file, or the position in a JSON `entries`.
    pub line: usize,
    pub written: String,
    pub spoken_forms: Vec<String>,
    pub misrecognitions: Vec<String>,
    pub kind: Option<TermKind>,
    pub match_policy: Option<MatchPolicy>,
    pub note: Option<String>,
    /// Marked `"tier": "private"`.
    pub private: bool,
    pub affiliation: ImportedAffiliation,
    /// A JSON row this version cannot read: planned `invalid` with code `unreadable`.
    pub unreadable: bool,
}

impl ImportRow {
    fn text(line: usize, written: &str, spoken_forms: Vec<String>, note: Option<String>) -> Self {
        Self {
            line,
            written: written.to_string(),
            spoken_forms,
            misrecognitions: Vec::new(),
            kind: None,
            match_policy: None,
            note,
            private: false,
            affiliation: ImportedAffiliation::Unclaimed,
            unreadable: false,
        }
    }
}

/// Why a whole import file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// JSON that is not a Biorouter dictionary export.
    NotAnExport,
    /// CSV or TSV the reader could not parse.
    Unparsable(String),
}

/// MSG-IMPORT-NOT-EXPORT.
pub const MSG_IMPORT_NOT_EXPORT: &str = "That file is not a Biorouter dictionary export.";
/// MSG-IMPORT-ROW-UNREADABLE: the sentence for a row planned `invalid` with code
/// `unreadable`.
pub const MSG_IMPORT_ROW_UNREADABLE: &str = "This row could not be read.";

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnExport => f.write_str(MSG_IMPORT_NOT_EXPORT),
            // MSG-IMPORT-UNPARSABLE.
            Self::Unparsable(detail) => write!(f, "That file could not be read: {detail}."),
        }
    }
}

impl std::error::Error for ImportError {}

fn split_forms(text: &str, separator: char) -> Vec<String> {
    text.split(separator)
        .map(str::trim)
        .filter(|form| !form.is_empty())
        .map(str::to_string)
        .collect()
}

/// A line's text before its comment. A `#` starts one at the start of the line or after
/// a space or a tab, so `C#` and `F#`, which `add` accepts, import as written instead of
/// losing their `#` and everything after it.
fn before_comment(line: &str) -> &str {
    let mut previous: Option<char> = None;
    for (at, c) in line.char_indices() {
        if c == '#' && previous.is_none_or(char::is_whitespace) {
            // `at` comes from `char_indices`, so it is always a boundary.
            return line.get(..at).unwrap_or(line);
        }
        previous = Some(c);
    }
    line
}

fn parse_lines(content: &str) -> Vec<ImportRow> {
    content
        .lines()
        .enumerate()
        .filter_map(|(i, raw)| {
            let line = before_comment(raw).trim();
            if line.is_empty() {
                return None;
            }
            let (written, forms) = match line.split_once('=') {
                Some((written, forms)) => (written.trim(), split_forms(forms, ',')),
                None => (line, Vec::new()),
            };
            Some(ImportRow::text(i + 1, written, forms, None))
        })
        .collect()
}

fn parse_delimited(content: &str, delimiter: u8) -> Result<Vec<ImportRow>, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(content.as_bytes());
    let mut columns = (0usize, Some(1usize), Some(2usize));
    let mut rows = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|e| ImportError::Unparsable(e.to_string()))?;
        let line = record
            .position()
            .map_or(index + 1, |position| position.line() as usize);
        let cells: Vec<String> = record.iter().map(|cell| cell.trim().to_string()).collect();
        if index == 0
            && cells
                .iter()
                .any(|cell| cell.eq_ignore_ascii_case("written"))
        {
            let find = |name: &str| cells.iter().position(|c| c.eq_ignore_ascii_case(name));
            columns = (
                find("written").unwrap_or(0),
                find("spoken_forms"),
                find("note"),
            );
            continue;
        }
        let cell = |at: Option<usize>| at.and_then(|i| cells.get(i)).cloned().unwrap_or_default();
        let written = cell(Some(columns.0));
        if written.is_empty() && cells.iter().all(String::is_empty) {
            continue;
        }
        let note = Some(cell(columns.2)).filter(|n| !n.is_empty());
        rows.push(ImportRow::text(
            line,
            &written,
            split_forms(&cell(columns.1), ';'),
            note,
        ));
    }
    Ok(rows)
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    match value {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(_) => None,
    }
}

fn parse_json(content: &str) -> Result<Vec<ImportRow>, ImportError> {
    let value: Value = serde_json::from_str(content).map_err(|_| ImportError::NotAnExport)?;
    let is_export = value.get("format").and_then(Value::as_str) == Some(EXPORT_FORMAT)
        && value.get("version").and_then(Value::as_u64) == Some(u64::from(EXPORT_VERSION));
    let Some(entries) = value
        .get("entries")
        .and_then(Value::as_array)
        .filter(|_| is_export)
    else {
        return Err(ImportError::NotAnExport);
    };
    Ok(entries
        .iter()
        .enumerate()
        .map(|(i, entry)| json_row(i + 1, entry))
        .collect())
}

fn json_row(line: usize, entry: &Value) -> ImportRow {
    let unreadable = ImportRow {
        unreadable: true,
        ..ImportRow::text(line, "", Vec::new(), None)
    };
    let Some(object) = entry.as_object() else {
        return unreadable;
    };
    let tier = object.get("tier");
    let private = match tier.and_then(Value::as_str) {
        Some("private") => true,
        Some("public") => false,
        None if tier.is_none_or(Value::is_null) => false,
        _ => return unreadable,
    };
    let (Some(written), Some(spoken_forms), Some(misrecognitions)) = (
        object.get("written").and_then(Value::as_str),
        strings(object.get("spoken_forms")),
        strings(object.get("misrecognitions")),
    ) else {
        return unreadable;
    };
    let note = match object.get("note") {
        None | Some(Value::Null) => None,
        Some(Value::String(note)) => Some(note.clone()),
        Some(_) => return unreadable,
    };
    ImportRow {
        line,
        written: written.to_string(),
        spoken_forms,
        misrecognitions,
        kind: object
            .get("kind")
            .and_then(Value::as_str)
            .and_then(TermKind::parse),
        match_policy: object
            .get("match")
            .and_then(Value::as_str)
            .and_then(MatchPolicy::parse),
        note,
        private,
        affiliation: if private {
            ImportedAffiliation::read(object.get("affiliation"))
        } else {
            ImportedAffiliation::Unclaimed
        },
        unreadable: false,
    }
}

/// Read an import file in `format`.
pub fn parse_import(format: ImportFormat, content: &str) -> Result<Vec<ImportRow>, ImportError> {
    match format {
        ImportFormat::Lines => Ok(parse_lines(content)),
        ImportFormat::Csv => parse_delimited(content, b','),
        ImportFormat::Tsv => parse_delimited(content, b'\t'),
        ImportFormat::Json => parse_json(content),
    }
}

/// Whether `content` is the export JSON, so standard input needs no `--format`.
pub fn looks_like_export(content: &str) -> bool {
    content.trim_start().starts_with('{') && parse_json(content).is_ok()
}

/// What an import did, or would do, with one row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "plan", rename_all = "snake_case")]
pub enum RowOutcome {
    New,
    /// New spoken forms or misrecognitions went into the existing row.
    Merged,
    /// The row exists with everything this one carries.
    Duplicate,
    /// Refused; `code` is a validation code, `cap`, or `unreadable`.
    Invalid {
        code: &'static str,
    },
}

/// One row's plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedRow {
    pub line: usize,
    pub tier: EntryTier,
    #[serde(flatten)]
    pub outcome: RowOutcome,
}

/// Every row's plan, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportPlan {
    pub rows: Vec<PlannedRow>,
}

impl ImportPlan {
    /// How many rows had `outcome`'s kind, counting only rows of a tier the caller may
    /// list (oracle rule 3).
    pub fn count(&self, reach: Reach, which: fn(&RowOutcome) -> bool) -> usize {
        self.rows
            .iter()
            .filter(|row| reach.lists(row.tier) && which(&row.outcome))
            .count()
    }
}

fn invalid(code: ValidationCode) -> RowOutcome {
    RowOutcome::Invalid {
        code: code.as_str(),
    }
}

impl StoreFile {
    /// Merge `rows` into the store. Rows marked private import private; the rest take
    /// `default_tier`, the importer's stamp. `is_secret` is the core's secret scan, which
    /// the leaf crate cannot call. A person's act: a tombstone for an imported term is
    /// cleared, and an imported misrecognition starts at count 2.
    pub fn import(
        &mut self,
        rows: Vec<ImportRow>,
        default_tier: EntryTier,
        is_secret: &dyn Fn(&str) -> bool,
        at: DateTime<Utc>,
    ) -> ImportPlan {
        let mut plan = ImportPlan::default();
        for row in rows {
            let tier = if row.private {
                EntryTier::Private
            } else {
                default_tier
            };
            let outcome = self.import_row(&row, tier, is_secret, at);
            plan.rows.push(PlannedRow {
                line: row.line,
                tier,
                outcome,
            });
        }
        plan
    }

    fn import_row(
        &mut self,
        row: &ImportRow,
        tier: EntryTier,
        is_secret: &dyn Fn(&str) -> bool,
        at: DateTime<Utc>,
    ) -> RowOutcome {
        if row.unreadable {
            return RowOutcome::Invalid { code: "unreadable" };
        }
        let checked = (|| {
            let written = validate::term(&row.written)?;
            let spoken = validate::terms(&row.spoken_forms, validate::MAX_SPOKEN_FORMS)?;
            let heard = validate::terms(&row.misrecognitions, validate::MAX_MISRECOGNITIONS)?;
            let mut every = std::iter::once(&written).chain(&spoken).chain(&heard);
            if every.any(|text| is_secret(text)) {
                return Err(ValidationCode::SecretShaped);
            }
            Ok((written, spoken, heard))
        })();
        let (written, spoken, heard) = match checked {
            Ok(fields) => fields,
            Err(code) => return invalid(code),
        };
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.tier == tier && e.written == written)
        {
            return if self.merge_forms(index, &spoken, &heard, at) {
                RowOutcome::Merged
            } else {
                RowOutcome::Duplicate
            };
        }
        let term = NewTerm {
            source: EntrySource::Import,
            status: EntryStatus::Active,
            spoken_forms: spoken,
            misrecognitions: heard,
            kind: row.kind,
            match_policy: row.match_policy,
            note: row.note.clone(),
            affiliation: row.affiliation.stored(),
            ..NewTerm::person(written, tier)
        };
        match self.add_with(term, None, at) {
            Ok(AddOutcome::Created { .. }) => RowOutcome::New,
            Ok(AddOutcome::Exists { .. } | AddOutcome::Skipped) => RowOutcome::Duplicate,
            Err(StoreError::Invalid(code)) => invalid(code),
            Err(StoreError::Cap(_)) => RowOutcome::Invalid { code: "cap" },
            Err(_) => RowOutcome::Invalid { code: "unreadable" },
        }
    }

    /// The export JSON for the active rows the caller includes: public rows, and private
    /// rows only when `reach` is [`Reach::All`] (the person chose to include them after
    /// MSG-EXPORT-PRIVATE).
    pub fn export(&self, reach: Reach, at: DateTime<Utc>) -> Value {
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.status == EntryStatus::Active && reach.lists(e.tier))
            .collect();
        rows.sort_by(|a, b| a.written.cmp(&b.written).then(a.tier.cmp(&b.tier)));
        let entries: Vec<Value> = rows
            .into_iter()
            .map(|entry| {
                let mut row = Map::new();
                row.insert("written".into(), json!(entry.written));
                row.insert("spoken_forms".into(), json!(entry.spoken_forms));
                let heard: Vec<&str> = entry
                    .misrecognitions
                    .iter()
                    .map(|m| m.text.as_str())
                    .collect();
                row.insert("misrecognitions".into(), json!(heard));
                row.insert("kind".into(), json!(entry.kind));
                row.insert("match".into(), json!(entry.match_policy));
                row.insert("note".into(), json!(entry.note));
                if entry.tier.is_private() {
                    row.insert("tier".into(), json!("private"));
                    row.insert("affiliation".into(), json!(entry.affiliation));
                }
                Value::Object(row)
            })
            .collect();
        json!({
            "format": EXPORT_FORMAT,
            "version": EXPORT_VERSION,
            "exported_at": at,
            "entries": entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::now;

    fn never_secret(_: &str) -> bool {
        false
    }

    fn outcomes(plan: &ImportPlan) -> Vec<RowOutcome> {
        plan.rows.iter().map(|row| row.outcome.clone()).collect()
    }

    #[test]
    fn lines_with_spoken_forms_and_comments() {
        let rows = parse_import(
            ImportFormat::Lines,
            "# my terms\nUCSF\nEPAS1 = epas one, e pass one\n\n  PHD2  # a gene\n",
        )
        .unwrap();
        let summary: Vec<(usize, &str, Vec<String>)> = rows
            .iter()
            .map(|r| (r.line, r.written.as_str(), r.spoken_forms.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (2, "UCSF", vec![]),
                (3, "EPAS1", vec!["epas one".into(), "e pass one".into()]),
                (5, "PHD2", vec![]),
            ]
        );
        // A `#` inside a term is part of it; one after a space starts a comment.
        let rows = parse_import(
            ImportFormat::Lines,
            "C# = see sharp # a language\nF#\n#C\n\tVHL\t# a gene\n",
        )
        .unwrap();
        let summary: Vec<(&str, Vec<String>)> = rows
            .iter()
            .map(|r| (r.written.as_str(), r.spoken_forms.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("C#", vec!["see sharp".to_string()]),
                ("F#", vec![]),
                ("VHL", vec![]),
            ]
        );
    }

    #[test]
    fn csv_and_tsv_with_the_header() {
        let csv =
            "written,spoken_forms,note\nEPAS1,epas one;e pass one,\"a gene, human\"\nUCSF,,\n";
        let rows = parse_import(ImportFormat::Csv, csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].spoken_forms, vec!["epas one", "e pass one"]);
        assert_eq!(rows[0].note.as_deref(), Some("a gene, human"));
        assert_eq!((rows[1].line, rows[1].note.clone()), (3, None));
        let tsv = "written\tspoken_forms\tnote\nEPAS1\tepas one\t\n";
        let rows = parse_import(ImportFormat::Tsv, tsv).unwrap();
        assert_eq!(rows[0].spoken_forms, vec!["epas one"]);
        let headless = parse_import(ImportFormat::Csv, "EPAS1,epas one\n").unwrap();
        assert_eq!(headless[0].written, "EPAS1");
    }

    /// An import never deletes; rows that exist merge their forms or are duplicates, and
    /// each refused row names its line and code.
    #[test]
    fn the_plan_new_merged_duplicate_and_invalid() {
        let mut file = StoreFile::default();
        let first = parse_import(ImportFormat::Lines, "EPAS1 = epas one\nUCSF\n").unwrap();
        let plan = file.import(first, EntryTier::Public, &never_secret, now());
        assert_eq!(outcomes(&plan), vec![RowOutcome::New, RowOutcome::New]);
        let second = parse_import(
            ImportFormat::Lines,
            "EPAS1 = e pass one\nUCSF\nhttps://example.org\nsk-proj-abcdefghij\nfine term\n",
        )
        .unwrap();
        let plan = file.import(
            second,
            EntryTier::Public,
            &|t: &str| t == "fine term",
            now(),
        );
        assert_eq!(
            outcomes(&plan),
            vec![
                RowOutcome::Merged,
                RowOutcome::Duplicate,
                RowOutcome::Invalid { code: "url" },
                RowOutcome::Invalid {
                    code: "secret_shaped"
                },
                RowOutcome::Invalid {
                    code: "secret_shaped"
                },
            ]
        );
        assert_eq!(plan.rows[2].line, 3);
        assert_eq!(file.entries.len(), 2);
        let epas = file.rows_written("EPAS1", Reach::All)[0];
        assert_eq!(epas.spoken_forms, vec!["epas one", "e pass one"]);
        assert_eq!(epas.source, EntrySource::Import);
    }

    /// DD-43, O-32, O-36: a private row keeps a well formed affiliation, a row without one
    /// stores `null`, and any other value stores `[]`; export carries it.
    #[test]
    fn affiliations_round_trip_through_export_and_import() {
        let export = r#"{"format": "biorouter.dictionary", "version": 1, "exported_at": "2026-11-04T18:00:00Z",
          "entries": [
            {"written": "EPAS1", "spoken_forms": ["epas one"], "misrecognitions": ["a pass one"], "kind": "gene_symbol", "match": "exact", "note": null},
            {"written": "PHD2", "tier": "private", "affiliation": null},
            {"written": "UCSF", "tier": "private", "affiliation": {"institutions": [" UCSF ", "stanford"]}},
            {"written": "SPOKE", "tier": "private"},
            {"written": "VHL", "tier": "private", "affiliation": {"institutions": []}},
            {"written": "ARNT", "tier": "private", "affiliation": "ucsf"},
            {"written": "EGLN1", "tier": "private", "affiliation": {"institutions": ["a","b","c","d","e","f","g","h","i"]}},
            {"written": "HIF1A", "tier": "private", "affiliation": {"institutions": ["ucsf"], "extra": 1}},
            {"written": "NOTCH1", "affiliation": {"institutions": ["ucsf"]}},
            {"written": "BAD", "tier": "secret"},
            "not an object"
          ]}"#;
        assert!(looks_like_export(export));
        let rows = parse_import(ImportFormat::Json, export).unwrap();
        let mut file = StoreFile::default();
        let plan = file.import(rows, EntryTier::Public, &never_secret, now());
        assert_eq!(
            plan.rows
                .iter()
                .filter(|r| r.outcome == RowOutcome::New)
                .count(),
            9
        );
        assert_eq!(
            outcomes(&plan)[9..],
            [
                RowOutcome::Invalid { code: "unreadable" },
                RowOutcome::Invalid { code: "unreadable" }
            ]
        );
        let affiliation = |w: &str| {
            let rows = file.rows_written(w, Reach::All);
            assert_eq!(rows.len(), 1, "{w}");
            (rows[0].tier, rows[0].affiliation.clone())
        };
        let ids = |ids: &[&str]| {
            Some(RowAffiliation {
                institutions: ids.iter().map(|s| s.to_string()).collect(),
            })
        };
        assert_eq!(affiliation("EPAS1"), (EntryTier::Public, None));
        assert_eq!(affiliation("PHD2"), (EntryTier::Private, None));
        assert_eq!(
            affiliation("UCSF"),
            (EntryTier::Private, ids(&["stanford", "ucsf"]))
        );
        assert_eq!(affiliation("SPOKE"), (EntryTier::Private, None));
        assert_eq!(affiliation("VHL"), (EntryTier::Private, ids(&[])));
        assert_eq!(affiliation("ARNT"), (EntryTier::Private, ids(&[])));
        assert_eq!(affiliation("EGLN1"), (EntryTier::Private, ids(&[])));
        assert_eq!(affiliation("HIF1A"), (EntryTier::Private, ids(&[])));
        assert_eq!(
            affiliation("NOTCH1"),
            (EntryTier::Public, None),
            "public rows carry none"
        );
        let epas = file.rows_written("EPAS1", Reach::All)[0];
        assert_eq!(epas.misrecognitions[0].count, 2, "the person supplied it");

        let public_only = file.export(Reach::Public, now());
        let written: Vec<&str> = public_only["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["written"].as_str().unwrap())
            .collect();
        assert_eq!(written, vec!["EPAS1", "NOTCH1"]);
        let everything = file.export(Reach::All, now());
        let ucsf = everything["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["written"] == "UCSF")
            .unwrap();
        assert_eq!(ucsf["tier"], "private");
        assert_eq!(
            ucsf["affiliation"],
            json!({"institutions": ["stanford", "ucsf"]})
        );
        let phd2 = everything["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["written"] == "PHD2")
            .unwrap();
        assert_eq!(phd2["affiliation"], Value::Null);
        assert!(phd2.as_object().unwrap().contains_key("affiliation"));
        for entry in everything["entries"].as_array().unwrap() {
            for key in ["id", "rev", "suggested_in", "uses", "seen"] {
                assert!(entry.get(key).is_none(), "export carries no {key}");
            }
        }

        // The export imports elsewhere with the same tiers and institutions.
        let mut elsewhere = StoreFile::default();
        let again = parse_import(ImportFormat::Json, &everything.to_string()).unwrap();
        elsewhere.import(again, EntryTier::Public, &never_secret, now());
        let ucsf = elsewhere.rows_written("UCSF", Reach::All)[0];
        assert_eq!(ucsf.affiliation, ids(&["stanford", "ucsf"]));
    }

    /// An imported row refused at the active cap is planned `cap` and the rest of the
    /// import is still written, so the person's tombstone for that term must survive: it
    /// still stops the agent, the extractor and the learner.
    #[test]
    fn a_term_refused_at_the_cap_keeps_its_tombstone() {
        let mut file = StoreFile::default();
        let first = parse_import(ImportFormat::Lines, "SPOKE\n").unwrap();
        file.import(first, EntryTier::Public, &never_secret, now());
        let id = file.rows_written("SPOKE", Reach::All)[0].id.clone();
        file.remove(&id, now()).unwrap();
        let fill: String = (0..crate::store::ops::MAX_ACTIVE_PER_TIER)
            .map(|i| format!("TERM{i}\n"))
            .collect();
        let rows = parse_import(ImportFormat::Lines, &fill).unwrap();
        file.import(rows, EntryTier::Public, &never_secret, now());
        let again = parse_import(ImportFormat::Lines, "SPOKE\n").unwrap();
        let plan = file.import(again, EntryTier::Public, &never_secret, now());
        assert_eq!(outcomes(&plan), vec![RowOutcome::Invalid { code: "cap" }]);
        assert!(file.rows_written("SPOKE", Reach::All).is_empty());
        assert!(file.tombstoned("spoke", EntryTier::Public));
    }

    #[test]
    fn json_that_is_not_an_export_is_refused_whole() {
        for content in [
            "",
            "[]",
            "{}",
            r#"{"format": "other", "version": 1, "entries": []}"#,
        ] {
            assert_eq!(
                parse_import(ImportFormat::Json, content),
                Err(ImportError::NotAnExport),
                "{content}"
            );
        }
        assert!(!looks_like_export("EPAS1\nUCSF\n"));
        assert_eq!(
            ImportFormat::from_extension(Path::new("terms.TSV")),
            Some(ImportFormat::Tsv)
        );
        assert_eq!(
            ImportFormat::from_extension(Path::new("terms.txt")),
            Some(ImportFormat::Lines)
        );
        assert_eq!(ImportFormat::from_extension(Path::new("terms")), None);
    }
}
