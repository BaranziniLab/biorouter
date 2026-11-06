//! Tier 1 accuracy (`docs/dictation/testing-and-release.md`, "Tier 1: networked nightly
//! accuracy"), run by `dictation-accuracy.yml` and by hand. The two tests below run only
//! with `--ignored`: they need the real speech models, and the PriMock57 evaluation set
//! for the second. `the_scoring_helpers` runs offline every time.
//!
//! * `jfk_with_each_catalog_model`: the words of `jfk.wav` through the real service and
//!   worker, with each catalog model;
//! * `primock57_baseline`: the evaluation set `eval-primock57-subset-r1` (60 utterances
//!   naming 26 drugs 85 times, and 16 gaps where the recorded speaker is silent) with no
//!   dictionary, the baseline arm of PR-12, on the clean clips and on SNR 10 and reverb
//!   copies made here with a fixed seed. It reports strict recall, collapses, word error
//!   rates and the time per clip, writes the transcripts and per term tables, and holds
//!   the baseline floors: no output on the gaps (turbo at most 1 of 16) and no collapse on
//!   any condition.
//! * `primock57_dictionary` (PR-14): the dictionary arm, the shipped mechanism with the
//!   set's 26 terms as the person's dictionary, built through `bias_set` like every other
//!   caller. It reports strict recall beside the baseline's and holds the same floors.
//! * `m22_arms` (PR-14): M-22's arms (`testing-and-release.md`, "The M-22 decision and its
//!   fallback"): no dictionary, the token prompt, the decoder bonus and both, each with
//!   correction and without, on the real sets (PriMock57, and the Fareez et al. 2022
//!   subset when `BIOROUTER_DICTATION_FAREEZ_DIR` names one) and the synthetic set
//!   (`BIOROUTER_DICTATION_SYNTHETIC_DIR`), plus `temperature_inc` 0 and, for small.en,
//!   timestamps off. Every arm is built from one eligible set whose ranking is fixed by
//!   the term lists alone, so every arm that carries a prompt sends the same 30 terms,
//!   which the run prints and records. It computes the DD-44 rules that were fixed before
//!   the numbers (rule 1 for the channels, rule 2 for `temperature_inc` and timestamps)
//!   and writes both decisions; it asserts nothing about which channel wins.
//!
//! Inputs, read only by this test:
//! * `BIOROUTER_DICTATION_MODELS_DIR`: the folder holding the catalog models; a model
//!   missing from it is installed there through the product's own installer, so a cache
//!   miss in CI also exercises the download path with the compiled pins;
//! * `BIOROUTER_DICTATION_EVALSET`: the evaluation tarball, checked against its pin;
//! * `BIOROUTER_DICTATION_ACCURACY_MODELS`: the models to run, comma separated (both
//!   catalog models when unset);
//! * `BIOROUTER_DICTATION_ACCURACY_OUT`: where the tables and transcripts go.
//!
//! Measured accuracy figures go to this test's output and the workflow's artifacts only,
//! never into user facing documents (O-23). The recordings are PriMock57 (CC BY 4.0) and
//! the vendored JFK sample (O-38).
//!
//! This test binary is its own dictation worker, so it has its own `main()` in place of
//! libtest's (Cargo's `harness = false`): `main()` answers the worker's hidden first
//! argument first, as it does in `biorouter` and `biorouterd`, then runs the tests. It
//! takes libtest's `--ignored`, `--include-ignored` and name filters; other flags are
//! accepted and have no effect.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use biorouter::dictation::catalog::{SttCatalogEntry, CATALOG, SMALL_EN_ID, TURBO_ID};
use biorouter::dictation::models::{self, InstallOptions, Network, WriteMode};
use biorouter::dictation::service::{self, TranscribeRequest};
use biorouter::dictation::{
    audio, bias_set, BiasChannels, BiasTarget, DictationError, DictionaryRequest,
    RecordingTranscript, Standing,
};
use biorouter_dictation::protocol::DecodeOptions;
use biorouter_dictation::recognize::EligibleEntries;
use biorouter_dictation::store::{DictionaryStore, EntryTier, NewTerm, Reach};
use serde_json::{json, Value};

type Test = (
    &'static str,
    Option<&'static str>,
    fn(&tokio::runtime::Runtime),
);

const TESTS: &[Test] = &[
    ("the_scoring_helpers", None, |_| the_scoring_helpers()),
    (
        "jfk_with_each_catalog_model",
        Some("needs the real speech models"),
        |rt| rt.block_on(jfk_with_each_catalog_model()),
    ),
    (
        "primock57_baseline",
        Some("needs the real speech models and the evaluation set"),
        |rt| rt.block_on(primock57_baseline()),
    ),
    (
        "primock57_dictionary",
        Some("needs the real speech models and the evaluation set"),
        |rt| rt.block_on(primock57_dictionary()),
    ),
    (
        "m22_arms",
        Some("needs the real speech models, the evaluation set and the synthetic set"),
        |rt| rt.block_on(m22_arms()),
    ),
];

fn main() -> ExitCode {
    biorouter_dictation::worker::run_if_invoked();
    // The test process reads configuration and writes verification records under a root
    // of its own, never the developer's. Set before any thread starts.
    let root = std::env::temp_dir().join(format!("biorouter-accuracy-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    std::env::set_var("BIOROUTER_PATH_ROOT", &root);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let only_ignored = args.iter().any(|a| a == "--ignored");
    let with_ignored = only_ignored || args.iter().any(|a| a == "--include-ignored");
    let mut filters: Vec<&str> = Vec::new();
    let mut skip_value = false;
    for arg in &args {
        if skip_value {
            skip_value = false;
        } else if matches!(
            arg.as_str(),
            "--test-threads" | "--skip" | "--format" | "--color" | "--logfile"
        ) {
            skip_value = true;
        } else if !arg.starts_with('-') {
            filters.push(arg);
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("a runtime");
    let (mut passed, mut failed, mut ignored) = (0, Vec::new(), 0);
    for &(name, ignore, test) in TESTS {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f)) {
            continue;
        }
        let selected = match ignore {
            Some(_) => with_ignored,
            None => !only_ignored,
        };
        if !selected {
            println!("test {name} ... ignored, {}", ignore.unwrap_or_default());
            ignored += 1;
            continue;
        }
        println!("test {name} ...");
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(&runtime))) {
            Ok(()) => {
                println!("test {name} ... ok");
                passed += 1;
            }
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(name);
            }
        }
    }
    println!(
        "\ntest result: {}. {passed} passed; {} failed; {ignored} ignored",
        if failed.is_empty() { "ok" } else { "FAILED" },
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

const EVALSET_ENV: &str = "BIOROUTER_DICTATION_EVALSET";
/// `primock57-subset-r1.tar` of the immutable release `eval-primock57-subset-r1`.
const EVALSET_SHA256: &str = "2776a659f659181f9a4aa67c98d47bf531ad0c904623f9e77d075a714af599e1";
const MODELS_ENV: &str = "BIOROUTER_DICTATION_ACCURACY_MODELS";
const OUT_ENV: &str = "BIOROUTER_DICTATION_ACCURACY_OUT";
const CPU_ENV: &str = "BIOROUTER_DICTATION_ACCURACY_CPU";
const JFK_TEXT: &str = "And so my fellow Americans, ask not what your country can do for you, ask what you can do for your country.";

/// Disfluencies the reference transcripts carry and Whisper leaves out, dropped from both
/// sides before words are compared.
const FILLERS: &[&str] = &["uh", "um", "er", "erm", "ah", "hmm", "mm", "mhm", "uhm"];

fn selected_models() -> Vec<&'static SttCatalogEntry> {
    let ids: Vec<String> = match std::env::var(MODELS_ENV) {
        Ok(list) if !list.trim().is_empty() => {
            list.split(',').map(|id| id.trim().to_string()).collect()
        }
        _ => vec![SMALL_EN_ID.to_string(), TURBO_ID.to_string()],
    };
    ids.iter()
        .map(|id| {
            CATALOG
                .entry(id)
                .unwrap_or_else(|| panic!("{id} is not a catalog model"))
        })
        .collect()
}

