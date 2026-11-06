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
//!   any condition. PR-14 adds the dictionary arm and its floors.
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
use biorouter::dictation::{audio, DictationError, RecordingTranscript};
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
        .expect("a transcript");
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

fn the_scoring_helpers() {
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
}
