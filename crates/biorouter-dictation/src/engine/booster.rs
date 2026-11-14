//! The decoder bonus (`dictionary.md`, "Decoder bonus", DD-46): a `logits_filter_callback`
//! over the whole bias set, greedy decoding only.
//!
//! * A trie over the bytes of `" " + written` and `written` for every entry `set_bias`
//!   sends (the parent sends only entries with `bias` `auto` or `always` that do not
//!   collide unless pinned). Aliases are never boosted.
//! * Per trie node, the vocabulary ids below end of text whose bytes continue some form
//!   from that node, precomputed from the model's own byte to id map, so whatever pieces
//!   the model emits are followed.
//! * Per callback, stateless: the byte tail of the decoded text back to the last id at or
//!   above end of text; the nodes reached by a suffix that starts at a word boundary (a
//!   space, the window's start, or after `( [ / " '`); and the gated bonuses.
//!
//! The parameters are [`BonusParams`]: `b_start = gate_start = 2.0`, `b_cont = gate_cont =
//! 5.0`, `ramp = 2`, `first = 0` (no start bonus before a window's first text token),
//! `min_start = 2`, the prefix steal guard, `cap = 5.0`, and never an id at or above end of
//! text. Measured in the spike: 72 % strict recall at 1,000 terms against 49 to 52 % for
//! the same terms as a prompt, 0 dictionary insertions on 75 probe clips, 0.57 ms per
//! utterance [M].
//!
//! The table is immutable once built, so the callback is reentrant: whisper.cpp calls it
//! once per decoder per step, from up to `min(n_threads, n_decoders)` threads at once. It
//! allocates nothing: the tail and the candidates live in fixed buffers on the stack. It
//! runs inside `catch_unwind`, and a bonus that cannot be applied leaves the logits as they
//! are.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use super::bpe::Vocab;
use crate::protocol::{BonusParams, SetBias};
use crate::recognize::words;

/// The longest byte tail a callback reads.
const TAIL_BYTES: usize = 1024;
/// The most decoded tokens a callback walks back over.
const TAIL_TOKENS: usize = 512;
/// The most candidates one step considers; more are dropped, which only lowers a bonus.
const MAX_CANDIDATES: usize = 256;

struct Node {
    kids: Vec<(u8, u32)>,
    /// Bytes matched from the root, the leading space not counted.
    core: u16,
}

/// One built decoder bonus table.
pub struct BonusTable {
    nodes: Vec<Node>,
    /// Per node, the ids that continue a form from it.
    cont: Vec<Box<[u32]>>,
    /// Ids that start a form, with its leading space and without it.
    start_space: Box<[u32]>,
    start_bare: Box<[u32]>,
    vocab: Arc<Vocab>,
    eot: i32,
    max_form: usize,
    params: BonusParams,
    /// Callbacks and their time, for measurement; relaxed counters, no lock.
    calls: AtomicU64,
    nanos: AtomicU64,
}

impl BonusTable {
    /// Builds the trie and the per node continuations for `set`'s entries over `vocab`.
    pub fn build(set: &SetBias, vocab: Arc<Vocab>) -> BonusTable {
        let params = set.params;
        let mut table = BonusTable {
            nodes: vec![Node {
                kids: Vec::new(),
                core: 0,
            }],
            cont: Vec::new(),
            start_space: Box::new([]),
            start_bare: Box::new([]),
            eot: vocab.eot(),
            max_form: 0,
            params,
            vocab: Arc::clone(&vocab),
            calls: AtomicU64::new(0),
            nanos: AtomicU64::new(0),
        };
        let mut cont: Vec<Vec<u32>> = vec![Vec::new()];
        let mut start_space: Vec<u32> = Vec::new();
        let mut start_bare: Vec<u32> = Vec::new();
        let longest = vocab.longest().max(1);
        for entry in &set.entries {
            let written = entry.written.trim();
            if written.is_empty() {
                continue;
            }
            for form in [format!(" {written}"), written.to_string()] {
                let bytes = form.as_bytes();
                let path = table.insert(bytes);
                cont.resize_with(table.nodes.len(), Vec::new);
                table.max_form = table.max_form.max(bytes.len());
                for i in 0..bytes.len() {
                    for j in i + 1..=bytes.len().min(i + longest) {
                        let Some(id) = bytes.get(i..j).and_then(|b| vocab.id(b)) else {
                            continue;
                        };
                        let Ok(id) = u32::try_from(id) else {
                            continue;
                        };
                        if i == 0 {
                            if !table.starts_form(bytes, j) {
                                continue;
                            }
                            let list = if bytes.first() == Some(&b' ') {
                                &mut start_space
                            } else {
                                &mut start_bare
                            };
                            if !list.contains(&id) {
                                list.push(id);
                            }
                        } else if let Some(node) =
                            path.get(i).and_then(|n| cont.get_mut(*n as usize))
                        {
                            if !node.contains(&id) {
                                node.push(id);
                            }
                        }
                    }
                }
            }
        }
        table.cont = cont.into_iter().map(Vec::into_boxed_slice).collect();
        table.start_space = start_space.into_boxed_slice();
        table.start_bare = start_bare.into_boxed_slice();
        table
    }