/// The load spec for `entry`, installing it into `BIOROUTER_DICTATION_MODELS_DIR` with the
/// product's installer when it is not there yet. `BIOROUTER_DICTATION_ACCURACY_CPU=1`
/// keeps a Metal host on the CPU, to see what the hosted runners will.
async fn spec_for(entry: &'static SttCatalogEntry) -> biorouter::dictation::LoadSpec {
    let mut spec = resolve(entry).await;
    if std::env::var_os(CPU_ENV).is_some_and(|v| v == "1") {
        spec.use_gpu = false;
    }
    spec
}

async fn resolve(entry: &'static SttCatalogEntry) -> biorouter::dictation::LoadSpec {
    let svc = service::service();
    match svc.resolve_for(Some(entry.id), Some("en")).await {
        Ok(spec) => spec,
        Err(DictationError::NotInstalled { .. } | DictationError::DetectorNotInstalled { .. }) => {
            let dir = std::env::var_os(biorouter::dictation::paths::MODELS_DIR_ENV)
                .map(PathBuf::from)
                .expect("set BIOROUTER_DICTATION_MODELS_DIR to the folder for the catalog models");
            let detector = CATALOG
                .entry(entry.requires[0])
                .expect("the model's detector");
            println!("installing {} into {}", entry.id, dir.display());
            models::install_files(
                &dir,
                &[detector, entry],
                &InstallOptions::new(Network::production(), WriteMode::User),
            )
            .await
            .unwrap_or_else(|e| panic!("installing {}: {e}", entry.id));
            svc.resolve_for(Some(entry.id), Some("en"))
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", entry.id))
        }
        Err(e) => panic!("{}: {e}", entry.id),
    }
}

/// The words of `text`, case folded, punctuation and fillers left out.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty() && !FILLERS.contains(w))
        .map(str::to_string)
        .collect()
}

/// Word level edit distance.
fn edits(reference: &[String], output: &[String]) -> usize {
    let mut row: Vec<usize> = (0..=output.len()).collect();
    for (i, r) in reference.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, o) in output.iter().enumerate() {
            let substitute = previous + usize::from(r != o);
            previous = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[output.len()]
}

/// How often `term` occurs in `text` as a whole word, case ignored (the evaluation set's
/// own rule in `terms.json`).
fn occurrences(text: &str, term: &str) -> usize {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let term: Vec<char> = term.to_lowercase().chars().collect();
    if term.is_empty() || text.len() < term.len() {
        return 0;
    }
    (0..=text.len() - term.len())
        .filter(|&i| {
            text[i..i + term.len()] == term[..]
                && !i.checked_sub(1).is_some_and(|p| text[p].is_alphanumeric())
                && !text
                    .get(i + term.len())
                    .is_some_and(|c| c.is_alphanumeric())
        })
        .count()
}

/// 16 kHz mono samples of a FLAC or WAV file.
fn read_audio(path: &Path) -> Vec<i16> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::io::MediaSourceStream;
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = symphonia::core::probe::Hint::new();
    hint.with_extension("flac");
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &Default::default(), &Default::default())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut format = probed.format;
    let track = format.default_track().expect("a track").clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("a FLAC decoder");
    let mut mono: Vec<f32> = Vec::new();
    let mut rate = 16_000;
    while let Ok(packet) = format.next_packet() {
        let Ok(decoded) = decoder.decode(&packet) else {
            continue;
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        mono.extend(
            buffer
                .samples()
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
    }
    if rate != 16_000 {
        mono = audio::resample_to_16k(&mono, rate);
    }
    audio::f32_to_s16(&mono)
}

/// A fixed sequence for the degraded copies.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn gaussian(&mut self) -> f64 {
        let (u, v) = (self.next().max(1e-12), self.next());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

fn seed_of(name: &str) -> u64 {
    name.bytes()
        .fold(0x9E37_79B9_7F4A_7C15u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3)
        })
        .max(1)
}

/// White noise at 10 dB below the clip's own power.
fn with_noise(pcm: &[i16], name: &str) -> Vec<i16> {
    let power = pcm.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / pcm.len().max(1) as f64;
    let sigma = (power / 10.0).sqrt();
    let mut rng = Seeded(seed_of(name));
    pcm.iter()
        .map(|&s| (f64::from(s) + sigma * rng.gaussian()).clamp(-32_768.0, 32_767.0) as i16)
        .collect()
}

/// A room: the direct path and 400 reflections between 5 and 300 ms that decay with an
/// RT60 of 0.4 s, scaled back to the clip's peak.
fn with_reverb(pcm: &[i16], name: &str) -> Vec<i16> {
    let mut rng = Seeded(seed_of(name) ^ 0xA5A5);
    let mut taps: Vec<(usize, f64)> = vec![(0, 1.0)];
    for _ in 0..400 {
        let delay = 0.005 + 0.295 * rng.next();
        let gain = 0.25 * rng.gaussian() * (-6.91 * delay / 0.4).exp();
        taps.push(((delay * 16_000.0) as usize, gain));
    }
    let mut out = vec![0f64; pcm.len()];
    for &(delay, gain) in &taps {
        for (i, &s) in pcm.iter().enumerate() {
            if let Some(slot) = out.get_mut(i + delay) {
                *slot += gain * f64::from(s);
            }
        }
    }
    let peak_in = pcm
        .iter()
        .map(|s| s.unsigned_abs())
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let peak_out = out.iter().fold(1.0f64, |m, s| m.max(s.abs()));
    out.iter()
        .map(|s| (s * peak_in / peak_out).round().clamp(-32_768.0, 32_767.0) as i16)
        .collect()
}