    /// Whether the first `j` bytes of `form` may start it: at least `min_start`
    /// characters unless they are the whole form, and not an English word unless they are
    /// the whole form. An English word here is one the list holds in lower case or in
    /// capitals, the reading `collides` takes: a piece the list holds only capitalized or
    /// in mixed case (` PH` of PHD2, from `pH`) may start a term, as it did in the spike.
    fn starts_form(&self, form: &[u8], j: usize) -> bool {
        let whole = j == form.len();
        let piece = form.get(..j).unwrap_or_default();
        let core = piece.strip_prefix(b" ").unwrap_or(piece);
        let core_text = std::str::from_utf8(core).ok();
        let chars = core_text.map_or(core.len(), |t| t.chars().count());
        if whole {
            return true;
        }
        if chars < self.params.min_start as usize {
            return false;
        }
        !(self.params.skip_word_starts && core_text.is_some_and(words::is_common))
    }

    /// Inserts `bytes` and returns the node after each prefix, the root first.
    fn insert(&mut self, bytes: &[u8]) -> Vec<u32> {
        let mut path = Vec::with_capacity(bytes.len() + 1);
        let mut node = 0u32;
        path.push(node);
        for (i, &b) in bytes.iter().enumerate() {
            let next = match self.child(node, b) {
                Some(n) => n,
                None => {
                    let parent_core = self.nodes[node as usize].core;
                    let core = if i == 0 && b == b' ' {
                        parent_core
                    } else {
                        parent_core.saturating_add(1)
                    };
                    let id = self.nodes.len() as u32;
                    self.nodes.push(Node {
                        kids: Vec::new(),
                        core,
                    });
                    self.nodes[node as usize].kids.push((b, id));
                    id
                }
            };
            node = next;
            path.push(node);
        }
        path
    }

    fn child(&self, node: u32, b: u8) -> Option<u32> {
        self.nodes
            .get(node as usize)?
            .kids
            .iter()
            .find(|(k, _)| *k == b)
            .map(|(_, n)| *n)
    }

    fn walk(&self, bytes: &[u8]) -> Option<u32> {
        let mut node = 0;
        for &b in bytes {
            node = self.child(node, b)?;
        }
        Some(node)
    }

    /// The trie's node count, as `bias_ready` reports it.
    pub fn nodes(&self) -> usize {
        self.nodes.len()
    }

    /// Callbacks made through this table and their total time, in nanoseconds.
    pub fn usage(&self) -> (u64, u64) {
        (
            self.calls.load(Ordering::Relaxed),
            self.nanos.load(Ordering::Relaxed),
        )
    }

    /// One decode step: adds the gated bonuses to `logits` given the tokens decoded so far
    /// in this window.
    pub fn apply(&self, tokens: &[i32], logits: &mut [f32]) {
        let started = Instant::now();
        self.apply_inner(tokens, logits);
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.nanos
            .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    fn apply_inner(&self, tokens: &[i32], logits: &mut [f32]) {
        // 1. The byte tail: back to the last special or timestamp token, and no further
        // than a form can span plus one byte for the boundary check.
        let mut tail = [0u8; TAIL_BYTES];
        let (tail_len, at_start) = self.byte_tail(tokens, &mut tail);
        let text_seen = tokens.iter().any(|&id| (0..self.eot).contains(&id));
        let tail = &tail[..tail_len];

        // 2. The row's best logit: the gates are relative to what the decoder would pick.
        let (best, argmax) = best_of(logits);
        if !best.is_finite() {
            return;
        }

        // 3. Candidates, the largest bonus per id.
        let mut cand = [(0u32, 0f32); MAX_CANDIDATES];
        let n_cand = self.candidates(logits, best, tail, at_start, text_seen, &mut cand);

        // 4. Apply, the largest bonus per id clamped to `cap`.
        if n_cand == 0 {
            return;
        }
        self.apply_bonuses(&cand[..n_cand], argmax, logits);
    }

    /// Step 1 of [`Self::apply_inner`]: walks back over the window's trailing text tokens
    /// and copies their bytes, oldest first, into `tail`. Returns how many bytes `tail`
    /// holds and whether the walk stopped at the window's start or a special token.
    fn byte_tail(&self, tokens: &[i32], tail: &mut [u8; TAIL_BYTES]) -> (usize, bool) {
        let mut ids = [0i32; TAIL_TOKENS];
        let mut n_ids = 0;
        let mut len = 0usize;
        let mut j = tokens.len();
        while j > 0 && n_ids < TAIL_TOKENS {
            let id = tokens[j - 1];
            if id >= self.eot || id < 0 {
                break;
            }
            ids[n_ids] = id;
            n_ids += 1;
            len += self.vocab.token(id).map_or(0, <[u8]>::len);
            j -= 1;
            if len > self.max_form + 1 {
                break;
            }
        }
        // `j` is now the index after the token that stopped the walk.
        let at_start = j == 0 || tokens.get(j - 1).is_some_and(|&id| id >= self.eot);
        let mut tail_len = 0;
        for &id in ids[..n_ids].iter().rev() {
            let bytes = self.vocab.token(id).unwrap_or_default();
            for &b in bytes {
                if tail_len < TAIL_BYTES {
                    tail[tail_len] = b;
                    tail_len += 1;
                }
            }
        }
        (tail_len, at_start)
    }

    /// Step 3 of [`Self::apply_inner`]: gathers into `cand` the start and continuation ids
    /// whose gate passes against `best`, keeping the largest bonus per id, and returns how
    /// many it holds.
    fn candidates(
        &self,
        logits: &[f32],
        best: f32,
        tail: &[u8],
        at_start: bool,
        text_seen: bool,
        cand: &mut [(u32, f32); MAX_CANDIDATES],
    ) -> usize {
        let p = &self.params;
        let mut n_cand = 0usize;
        let mut consider = |id: u32, bonus: f32, gate: f32| {
            let Some(&l) = logits.get(id as usize) else {
                return;
            };
            if bonus <= 0.0 || !l.is_finite() || best - l > gate {
                return;
            }
            if let Some(slot) = cand[..n_cand].iter_mut().find(|(c, _)| *c == id) {
                slot.1 = slot.1.max(bonus);
            } else if n_cand < MAX_CANDIDATES {
                cand[n_cand] = (id, bonus);
                n_cand += 1;
            }
        };
        // `first`: before the window's first text token the start bonus scales by it.
        let factor = if text_seen { 1.0 } else { p.first };
        if p.b_start * factor > 0.0 {
            for &id in self.start_space.iter() {
                consider(id, p.b_start * factor, p.gate_start * factor);
            }
            let bare_ok =
                (tail.is_empty() && at_start) || tail.last().is_some_and(|&b| is_open_punct(b));
            if bare_ok {
                for &id in self.start_bare.iter() {
                    consider(id, p.b_start * factor, p.gate_start * factor);
                }
            }
        }
        for pos in 0..tail.len() {
            let boundary = tail[pos] == b' '
                || (pos == 0 && at_start)
                || (pos > 0 && is_open_punct(tail[pos - 1]));
            if !boundary {
                continue;
            }
            let Some(node) = self.walk(&tail[pos..]).filter(|n| *n > 0) else {
                continue;
            };
            let core = self.nodes[node as usize].core;
            if u32::from(core) < p.min_cont {
                continue;
            }
            let r = if p.ramp > 0.0 {
                (f32::from(core) / p.ramp).min(1.0)
            } else {
                1.0
            };
            for &id in self.cont[node as usize].iter() {
                consider(id, p.b_cont * r, p.gate_cont * r);
            }
        }
        n_cand
    }

    /// Step 4 of [`Self::apply_inner`]: adds each candidate's bonus, clamped to `cap`, to
    /// its logit.
    fn apply_bonuses(&self, cand: &[(u32, f32)], argmax: i64, logits: &mut [f32]) {
        let p = &self.params;
        // The prefix steal guard: when the model's choice is not a candidate but extends a
        // candidate's bytes (` PhD` against the start ` Ph`), boosting the shorter piece
        // only moves the decoder onto another spelling of the same word, so such a
        // candidate is skipped.
        let argmax_is_candidate = cand.iter().any(|(c, _)| i64::from(*c) == argmax);
        let top: Option<&[u8]> = (p.prefix_guard && !argmax_is_candidate)
            .then(|| i32::try_from(argmax).ok())
            .flatten()
            .filter(|&id| id >= 0 && id < self.eot)
            .and_then(|id| self.vocab.token(id));
        for &(id, bonus) in cand {
            let Ok(signed) = i32::try_from(id) else {
                continue;
            };
            if signed >= self.eot {
                continue;
            }
            let piece = self.vocab.token(signed).unwrap_or_default();
            if top.is_some_and(|t| t.len() > piece.len() && t.starts_with(piece)) {
                continue;
            }
            if let Some(l) = logits.get_mut(id as usize) {
                *l += bonus.min(p.cap);
            }
        }
    }
}

fn is_open_punct(b: u8) -> bool {
    matches!(b, b'(' | b'[' | b'/' | b'"' | b'\'')
}

/// How many lanes [`best_of`] keeps.
const LANES: usize = 8;

/// The row's largest logit and the first index that holds it, or `(-inf, -1)` when no
/// logit exceeds `-inf`; a `NaN` never wins. The same answer as one scan with `>`, but
/// kept in eight lanes the compiler can vectorize: the whole row is read once per decoder
/// step, and the plain scan took 56 µs of every step in a release build [M].
fn best_of(logits: &[f32]) -> (f32, i64) {
    let mut lanes = [f32::NEG_INFINITY; LANES];
    let mut at = [usize::MAX; LANES];
    let chunks = logits.chunks_exact(LANES);
    let rest = chunks.remainder();
    for (c, chunk) in chunks.enumerate() {
        for i in 0..LANES {
            if chunk[i] > lanes[i] {
                lanes[i] = chunk[i];
                at[i] = c;
            }
        }
    }
    // Each lane holds the first index of its own largest value; among lanes with equal
    // values the smallest index wins, as it would in one scan.
    let mut best = f32::NEG_INFINITY;
    let mut argmax = -1i64;
    for i in 0..LANES {
        if at[i] == usize::MAX {
            continue;
        }
        let k = (at[i] * LANES + i) as i64;
        if lanes[i] > best || (lanes[i] == best && k < argmax) {
            best = lanes[i];
            argmax = k;
        }
    }
    let base = logits.len() - rest.len();
    for (j, &l) in rest.iter().enumerate() {
        if l > best {
            best = l;
            argmax = (base + j) as i64;
        }
    }
    (best, argmax)
}

/// The shim's callback: `user` is a `&BonusTable` that outlives the decode.
///
/// # Safety
/// `token_ids` holds `n_tokens` ids, `logits` holds `n_vocab` floats, and `user` points at
/// a live `BonusTable`; whisper.cpp guarantees the first two for the call.
pub unsafe extern "C" fn bonus_callback(
    token_ids: *const i32,
    n_tokens: i32,
    logits: *mut f32,
    n_vocab: i32,
    user: *mut c_void,
) {
    if user.is_null() || logits.is_null() || n_vocab <= 0 {
        return;
    }
    let tokens: &[i32] = if token_ids.is_null() || n_tokens <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(token_ids, n_tokens as usize)
    };
    let logits = std::slice::from_raw_parts_mut(logits, n_vocab as usize);
    let table = &*user.cast::<BonusTable>();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| table.apply(tokens, logits)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::BiasEntry;
    use std::time::Duration;