async fn transcribe(
    spec: &biorouter::dictation::LoadSpec,
    pcm: Vec<i16>,
) -> (RecordingTranscript, u64) {
    let started = Instant::now();
    let transcript = service::service()
        .transcribe_recording_with(spec.clone(), Arc::from(pcm), TranscribeRequest::new("en"))
        .await
        .expect("a transcript")
        .transcript;
    (transcript, started.elapsed().as_millis() as u64)
}

fn out_dir() -> PathBuf {
    let dir = std::env::var_os(OUT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("biorouter-dictation-accuracy"));
    std::fs::create_dir_all(&dir).expect("the output folder");
    dir
}

/// The words of `jfk.wav` with each catalog model. The models agree on the words and
/// not on the commas, so punctuation and case are not compared.
async fn jfk_with_each_catalog_model() {
    let jfk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source/samples/jfk.wav");
    let bytes = std::fs::read(&jfk).expect("jfk.wav");
    let pcm = audio::parse_wav_s16(&bytes)
        .expect("16 bit PCM")
        .to_16k_mono();
    for entry in selected_models() {
        let spec = spec_for(entry).await;
        let loaded = service::service()
            .prepare_with(spec.clone())
            .await
            .expect("loads");
        let (transcript, ms) = transcribe(&spec, pcm.clone()).await;
        println!(
            "{} on {}: {ms} ms: {}",
            entry.id, loaded.backend, transcript.text
        );
        assert_eq!(words(&transcript.text), words(JFK_TEXT), "{}", entry.id);
        service::service().unload().await;
    }
}

struct Clip {
    name: String,
    set: String,
    category: Option<String>,
    text: String,
    terms: BTreeMap<String, usize>,
    pcm: Vec<i16>,
}

fn evaluation_set() -> (Vec<Clip>, Vec<String>, tempfile::TempDir) {
    let tarball = std::env::var_os(EVALSET_ENV)
        .map(PathBuf::from)
        .expect("set BIOROUTER_DICTATION_EVALSET to primock57-subset-r1.tar");
    let digest = biorouter_dictation::build_info::sha256_file(&tarball).expect("the tarball");
    assert_eq!(
        digest, EVALSET_SHA256,
        "the evaluation set is not the pinned one"
    );
    let dir = tempfile::tempdir().expect("a folder");
    tar::Archive::new(std::fs::File::open(&tarball).expect("the tarball"))
        .unpack(dir.path())
        .expect("unpacks");
    let base = dir.path().join("primock57-subset-r1");
    let terms: Value =
        serde_json::from_str(&std::fs::read_to_string(base.join("terms.json")).unwrap()).unwrap();
    let terms: Vec<String> = terms["terms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap().to_string())
        .collect();
    let manifest = std::fs::read_to_string(base.join("manifest.jsonl")).unwrap();
    let clips = manifest
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            let clip = row["clip"].as_str().unwrap();
            Clip {
                name: clip.to_string(),
                set: row["set"].as_str().unwrap().to_string(),
                category: row["category"].as_str().map(str::to_string),
                text: row["text"].as_str().unwrap_or_default().to_string(),
                terms: row["terms"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0) as usize))
                            .collect()
                    })
                    .unwrap_or_default(),
                pcm: read_audio(&base.join(clip)),
            }
        })
        .collect();
    (clips, terms, dir)
}

#[derive(Default)]
struct Tally {
    occurrences: usize,
    hits: usize,
    collapses: Vec<String>,
    insertions: usize,
    reference_words: usize,
    word_edits: usize,
    times_ms: Vec<u64>,
    audio_ms: u64,
    per_term: BTreeMap<String, (usize, usize)>,
}

fn percentile(values: &[u64], p: f64) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted
        .get(((sorted.len() as f64 - 1.0) * p).round() as usize)
        .copied()
        .unwrap_or(0)
}

/// The baseline arm on PriMock57 with its floors.
async fn primock57_baseline() {
    let (clips, terms, _unpacked) = evaluation_set();
    let speech: Vec<&Clip> = clips.iter().filter(|c| c.set == "speech").collect();
    let gaps: Vec<&Clip> = clips.iter().filter(|c| c.set == "gap").collect();
    assert_eq!((speech.len(), gaps.len()), (60, 16), "the set's shape");
    assert_eq!(
        speech.iter().flat_map(|c| c.terms.values()).sum::<usize>(),
        85
    );
    let out = out_dir();
    let mut failures: Vec<String> = Vec::new();
    let mut summary = serde_json::Map::new();

    for entry in selected_models() {
        let spec = spec_for(entry).await;
        let loaded = service::service()
            .prepare_with(spec.clone())
            .await
            .expect("loads");
        let mut model_summary = serde_json::Map::new();
        for condition in ["clean", "snr10", "reverb"] {
            let mut tally = Tally::default();
            let mut transcripts = Vec::new();
            for clip in &speech {
                let pcm = match condition {
                    "snr10" => with_noise(&clip.pcm, &clip.name),
                    "reverb" => with_reverb(&clip.pcm, &clip.name),
                    _ => clip.pcm.clone(),
                };
                tally.audio_ms += pcm.len() as u64 / 16;
                let (transcript, ms) = transcribe(&spec, pcm).await;
                tally.times_ms.push(ms);
                let reference = words(&clip.text);
                let output = words(&transcript.text);
                tally.reference_words += reference.len();
                tally.word_edits += edits(&reference, &output);
                if output.len() * 4 <= reference.len() {
                    tally.collapses.push(clip.name.clone());
                }
                for term in &terms {
                    let expected = clip.terms.get(term).copied().unwrap_or(0);
                    let found = occurrences(&transcript.text, term);
                    let hit = found.min(expected);
                    tally.occurrences += expected;
                    tally.hits += hit;
                    tally.insertions += found.saturating_sub(expected);
                    let row = tally.per_term.entry(term.clone()).or_default();
                    row.0 += expected;
                    row.1 += hit;
                }
                transcripts.push(json!({
                    "clip": clip.name, "reference": clip.text, "output": transcript.text,
                    "ms": ms, "audio_ms": transcript.audio_ms,
                }));
            }
            let recall = tally.hits as f64 / tally.occurrences.max(1) as f64;
            let wer = tally.word_edits as f64 / tally.reference_words.max(1) as f64;
            let rtf = tally.times_ms.iter().sum::<u64>() as f64 / tally.audio_ms.max(1) as f64;
            println!(
                "{} {} on {}: strict recall {}/{} ({:.1} %), collapses {}, word error rate {:.1} %, spurious term insertions {}, per clip median {} ms p90 {} ms, real time factor {:.3}",
                entry.id,
                condition,
                loaded.backend,
                tally.hits,
                tally.occurrences,
                100.0 * recall,
                tally.collapses.len(),
                100.0 * wer,
                tally.insertions,
                percentile(&tally.times_ms, 0.5),
                percentile(&tally.times_ms, 0.9),
                rtf
            );
            if !tally.collapses.is_empty() {
                failures.push(format!(
                    "{} {condition}: {} collapses ({:?})",
                    entry.id,
                    tally.collapses.len(),
                    tally.collapses
                ));
            }
            let per_term: Value = tally
                .per_term
                .iter()
                .map(|(term, (n, hit))| (term.clone(), json!({"occurrences": n, "hits": hit})))
                .collect::<serde_json::Map<_, _>>()
                .into();
            model_summary.insert(
                condition.to_string(),
                json!({
                    "strict_recall": recall, "hits": tally.hits, "occurrences": tally.occurrences,
                    "collapses": tally.collapses, "word_error_rate": wer,
                    "spurious_insertions": tally.insertions,
                    "median_ms": percentile(&tally.times_ms, 0.5),
                    "p90_ms": percentile(&tally.times_ms, 0.9), "real_time_factor": rtf,
                    "per_term": per_term,
                }),
            );
            std::fs::write(
                out.join(format!("{}-{condition}.json", entry.id)),
                serde_json::to_vec_pretty(&transcripts).unwrap(),
            )
            .unwrap();
        }

        let mut outputs = Vec::new();
        let mut gap_rows = Vec::new();
        for gap in &gaps {
            let (transcript, ms) = transcribe(&spec, gap.pcm.clone()).await;
            if !transcript.text.is_empty() {
                outputs.push(format!("{}: {}", gap.name, transcript.text));
            }
            gap_rows.push(json!({
                "clip": gap.name, "category": gap.category, "output": transcript.text, "ms": ms,
            }));
        }
        let allowed = if entry.id == TURBO_ID { 1 } else { 0 };
        println!(
            "{} gaps: {} of {} with output (floor {allowed}): {outputs:?}",
            entry.id,
            outputs.len(),
            gaps.len()
        );
        if outputs.len() > allowed {
            failures.push(format!(
                "{}: {} of 16 gaps gave output, the floor is {allowed}: {outputs:?}",
                entry.id,
                outputs.len()
            ));
        }
        model_summary.insert(
            "gaps".into(),
            json!({"outputs": outputs.len(), "of": gaps.len(), "floor": allowed}),
        );
        model_summary.insert("backend".into(), json!(loaded.backend));
        std::fs::write(
            out.join(format!("{}-gaps.json", entry.id)),
            serde_json::to_vec_pretty(&gap_rows).unwrap(),
        )
        .unwrap();
        summary.insert(entry.id.to_string(), Value::Object(model_summary));
        service::service().unload().await;
    }
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&Value::Object(summary)).unwrap(),
    )
    .unwrap();
    println!("tables and transcripts: {}", out.display());
    assert!(failures.is_empty(), "baseline floors failed: {failures:#?}");
}

const SYNTHETIC_ENV: &str = "BIOROUTER_DICTATION_SYNTHETIC_DIR";
const FAREEZ_ENV: &str = "BIOROUTER_DICTATION_FAREEZ_DIR";
/// The synthetic dictation set's 14 terms (`speech-model-benchmarks.md`).
const SYNTHETIC_TERMS: [&str; 14] = [
    "EPAS1",
    "EGLN1",
    "HIF-2α",
    "UCSF",
    "Baranzini",
    "SPOKE",
    "OMOP",
    "natalizumab",
    "ocrelizumab",
    "scRNA-seq",
    "CRISPRi",
    "Biorouter",
    "PHD2",
    "VHL",
];

/// The person's dictionary of `terms`, all public, in a store of its own, through the
/// bias set every caller uses. Each term is stamped one second older than the one before
/// it. With no context and no usage every score is equal, and ties break on `updated_at`
/// descending, so the ranking (and with it the prompt's 30 terms and the fuzzy set) is the
/// list's own order: the same in every run and in every arm built from this value. Equal
/// stamps would fall through to the random ids.
fn eligible_of(terms: &[String]) -> EligibleEntries {
    let dir = tempfile::tempdir().expect("a folder");
    let store = DictionaryStore::open(dir.path().join("dictionary"));
    let newest = chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("a time");
    for (i, term) in terms.iter().enumerate() {
        let at = newest - chrono::Duration::seconds(i as i64);
        store
            .write(|file| {
                file.add(
                    NewTerm::person(term.clone(), EntryTier::Public),
                    Reach::All,
                    at,
                )
            })
            .unwrap_or_else(|e| panic!("{term}: {e}"));
    }
    let view = store.view().expect("the store");
    bias_set(&view, &BiasTarget::public(), Standing::public(), true)
}

/// [`eligible_of`], ranked and prepared with `channels`.
fn dictionary_of(terms: &[String], channels: BiasChannels) -> Arc<DictionaryRequest> {
    Arc::new(DictionaryRequest::new(eligible_of(terms), "", channels))
}

async fn dictate(
    spec: &biorouter::dictation::LoadSpec,
    pcm: Vec<i16>,
    dictionary: &Arc<DictionaryRequest>,
    decode: Option<DecodeOptions>,
) -> (biorouter::dictation::DictatedRecording, u64) {
    service::service()
        .install_bias(dictionary)
        .expect("the service");
    let mut request = TranscribeRequest::new("en");
    request.dictionary = Some(Arc::clone(dictionary));
    request.decode = decode;
    let started = Instant::now();
    let dictated = service::service()
        .transcribe_recording_with(spec.clone(), Arc::from(pcm), request)
        .await
        .expect("a transcript");
    (dictated, started.elapsed().as_millis() as u64)
}