    /// A small vocabulary: every byte, a few merges, then end of text and timestamps.
    fn vocab() -> Arc<Vocab> {
        let mut tokens: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
        for piece in [" EP", "AS", "1", " the", " Zy", " Zyq", "EP"] {
            tokens.push(piece.as_bytes().to_vec());
        }
        let eot = tokens.len() as i32;
        Arc::new(Vocab::new(tokens, eot))
    }

    fn id(v: &Vocab, piece: &str) -> i32 {
        v.id(piece.as_bytes()).expect(piece)
    }

    fn table(terms: &[&str]) -> BonusTable {
        let set = SetBias {
            revision: 1,
            scope: "public".into(),
            entries: terms
                .iter()
                .map(|t| BiasEntry {
                    written: t.to_string(),
                    bias: "auto".into(),
                })
                .collect(),
            params: BonusParams::default(),
        };
        BonusTable::build(&set, vocab())
    }

    /// A row of logits a little above zero everywhere, with `top` the model's choice.
    fn row(n: usize, top: i32) -> Vec<f32> {
        let mut logits: Vec<f32> = (0..n).map(|i| (i % 7) as f32 * 0.01).collect();
        logits[top as usize] = 3.0;
        logits
    }

    #[test]
    fn a_term_in_progress_is_continued_and_started() {
        let t = table(&["EPAS1"]);
        let v = vocab();
        let n = v.eot() as usize + 10;
        let ts = v.eot() + 5;
        let ep = id(&v, " EP");
        let as_ = id(&v, "AS");
        // After `<ts> the`, a start piece close to the top gets the start bonus.
        let mut logits = row(n, id(&v, " the"));
        logits[ep as usize] = 1.5;
        t.apply(&[ts, id(&v, " the")], &mut logits);
        assert!(
            (logits[ep as usize] - 3.5).abs() < 1e-6,
            "{}",
            logits[ep as usize]
        );
        // After ` EP`, `AS` continues the form: bonus 5 * min(1, 2 / 2).
        let mut logits = row(n, id(&v, " the"));
        logits[as_ as usize] = 0.0;
        t.apply(&[ts, ep], &mut logits);
        assert!(
            (logits[as_ as usize] - 5.0).abs() < 1e-6,
            "{}",
            logits[as_ as usize]
        );
    }