/// The dictionary arm of Tier 1: the shipped mechanism, with the set's own 26 terms as
/// the person's dictionary, and the baseline's floors.
async fn primock57_dictionary() {
    let (clips, terms, _unpacked) = evaluation_set();
    let speech: Vec<&Clip> = clips.iter().filter(|c| c.set == "speech").collect();
    let gaps: Vec<&Clip> = clips.iter().filter(|c| c.set == "gap").collect();
    let dictionary = dictionary_of(&terms, BiasChannels::DEFAULT);
    let mut failures = Vec::new();
    let mut summary = serde_json::Map::new();
    for entry in selected_models() {
        let spec = spec_for(entry).await;
        service::service()
            .prepare_with(spec.clone())
            .await
            .expect("loads");
        let (mut hits, mut raw_hits, mut occurrences) = (0, 0, 0);
        let mut collapses = Vec::new();
        let mut times = Vec::new();
        for clip in &speech {
            let (dictated, ms) = dictate(&spec, clip.pcm.clone(), &dictionary, None).await;
            times.push(ms);
            let text = &dictated.transcript.text;
            let raw = &dictated.transcript.raw_text;
            if words(text).len() * 4 <= words(&clip.text).len() {
                collapses.push(clip.name.clone());
            }
            for term in &terms {
                let expected = clip.terms.get(term).copied().unwrap_or(0);
                occurrences += expected;
                hits += occurrences_capped(text, term, expected);
                raw_hits += occurrences_capped(raw, term, expected);
            }
        }
        let mut gap_outputs = Vec::new();
        for gap in &gaps {
            let (dictated, _) = dictate(&spec, gap.pcm.clone(), &dictionary, None).await;
            if !dictated.transcript.text.is_empty() {
                gap_outputs.push(format!("{}: {}", gap.name, dictated.transcript.text));
            }
        }
        let allowed = if entry.id == TURBO_ID { 1 } else { 0 };
        println!(
            "{} with the dictionary: strict recall {hits}/{occurrences} ({:.1} %), before correction {raw_hits}, collapses {}, gaps with output {} (floor {allowed}), per clip median {} ms p90 {} ms",
            entry.id,
            100.0 * hits as f64 / occurrences.max(1) as f64,
            collapses.len(),
            gap_outputs.len(),
            percentile(&times, 0.5),
            percentile(&times, 0.9),
        );
        if !collapses.is_empty() {
            failures.push(format!("{}: collapses {collapses:?}", entry.id));
        }
        if gap_outputs.len() > allowed {
            failures.push(format!("{}: gaps {gap_outputs:?}", entry.id));
        }
        summary.insert(
            entry.id.to_string(),
            json!({"hits": hits, "raw_hits": raw_hits, "occurrences": occurrences,
                   "collapses": collapses, "gap_outputs": gap_outputs.len(),
                   "median_ms": percentile(&times, 0.5), "p90_ms": percentile(&times, 0.9)}),
        );
        service::service().unload().await;
    }
    std::fs::write(
        out_dir().join("dictionary-summary.json"),
        serde_json::to_vec_pretty(&Value::Object(summary)).unwrap(),
    )
    .unwrap();
    assert!(
        failures.is_empty(),
        "dictionary floors failed: {failures:#?}"
    );
}

/// `term`'s occurrences in `text` by the evaluation set's rule, at most `expected`.
fn occurrences_capped(text: &str, term: &str, expected: usize) -> usize {
    occurrences(text, term).min(expected)
}

/// Exact, case kept, whole word: the synthetic set's strict rule.
fn exact_occurrences(text: &str, term: &str) -> usize {
    text.match_indices(term)
        .filter(|(at, _)| {
            let before = text.get(..*at).and_then(|s| s.chars().next_back());
            let after = text.get(at + term.len()..).and_then(|s| s.chars().next());
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
        .count()
}

/// A two sided exact sign test over the pairs one arm won and the other lost.
fn sign_test(wins: usize, losses: usize) -> f64 {
    let n = wins + losses;
    if n == 0 {
        return 1.0;
    }
    let k = wins.min(losses);
    let mut tail = 0.0f64;
    let mut binomial = 1.0f64;
    for i in 0..=k {
        if i > 0 {
            binomial = binomial * (n - i + 1) as f64 / i as f64;
        }
        tail += binomial;
    }
    (2.0 * tail / 2f64.powi(n as i32)).min(1.0)
}

/// The lists taken in turn (the first of each, then the second of each, and so on), each
/// term once, where it first appears.
fn interleave(lists: &[&Vec<String>]) -> Vec<String> {
    let longest = lists.iter().map(|l| l.len()).max().unwrap_or(0);
    let mut out: Vec<String> = Vec::new();
    for i in 0..longest {
        for list in lists {
            if let Some(term) = list.get(i) {
                if !out.contains(term) {
                    out.push(term.clone());
                }
            }
        }
    }
    out
}

/// DD-44's test of `candidate` against `other` (`testing-and-release.md`, "The M-22
/// decision and its fallback", rule 1): the reasons it fails, empty when it passes.
fn fails_against(
    candidate: &ArmScore,
    other_name: &str,
    other: &ArmScore,
    margin: f64,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if candidate.real() < other.real() - margin {
        reasons.push(format!(
            "real recall below {other_name} by more than the margin"
        ));
    }
    if candidate.synthetic() < other.synthetic() - 0.05 {
        reasons.push(format!(
            "synthetic recall below {other_name} by more than 5 points"
        ));
    }
    let pairs = || candidate.real_pairs.iter().zip(&other.real_pairs);
    let wins = pairs().filter(|(c, o)| c > o).count();
    let losses = pairs().filter(|(c, o)| c < o).count();
    let p = sign_test(wins, losses);
    if losses > wins && p < 0.05 {
        reasons.push(format!(
            "a sign test finds it worse than {other_name} (p {p:.3})"
        ));
    }
    for (metric, mine, theirs) in [
        (
            "nonspeech outputs",
            candidate.nonspeech_outputs,
            other.nonspeech_outputs,
        ),
        ("insertions", candidate.insertions, other.insertions),
        ("collapses", candidate.collapses, other.collapses),
    ] {
        if mine > theirs {
            reasons.push(format!(
                "more {metric} than {other_name} ({mine} against {theirs})"
            ));
        }
    }
    reasons
}

/// The gate metrics on which `candidate` is strictly better than `other` (rule 2).
fn better_than(candidate: &ArmScore, other: &ArmScore) -> Vec<&'static str> {
    let mut better = Vec::new();
    if candidate.real() > other.real() {
        better.push("real recall");
    }
    if candidate.synthetic() > other.synthetic() {
        better.push("synthetic recall");
    }
    if candidate.nonspeech_outputs < other.nonspeech_outputs {
        better.push("nonspeech outputs");
    }
    if candidate.insertions < other.insertions {
        better.push("insertions");
    }
    if candidate.collapses < other.collapses {
        better.push("collapses");
    }
    better
}

/// One clip of the M-22 material.
struct ArmClip {
    name: String,
    kind: &'static str,
    reference: String,
    /// Per term, the occurrences the reference holds.
    expected: Vec<(String, usize)>,
    pcm: Vec<i16>,
}

/// What one arm did, gate metric by gate metric.
#[derive(Default, Clone)]
struct ArmScore {
    real_hits: usize,
    real_occurrences: usize,
    synthetic_hits: usize,
    synthetic_occurrences: usize,
    nonspeech_outputs: usize,
    insertions: usize,
    collapses: usize,
    /// Per (clip, term) of the real sets, the hits, for the paired sign test.
    real_pairs: Vec<usize>,
    times_ms: Vec<u64>,
}

impl ArmScore {
    fn real(&self) -> f64 {
        self.real_hits as f64 / self.real_occurrences.max(1) as f64
    }

    fn synthetic(&self) -> f64 {
        self.synthetic_hits as f64 / self.synthetic_occurrences.max(1) as f64
    }
}

/// The synthetic set: `sentences.tsv` (id, reference, spoken), `probe_ref.tsv` (id,
/// reference) and `audio/` holding `s*` and `c*` dictation clips, `n_*` nonspeech clips and
/// `p*` probe clips; reversed speech is made here from the `s*` clips.
fn synthetic_set(dir: &Path) -> Vec<ArmClip> {
    let read_table = |name: &str| -> BTreeMap<String, String> {
        std::fs::read_to_string(dir.join(name))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut cols = line.split('\t');
                Some((cols.next()?.to_string(), cols.next()?.to_string()))
            })
            .collect()
    };
    let sentences = read_table("sentences.tsv");
    let probes = read_table("probe_ref.tsv");
    let mut clips = Vec::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir.join("audio"))
        .expect("the synthetic audio")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "wav"))
        .collect();
    names.sort();
    for path in names {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let id = stem.split('_').next().unwrap_or_default().to_string();
        let bytes = std::fs::read(&path).expect("a clip");
        let pcm = audio::parse_wav_s16(&bytes)
            .expect("16 bit PCM")
            .to_16k_mono();
        let (kind, reference) = if stem.starts_with("n_") {
            ("nonspeech", String::new())
        } else if let Some(reference) = probes.get(&id) {
            ("probe", reference.clone())
        } else if let Some(reference) = sentences.get(&id) {
            ("dictation", reference.clone())
        } else {
            continue;
        };
        let expected = SYNTHETIC_TERMS
            .iter()
            .map(|t| (t.to_string(), exact_occurrences(&reference, t)))
            .collect();
        if kind == "dictation" && id.starts_with('s') {
            let mut reversed = pcm.clone();
            reversed.reverse();
            clips.push(ArmClip {
                name: format!("{stem}_reversed"),
                kind: "reversed",
                reference: String::new(),
                expected: SYNTHETIC_TERMS.iter().map(|t| (t.to_string(), 0)).collect(),
                pcm: reversed,
            });
        }
        clips.push(ArmClip {
            name: stem,
            kind,
            reference,
            expected,
            pcm,
        });
    }
    clips
}

/// A real set built by `scripts/dictation/fetch-fareez-subset.sh`: `manifest.jsonl` rows of
/// `{clip, text, terms}` and the clips beside it.
fn fareez_set(dir: &Path) -> Vec<ArmClip> {
    let manifest = std::fs::read_to_string(dir.join("manifest.jsonl")).expect("the manifest");
    manifest
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let row: Value = serde_json::from_str(line).expect("a row");
            let clip = row["clip"].as_str().expect("a clip").to_string();
            ArmClip {
                pcm: read_audio(&dir.join(&clip)),
                name: clip,
                kind: "real",
                reference: row["text"].as_str().unwrap_or_default().to_string(),
                expected: row["terms"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0) as usize))
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// M-22's arms and the DD-44 decision (`testing-and-release.md`). A maintainer's
/// measurement, not a Tier 1 gate: with no synthetic set named it says so and returns, so
/// the nightly workflow, which runs every ignored test here, is not failed by it.
async fn m22_arms() {
    if std::env::var_os(SYNTHETIC_ENV).is_none() {
        println!("m22_arms: {SYNTHETIC_ENV} is not set, so M-22 was not run");
        return;
    }
    let (primock, primock_terms, _unpacked) = evaluation_set();
    let mut real: Vec<ArmClip> = primock
        .iter()
        .map(|c| ArmClip {
            name: c.name.clone(),
            kind: if c.set == "gap" { "gap" } else { "real" },
            reference: c.text.clone(),
            expected: primock_terms
                .iter()
                .map(|t| (t.clone(), c.terms.get(t).copied().unwrap_or(0)))
                .collect(),
            pcm: c.pcm.clone(),
        })
        .collect();
    let occurrences_in = |clips: &[ArmClip]| -> usize {
        clips
            .iter()
            .flat_map(|c| c.expected.iter().map(|(_, n)| *n))
            .sum()
    };
    let mut real_sets = vec![format!(
        "PriMock57 subset ({} clips, {} occurrences)",
        real.len(),
        occurrences_in(&real)
    )];
    let mut fareez_terms: Vec<String> = Vec::new();
    match std::env::var_os(FAREEZ_ENV) {
        Some(dir) => {
            let fareez = fareez_set(Path::new(&dir));
            for clip in &fareez {
                for (term, _) in &clip.expected {
                    if !fareez_terms.contains(term) {
                        fareez_terms.push(term.clone());
                    }
                }
            }
            real_sets.push(format!(
                "Fareez et al. 2022 subset ({} clips, {} occurrences)",
                fareez.len(),
                occurrences_in(&fareez)
            ));
            real.extend(fareez);
            // Sorted, so the order does not hang on how a JSON map iterates.
            fareez_terms.sort();
        }
        None => real_sets.push("Fareez subset not built: PriMock57 alone, 5 point margin".into()),
    }
    let synthetic = synthetic_set(Path::new(
        &std::env::var_os(SYNTHETIC_ENV)
            .expect("set BIOROUTER_DICTATION_SYNTHETIC_DIR to the synthetic set"),
    ));
    let synthetic_terms: Vec<String> = SYNTHETIC_TERMS.iter().map(|t| t.to_string()).collect();
    // The dictionary holds more terms than the prompt's 30, so its order decides which
    // terms the prompt carries. The three lists are taken in turn, so each set has its
    // share of the prompt, and the order is fixed by the lists alone.
    let terms = interleave(&[&primock_terms, &fareez_terms, &synthetic_terms]);
    let real_occurrences: usize = real
        .iter()
        .flat_map(|c| c.expected.iter().map(|(_, n)| *n))
        .sum();
    let margin = if real_occurrences >= 160 { 0.03 } else { 0.05 };
    println!("M-22 real sets: {real_sets:?}, {real_occurrences} occurrences, margin {margin}");
    // One eligible set for the whole run: every arm that carries a prompt sends the same
    // one, and every arm corrects with the same entries.
    let eligible = eligible_of(&terms);
    let prompt_terms =
        DictionaryRequest::new(eligible.clone(), "", BiasChannels::DEFAULT).prompt_terms();
    println!(
        "M-22 dictionary: {} terms, {} eligible; the prompt's {} terms, best last: {prompt_terms:?}",
        terms.len(),
        eligible.entries().len(),
        prompt_terms.len()
    );

    let channel_arms = [
        ("none", BiasChannels::NONE),
        (
            "prompt",
            BiasChannels {
                prompt: true,
                bonus: false,
            },
        ),
        (
            "bonus",
            BiasChannels {
                prompt: false,
                bonus: true,
            },
        ),
        ("both", BiasChannels::DEFAULT),
    ];
    let out = out_dir();
    let mut summary = serde_json::Map::new();
    let mut combination_holds = true;
    let mut setting_adopted: BTreeMap<&str, bool> = BTreeMap::new();
    for entry in selected_models() {
        let spec = spec_for(entry).await;
        service::service()
            .prepare_with(spec.clone())
            .await
            .expect("loads");
        let mut variants: Vec<(String, BiasChannels, Option<DecodeOptions>)> = channel_arms
            .iter()
            .map(|(name, channels)| (name.to_string(), *channels, None))
            .collect();
        let both_temp0 = DecodeOptions {
            temperature_inc: 0.0,
            ..DecodeOptions::dd47(0)
        };
        variants.push((
            "both, temperature_inc 0".into(),
            BiasChannels::DEFAULT,
            Some(both_temp0),
        ));
        if entry.id == SMALL_EN_ID {
            let no_timestamps = DecodeOptions {
                no_timestamps: true,
                ..DecodeOptions::dd47(0)
            };
            variants.push((
                "both, timestamps off".into(),
                BiasChannels::DEFAULT,
                Some(no_timestamps),
            ));
        }
        let mut scores: BTreeMap<String, (ArmScore, ArmScore)> = BTreeMap::new();
        for (name, channels, decode) in &variants {
            let dictionary = Arc::new(DictionaryRequest::new(eligible.clone(), "", *channels));
            if channels.prompt {
                assert_eq!(dictionary.prompt_terms(), prompt_terms, "{name}");
            }
            let (mut raw, mut corrected) = (ArmScore::default(), ArmScore::default());
            let mut transcripts = Vec::new();
            for clip in real.iter().chain(&synthetic) {
                let (dictated, ms) =
                    dictate(&spec, clip.pcm.clone(), &dictionary, decode.clone()).await;
                for (score, text) in [
                    (&mut raw, dictated.transcript.raw_text.clone()),
                    (&mut corrected, dictated.transcript.text.clone()),
                ] {
                    score.times_ms.push(ms);
                    let exact = clip.kind != "real";
                    let found = |term: &str| {
                        if exact {
                            exact_occurrences(&text, term)
                        } else {
                            occurrences(&text, term)
                        }
                    };
                    match clip.kind {
                        "real" => {
                            for (term, expected) in &clip.expected {
                                let hit = found(term).min(*expected);
                                score.real_hits += hit;
                                score.real_occurrences += expected;
                                if *expected > 0 {
                                    score.real_pairs.push(hit);
                                }
                            }
                        }
                        "dictation" => {
                            for (term, expected) in &clip.expected {
                                score.synthetic_hits += found(term).min(*expected);
                                score.synthetic_occurrences += expected;
                            }
                        }
                        "gap" | "nonspeech" => {
                            score.nonspeech_outputs += usize::from(!text.trim().is_empty());
                        }
                        _ => {
                            // Probe clips and reversed speech: any dictionary term the
                            // reference does not hold is an insertion.
                            for term in &terms {
                                score.insertions += exact_occurrences(&text, term)
                                    .saturating_sub(exact_occurrences(&clip.reference, term));
                            }
                        }
                    }
                    if matches!(clip.kind, "real" | "dictation")
                        && words(&text).len() * 4 <= words(&clip.reference).len()
                    {
                        score.collapses += 1;
                    }
                }
                transcripts.push(json!({"clip": clip.name, "kind": clip.kind,
                    "raw": dictated.transcript.raw_text, "text": dictated.transcript.text,
                    "changes": dictated.changes.len(), "ms": ms}));
            }
            std::fs::write(
                out.join(format!(
                    "m22-{}-{}.json",
                    entry.id,
                    name.replace([' ', ','], "_")
                )),
                serde_json::to_vec_pretty(&transcripts).unwrap(),
            )
            .unwrap();
            for (label, score) in [("before correction", &raw), ("with correction", &corrected)] {
                println!(
                    "{} {name} {label}: real {}/{} ({:.1} %), synthetic {}/{} ({:.1} %), nonspeech outputs {}, insertions {}, collapses {}, median {} ms",
                    entry.id,
                    score.real_hits,
                    score.real_occurrences,
                    100.0 * score.real(),
                    score.synthetic_hits,
                    score.synthetic_occurrences,
                    100.0 * score.synthetic(),
                    score.nonspeech_outputs,
                    score.insertions,
                    score.collapses,
                    percentile(&score.times_ms, 0.5),
                );
            }
            scores.insert(name.clone(), (raw, corrected));
        }
        // DD-44's rule 1, on the corrected text, because correction always runs.
        let arm = |name: &str| scores.get(name).map(|(_, c)| c.clone()).unwrap_or_default();
        let (both, prompt, bonus) = (arm("both"), arm("prompt"), arm("bonus"));
        let mut reasons = fails_against(&both, "prompt", &prompt, margin);
        reasons.extend(fails_against(&both, "bonus", &bonus, margin));
        println!(
            "{}: the combination {}",
            entry.id,
            if reasons.is_empty() {
                "passes every rule".to_string()
            } else {
                format!("fails: {reasons:?}")
            }
        );
        combination_holds &= reasons.is_empty();
        // Rule 2: an alternative setting replaces the default only if it passes the same
        // test against it and is strictly better on at least one gate metric. Measured
        // with both channels on, the arms the alternatives were run with.
        let mut settings = serde_json::Map::new();
        for (alternative, setting) in [
            ("both, temperature_inc 0", "temperature_inc"),
            ("both, timestamps off", "timestamps"),
        ] {
            let Some((_, alt)) = scores.get(alternative) else {
                continue;
            };
            let fails = fails_against(alt, "the default", &both, margin);
            let better = better_than(alt, &both);
            let adopt = fails.is_empty() && !better.is_empty();
            println!(
                "{}: {alternative}: fails {fails:?}, strictly better on {better:?}, so the default {}",
                entry.id,
                if adopt { "changes" } else { "stays" }
            );
            *setting_adopted.entry(setting).or_insert(true) &= adopt;
            settings.insert(
                alternative.to_string(),
                json!({"fails": fails, "better_on": better, "adopt": adopt}),
            );
        }
        summary.insert(
            entry.id.to_string(),
            json!({
                "arms": scores.iter().map(|(name, (raw, corrected))| (name.clone(), json!({
                    "real": [raw.real(), corrected.real()],
                    "synthetic": [raw.synthetic(), corrected.synthetic()],
                    "nonspeech_outputs": [raw.nonspeech_outputs, corrected.nonspeech_outputs],
                    "insertions": [raw.insertions, corrected.insertions],
                    "collapses": [raw.collapses, corrected.collapses],
                    "median_ms": percentile(&corrected.times_ms, 0.5),
                }))).collect::<serde_json::Map<_, _>>(),
                "combination_fails": reasons,
                "settings": settings,
            }),
        );
        service::service().unload().await;
    }
    let decision = if combination_holds {
        "the token prompt and the decoder bonus together stay the default"
    } else {
        "the declared fallback: the token prompt only, the decoder bonus off"
    };
    println!("M-22 decision: {decision}; real sets {real_sets:?}");
    let settings_decision: serde_json::Map<String, Value> = setting_adopted
        .iter()
        .map(|(setting, adopt)| {
            let outcome = match (*setting, *adopt) {
                ("temperature_inc", true) => "temperature_inc 0 replaces 0.2",
                ("temperature_inc", false) => "temperature_inc stays 0.2",
                (_, true) => "timestamps off for small.en",
                (_, false) => "timestamps stay on",
            };
            println!("M-22 rule 2, {setting}: {outcome}");
            (setting.to_string(), json!(outcome))
        })
        .collect();
    summary.insert("decision".into(), json!(decision));
    summary.insert("settings_decision".into(), Value::Object(settings_decision));
    summary.insert("real_sets".into(), json!(real_sets));
    summary.insert("real_occurrences".into(), json!(real_occurrences));
    summary.insert("dictionary_terms".into(), json!(terms));
    summary.insert("prompt_terms".into(), json!(prompt_terms));
    summary.insert(
        "gene_symbols_on_real_speech".into(),
        json!("not measured: no licensed public recording of real speech with gene symbols exists"),
    );
    std::fs::write(
        out.join("m22-summary.json"),
        serde_json::to_vec_pretty(&Value::Object(summary)).unwrap(),
    )
    .unwrap();
    println!("M-22 tables and transcripts: {}", out.display());
}