    /// End of text and timestamps are never touched, and a timestamp ends a match: ` EP
    /// <ts>` followed by `AS` is not a continuation.
    #[test]
    fn never_at_or_above_end_of_text() {
        let t = table(&["EPAS1"]);
        let v = vocab();
        let n = v.eot() as usize + 10;
        let ts = v.eot() + 5;
        let mut logits = row(n, v.eot());
        let before = logits.clone();
        t.apply(&[ts, id(&v, " EP"), ts], &mut logits);
        assert_eq!(logits[v.eot() as usize..], before[v.eot() as usize..]);
        let as_ = id(&v, "AS");
        assert!(logits[as_ as usize] <= before[as_ as usize] + 2.0 + 1e-6);
        assert_ne!(logits[as_ as usize], before[as_ as usize] + 5.0);
    }

    /// `first = 0`: before the window's first text token there is no start bonus.
    #[test]
    fn no_start_bonus_before_the_first_text_token() {
        let t = table(&["EPAS1"]);
        let v = vocab();
        let n = v.eot() as usize + 10;
        let ts = v.eot() + 5;
        let ep = id(&v, " EP");
        let mut logits = row(n, id(&v, " the"));
        logits[ep as usize] = 2.0;
        let before = logits.clone();
        t.apply(&[ts], &mut logits);
        assert_eq!(logits, before);
    }

    /// The prefix steal guard: the model's ` Zyq` extends the start piece ` Zy` of
    /// `Zyxwab` without being a candidate itself, so ` Zy` is left alone.
    #[test]
    fn the_prefix_steal_guard() {
        let t = table(&["Zyxwab"]);
        let v = vocab();
        let n = v.eot() as usize + 10;
        let ts = v.eot() + 5;
        let zy = id(&v, " Zy");
        let zyq = id(&v, " Zyq");
        let mut logits = row(n, zyq);
        logits[zy as usize] = 2.5;
        t.apply(&[ts, id(&v, " the")], &mut logits);
        assert_eq!(logits[zy as usize], 2.5);
        // When the top token does not extend it, the same piece is boosted.
        let mut logits = row(n, id(&v, " the"));
        logits[zy as usize] = 2.5;
        t.apply(&[ts, id(&v, " the")], &mut logits);
        assert_eq!(logits[zy as usize], 4.5);
    }