fn the_scoring_helpers() {
    assert!((sign_test(0, 0) - 1.0).abs() < 1e-12);
    assert!((sign_test(3, 3) - 1.0).abs() < 1e-12);
    // 0 wins of 6: 2 / 64.
    assert!((sign_test(0, 6) - 0.03125).abs() < 1e-12);
    assert_eq!(exact_occurrences("EPAS1 and epas1, EPAS10", "EPAS1"), 1);
    assert_eq!(
        words("Uh, so, similar to Ibuprofen; don't."),
        ["so", "similar", "to", "ibuprofen", "don't"]
    );
    let a = words("take the ventolin twice");
    let b = words("take ventolin twice daily");
    assert_eq!(edits(&a, &b), 2);
    assert_eq!(edits(&a, &a), 0);
    assert_eq!(
        occurrences(
            "Paracetamol and paracetamol, not paracetamols",
            "paracetamol"
        ),
        2
    );
    assert_eq!(occurrences("EpiPen", "epipen"), 1);
    assert_eq!(percentile(&[5, 1, 3], 0.5), 3);
    let tone: Vec<i16> = (0..16_000)
        .map(|i| ((i % 40) * 500 - 10_000) as i16)
        .collect();
    assert_eq!(with_noise(&tone, "a"), with_noise(&tone, "a"), "seeded");
    assert_ne!(with_noise(&tone, "a"), with_noise(&tone, "b"));
    let reverb = with_reverb(&tone, "a");
    assert_eq!(reverb.len(), tone.len());
    assert_eq!(
        reverb.iter().map(|s| s.unsigned_abs()).max(),
        tone.iter().map(|s| s.unsigned_abs()).max()
    );

    // M-22's dictionary: the lists in turn, each term once.
    let list = |terms: &[&str]| -> Vec<String> { terms.iter().map(|t| t.to_string()).collect() };
    assert_eq!(
        interleave(&[
            &list(&["a", "b", "c"]),
            &list(&["x"]),
            &list(&["b", "y", "z"])
        ]),
        list(&["a", "x", "b", "y", "c", "z"])
    );
    // More terms than the prompt holds, all with equal scores: the prompt is the same 30
    // in two separate stores, and they are the first 30 of the list that do not collide.
    let terms: Vec<String> = (0..45).map(|i| format!("Zorvatin{i:02}x")).collect();
    let first = DictionaryRequest::new(eligible_of(&terms), "", BiasChannels::DEFAULT);
    let second = DictionaryRequest::new(eligible_of(&terms), "", BiasChannels::DEFAULT);
    let mut expected: Vec<String> = terms[..30].to_vec();
    expected.reverse();
    assert_eq!(first.prompt_terms(), expected, "best last");
    assert_eq!(second.prompt_terms(), expected);

    // DD-44's rules on scores made by hand.
    let score = |real_hits: usize, insertions: usize, pairs: Vec<usize>| ArmScore {
        real_hits,
        real_occurrences: 100,
        synthetic_hits: 80,
        synthetic_occurrences: 100,
        insertions,
        real_pairs: pairs,
        ..ArmScore::default()
    };
    let base = score(80, 1, vec![1; 10]);
    assert!(fails_against(&score(78, 1, vec![1; 10]), "b", &base, 0.03).is_empty());
    assert_eq!(
        fails_against(&score(76, 1, vec![1; 10]), "b", &base, 0.03),
        ["real recall below b by more than the margin"]
    );
    assert_eq!(
        fails_against(&score(80, 2, vec![1; 10]), "b", &base, 0.03),
        ["more insertions than b (2 against 1)"]
    );
    assert_eq!(
        fails_against(&score(80, 1, vec![0; 10]), "b", &base, 0.03),
        ["a sign test finds it worse than b (p 0.002)"]
    );
    assert_eq!(
        better_than(&score(81, 0, vec![1; 10]), &base),
        ["real recall", "insertions"]
    );
    assert!(better_than(&base, &base).is_empty());
}