    /// `best_of` answers what one scan with `>` answers: the largest value and its first
    /// index, `NaN` never winning, `-0.0` and `0.0` equal, on rows of every length around
    /// the lane count and on Whisper's.
    #[test]
    fn best_of_matches_one_scan() {
        fn scan(logits: &[f32]) -> (f32, i64) {
            let mut best = (f32::NEG_INFINITY, -1i64);
            for (k, &l) in logits.iter().enumerate() {
                if l > best.0 {
                    best = (l, k as i64);
                }
            }
            best
        }
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let specials = [
            f32::NAN,
            f32::NEG_INFINITY,
            f32::INFINITY,
            0.0,
            -0.0,
            3.0,
            -3.0,
        ];
        let mut rows: Vec<Vec<f32>> = vec![
            vec![],
            vec![f32::NAN; 9],
            vec![f32::NEG_INFINITY; 17],
            vec![2.0; 23],
            vec![-0.0, 0.0, -0.0, 0.0, 0.0, -0.0, 0.0, -0.0, 0.0],
        ];
        for len in (0..40).chain([51_865, 51_866]) {
            for _ in 0..25 {
                rows.push(
                    (0..len)
                        .map(|_| match next() % 10 {
                            0 => specials[(next() % specials.len() as u64) as usize],
                            // Few distinct values, so ties across lanes are common.
                            _ => (next() % 7) as f32 - 3.0,
                        })
                        .collect(),
                );
            }
        }
        for row in &rows {
            let (want, got) = (scan(row), best_of(row));
            assert_eq!(want.1, got.1, "{row:?}");
            assert!(
                want.0.to_bits() == got.0.to_bits() || (want.0.is_nan() && got.0.is_nan()),
                "{want:?} {got:?}"
            );
        }
    }

    /// The same table from several threads gives the same rows, and a call stays well
    /// under 100 µs with a vocabulary the size of Whisper's.
    #[test]
    fn reentrant_and_fast() {
        let v = vocab();
        let terms: Vec<String> = (0..1000).map(|i| format!("EPAS{i}")).collect();
        let refs: Vec<&str> = terms.iter().map(String::as_str).collect();
        let t = Arc::new(table(&refs));
        let n = 51_866;
        let ts = v.eot() + 5;
        let tokens = vec![ts, id(&v, " the"), id(&v, " EP"), id(&v, "AS")];
        let expected = {
            let mut logits = row(n, id(&v, " the"));
            t.apply(&tokens, &mut logits);
            logits
        };
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let t = Arc::clone(&t);
                let tokens = tokens.clone();
                let expected = expected.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        let mut logits = row(n, id(&vocab(), " the"));
                        t.apply(&tokens, &mut logits);
                        assert_eq!(logits, expected);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("no panic");
        }
        let mut logits = row(n, id(&v, " the"));
        let rounds = 200;
        // The callback reads the row once, for its best logit, and then touches a few
        // candidates. One plain pass over the row, timed in this same build and
        // interleaved with the calls so both meet the same load, is the yardstick: a
        // callback that reads the row several times fails on any machine and in any
        // profile, a debug build in CI included.
        let (mut calls_time, mut pass_time) = (Duration::ZERO, Duration::ZERO);
        for _ in 0..rounds {
            let started = Instant::now();
            t.apply(&tokens, &mut logits);
            calls_time += started.elapsed();
            let started = Instant::now();
            let mut best = (f32::NEG_INFINITY, -1i64);
            for (k, &l) in std::hint::black_box(&logits).iter().enumerate() {
                if l > best.0 {
                    best = (l, k as i64);
                }
            }
            std::hint::black_box(best);
            pass_time += started.elapsed();
        }
        let (per_call, one_pass) = (calls_time / rounds, pass_time / rounds);
        eprintln!(
            "bonus callback: {per_call:?} per call over a 51,866 entry row; one pass over the \
             row: {one_pass:?}"
        );
        assert!(
            per_call <= one_pass * 3 + Duration::from_micros(50),
            "{per_call:?} per call against {one_pass:?} for one pass over the row"
        );
        // The plan's 100 µs is the shipped build's budget; a debug build spends about
        // 240 µs on the one pass alone [M].
        if !cfg!(debug_assertions) {
            assert!(per_call.as_micros() < 100, "{per_call:?} per call");
        }
        let (calls, nanos) = t.usage();
        assert!(calls >= 201 && nanos > 0);
    }

    /// PR-14's acceptance of DD-46, with the real model: the decoder bonus over 1,000
    /// terms, no prompt, on the synthetic benchmark's 36 term clips, reaches 70 % strict
    /// recall of the 14 targets or more with small.en (the spike measured 72 % [M]), and
    /// the callback stays cheap. Run by hand:
    ///
    /// ```text
    /// BIOROUTER_DICTATION_BENCH_DIR=<folder with audio/, sentences.tsv, dict_1000.tsv> \
    /// BIOROUTER_DICTATION_BENCH_MODEL=<ggml-small.en(-q8_0).bin> \
    /// BIOROUTER_DICTATION_BENCH_VAD=<ggml-silero-v6.2.0.bin> \
    /// cargo test -p biorouter-dictation --features engine --release --lib -- \
    ///   --ignored bonus_at_1000_terms --nocapture
    /// ```
    ///
    /// Two tables are measured. The spike's: all 1,000 terms, as the spike boosted them,
    /// which is what the 70 % floor holds. And the shipped bias set's: the terms whose
    /// bonus policy is `auto` and that do not collide with English (DD-46), which leaves
    /// out PHD2 (its letters are `phd`), SPOKE and CRISPRi unless the person pins them; its
    /// recall is reported beside the floor.
    #[test]
    #[ignore = "needs a real model and the benchmark audio"]
    fn bonus_at_1000_terms_on_the_benchmark_audio() {
        use crate::engine::{decode_params, vad_params, Engine, LoadOptions};
        use crate::protocol::BiasEntry;

        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let dir = std::path::PathBuf::from(var("BIOROUTER_DICTATION_BENCH_DIR"));
        let model = std::path::PathBuf::from(var("BIOROUTER_DICTATION_BENCH_MODEL"));
        let vad = std::path::PathBuf::from(var("BIOROUTER_DICTATION_BENCH_VAD"));
        let targets = [
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
        let count = |text: &str, term: &str| -> usize {
            text.match_indices(term)
                .filter(|(at, _)| {
                    let before = text.get(..*at).and_then(|s| s.chars().next_back());
                    let after = text.get(at + term.len()..).and_then(|s| s.chars().next());
                    !before.is_some_and(char::is_alphanumeric)
                        && !after.is_some_and(char::is_alphanumeric)
                })
                .count()
        };
        let sentences = std::fs::read_to_string(dir.join("sentences.tsv")).expect("sentences");
        let dictionary = std::fs::read_to_string(dir.join("dict_1000.tsv")).expect("dictionary");
        let entries = |shipped: bool| -> Vec<BiasEntry> {
            dictionary
                .lines()
                .filter_map(|line| line.split('\t').next())
                .filter(|w| !w.is_empty())
                .filter(|w| !shipped || !crate::recognize::collides(w))
                .map(|w| BiasEntry {
                    written: w.to_string(),
                    bias: "auto".into(),
                })
                .collect()
        };
        let _lock = crate::engine::test_support::engine_lock();
        let mut engine = Engine::load(&LoadOptions {
            model_path: &model,
            model_size: std::fs::metadata(&model).expect("the model").len(),
            vad_path: &vad,
            use_gpu: cfg!(all(target_os = "macos", target_arch = "aarch64")),
            n_threads: 8,
        })
        .expect("the model loads");
        let vocab = Arc::new(engine.vocabulary());
        let abort = std::sync::atomic::AtomicI32::new(0);
        let mut clips: Vec<(String, String, Vec<f32>)> = Vec::new();
        for line in sentences.lines() {
            let mut cols = line.split('\t');
            let (Some(id), Some(reference)) = (cols.next(), cols.next()) else {
                continue;
            };
            if !id.starts_with('s') {
                continue;
            }
            for voice in ["Daniel", "Karen", "Samantha"] {
                let path = dir.join("audio").join(format!("{id}_{voice}.wav"));
                let bytes = std::fs::read(&path).expect("a clip");
                let wav = crate::audio::parse_wav_s16(&bytes).expect("16 bit PCM");
                let pcm: Vec<f32> = wav
                    .to_16k_mono()
                    .iter()
                    .map(|&s| f32::from(s) / 32768.0)
                    .collect();
                clips.push((format!("{id}_{voice}"), reference.to_string(), pcm));
            }
        }
        // The shipped decode (DD-47, DD-48: timestamps on, the detector on), and the spike's
        // (timestamps off, no detector), which the 70 % floor was measured with.
        let shipped = (decode_params("en", 8), vad_params());
        let spike = (
            crate::engine::DecodeParams {
                no_timestamps: true,
                ..decode_params("en", 8)
            },
            crate::engine::VadParams {
                enabled: false,
                ..vad_params()
            },
        );
        // One decode to warm the pipelines before anything is timed.
        let _ = engine.transcribe(&clips[0].2, &shipped.0, &shipped.1, &abort);
        // The 14 targets as the token prompt, best last: what a person's own dictionary of
        // these terms sends, for the latency the prompt adds on the path from stop to text.
        let (prompt_ids, prompt_used) = crate::recognize::prompt::fit(
            &targets.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            150,
            |text| vocab.encode(text),
        );
        assert_eq!(prompt_used, targets.len());
        let mut recall_of = |label: &str,
                             decode: &(crate::engine::DecodeParams, crate::engine::VadParams),
                             prompt: &[i32],
                             table: Option<&BonusTable>|
         -> f64 {
            let (mut hits, mut total, mut decode_ms) = (0, 0, 0.0f64);
            let before = table.map(BonusTable::usage).unwrap_or_default();
            for (name, reference, pcm) in &clips {
                let started = Instant::now();
                let out = engine
                    .transcribe_with(pcm, &decode.0, &decode.1, prompt, table, &abort)
                    .expect("decodes");
                decode_ms += started.elapsed().as_secs_f64() * 1000.0;
                for term in targets {
                    let expected = count(reference, term);
                    total += expected;
                    hits += expected.min(count(&out.text, term));
                }
                if table.is_some() {
                    eprintln!("{label} {name}: {}", out.text);
                }
            }
            let recall = hits as f64 / total.max(1) as f64;
            let after = table.map(BonusTable::usage).unwrap_or_default();
            let callbacks = (after.0 - before.0, after.1 - before.1);
            eprintln!(
                "{label}: strict recall {hits}/{total} ({:.0} %), decode {:.0} ms per clip; \
                 {} callbacks, {:.1} µs each, {:.2} ms per utterance",
                100.0 * recall,
                decode_ms / clips.len() as f64,
                callbacks.0,
                callbacks.1 as f64 / callbacks.0.max(1) as f64 / 1000.0,
                callbacks.1 as f64 / clips.len() as f64 / 1e6,
            );
            recall
        };
        recall_of("no bonus, shipped decode", &shipped, &[], None);
        recall_of("no bonus, spike decode", &spike, &[], None);
        recall_of(
            "the 14 term prompt alone, shipped decode",
            &shipped,
            &prompt_ids,
            None,
        );
        let mut tables = Vec::new();
        for (label, filtered) in [
            ("the spike's 1,000 terms", false),
            ("the shipped bias set", true),
        ] {
            let set = SetBias {
                revision: 1,
                scope: "public".into(),
                entries: entries(filtered),
                params: BonusParams::default(),
            };
            let built = Instant::now();
            let table = BonusTable::build(&set, Arc::clone(&vocab));
            eprintln!(
                "{label}: {} terms, {} nodes, built in {:.1} ms",
                set.entries.len(),
                table.nodes(),
                built.elapsed().as_secs_f64() * 1000.0
            );
            tables.push((label, table));
        }
        let floor = recall_of(
            "the spike's 1,000 terms, spike decode",
            &spike,
            &[],
            Some(&tables[0].1),
        );
        recall_of(
            "the spike's 1,000 terms, shipped decode",
            &shipped,
            &[],
            Some(&tables[0].1),
        );
        recall_of(
            "the shipped bias set, shipped decode",
            &shipped,
            &[],
            Some(&tables[1].1),
        );
        recall_of(
            "the 14 term prompt and the shipped bias set, shipped decode",
            &shipped,
            &prompt_ids,
            Some(&tables[1].1),
        );
        assert!(floor >= 0.70, "strict recall {floor:.2}");
    }

    #[test]
    fn the_callback_survives_bad_input() {
        let t = table(&["EPAS1"]);
        let mut logits = vec![0.0f32; 300];
        // SAFETY: the pointers are valid for the lengths given, or null.
        unsafe {
            bonus_callback(
                std::ptr::null(),
                0,
                logits.as_mut_ptr(),
                300,
                std::ptr::from_ref(&t).cast_mut().cast(),
            );
            bonus_callback(
                std::ptr::null(),
                0,
                logits.as_mut_ptr(),
                300,
                std::ptr::null_mut(),
            );
            bonus_callback(
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                300,
                std::ptr::from_ref(&t).cast_mut().cast(),
            );
        }
    }
}
