//! The knowledge-base privacy tier (issue #56, design §9.3 B4).
//!
//! A knowledge base takes the tier of the most sensitive session that has
//! ingested into it. This module owns the store, the monotone raise and the
//! refusal; Task 10B calls `raise` from the four choke points and Task 10C
//! calls [`assert_reachable`] from the same four.
//!
//! A `bool` rather than an enum because `biorouter-mcp` cannot depend on
//! `biorouter`, where `ProviderTier` lives — see the task's decision (1).
//! `caller_is_private == true` is exactly `floor(Private) == Private`.
//!
//! ## Locking
//!
//! Every mutator here is `_unlocked` and takes NO lock. The knowledge root has
//! exactly one lock, `KnowledgeService::lock_root`, and both it and
//! `FileLockGuard` are private to the `service` module — a free function cannot
//! reach them, and `create_base`/`import_brkb`/`delete_base` are already inside
//! it when they call these. A second `FileLockGuard::acquire` in the same
//! process blocks forever (it opens a fresh fd and `flock`s exclusively).
//! Callers OUTSIDE the service use `KnowledgeService::raise_tier` /
//! `forget_tier`, which take the lock and delegate here. This is the tree's own
//! `_unlocked` convention — twenty helpers in `service.rs` already follow it.
//!
//! ## Known residuals
//!
//! ⚠ An earlier version of this block said `lock_root()` was in-process, that
//! two Biorouter processes could therefore lose a raise, and that closing it
//! "needs an OS advisory lock the tree does not have anywhere". All three are
//! false: `FileLockGuard::acquire` calls `file.lock_exclusive()` through
//! `fs2::FileExt` (`service.rs`), which IS flock/`LockFileEx` — cross-process,
//! and the same mechanism the paragraph above correctly relies on when it
//! explains why a second acquire in one process blocks. It is corrected rather
//! than deleted because inviting someone to "fix" a race the tree already closes
//! is worse than saying nothing, and because it obscured the two that are real:
//!
//! 1. **Readers take no lock, by design.** `is_private` and `has_entry_unlocked`
//!    never `flock`; they rely on the store being replaced only by `rename`, so
//!    a read sees the old file or the new one and never a torn one. What that
//!    does not give is freshness: a permit decided microseconds before a raise
//!    lands is still acted on. That is check-then-use, it is the same window
//!    Task 10A's own coverage note records for `primary_kb_for_context`, and
//!    closing it would mean holding the root lock across every knowledge read.
//! 2. **The lock is advisory and file-scoped.** Anything that edits `.kb-tiers`
//!    without going through `KnowledgeService` — a text editor, another tool —
//!    bypasses it entirely. The task accepts that: the file is machine-local and
//!    user-writable, and the threat model is "a public model reads private
//!    notes", not "a local attacker with a shell".

use crate::knowledge::affiliation::{CallerAffiliation, KbAffiliation};
use crate::knowledge::types::KbTier;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Bumped 1 -> 2 by DR-26 / Task 50 Step 1, which adds [`Store::affiliations`].
///
/// ⚠ **A version stamp, not a gate, and deliberately so.** Nothing reads it —
/// the store's whole downgrade-safety argument (see [`Store::provenance`]) is
/// that serde ignores unknown fields, so an older binary reads `bases` exactly
/// as before and never sees the new map. A reader that *refused* an unfamiliar
/// schema number would read the file as unparseable, which [`load`] treats as
/// PRIVATE for every base at once — locking a user out of all their knowledge
/// bases the first time they ran yesterday's build. The stamp is here so a human
/// reading the file, or a future migration that genuinely needs to branch, can
/// tell the two shapes apart.
///
/// The "migration" is therefore the absence of the new map: every base written
/// before this axis existed has no recorded owners and is reachable from every
/// private model. That is the **Missing** direction of the discipline below —
/// a fact ("this build never recorded any"), matching the tier migration's
/// fail-open (AR-2) — and it is not the same as the **Unreadable** direction,
/// where the answer is unknown and reads restrictively
/// ([`KbAffiliation::Unknown`]).
const SCHEMA: u32 = 2;

pub(super) const PUBLIC: &str = "public";
pub(super) const PRIVATE: &str = "private";

/// The meta key the daemon writes the caller's capability into. Defined here,
/// not in `server.rs`, because `agent_drafter` (CP4) reads the same key and two
/// spellings of it is exactly how a barrier silently stops working.
pub const CAPABILITY_TIER_META_KEY: &str = "biorouter-capability-tier";

/// The result meta key a built-in server sets when the result it returns was
/// read out of private content (DR-33, design §2.6), and the key beside it that
/// names the bases. The daemon's dispatch choke point reads both
/// ([`private_content_bases`]) and ratchets the CALLER's chat to Private before
/// the result is returned, with the reason `kb:<base id>`.
///
/// A mark rather than a ratchet here because this crate cannot reach the
/// session store; `dispatch_tool_call` is the one choke point every model's
/// call passes through, and it already ratchets there for private extensions.
pub const PRIVATE_CONTENT_META_KEY: &str = "privateContent";
pub const PRIVATE_CONTENT_BASES_META_KEY: &str = "privateContentKnowledgeBases";

/// Mark `result` as carrying content read out of the private `bases`. Nothing
/// is marked for an empty list.
pub fn mark_private_content(result: &mut rmcp::model::CallToolResult, bases: &[String]) {
    if bases.is_empty() {
        return;
    }
    let meta = result.meta.get_or_insert_with(rmcp::model::Meta::new);
    meta.0.insert(
        PRIVATE_CONTENT_META_KEY.to_string(),
        serde_json::Value::Bool(true),
    );
    meta.0.insert(
        PRIVATE_CONTENT_BASES_META_KEY.to_string(),
        serde_json::Value::Array(
            bases
                .iter()
                .map(|id| serde_json::Value::String(id.clone()))
                .collect(),
        ),
    );
}

/// The private bases a marked result was read from, or `None` when it carries
/// no mark. A mark with no readable list still counts: the content was private
/// even if the ids were lost, and the reader is ratcheted as for `unknown`.
pub fn private_content_bases(meta: Option<&rmcp::model::Meta>) -> Option<Vec<String>> {
    let meta = meta?;
    if meta.0.get(PRIVATE_CONTENT_META_KEY) != Some(&serde_json::Value::Bool(true)) {
        return None;
    }
    let bases = meta
        .0
        .get(PRIVATE_CONTENT_BASES_META_KEY)
        .and_then(|v| v.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(bases)
}

/// The private bases among `ids`, in order: what a call reading them gives a
/// caller that it must be told about ([`mark_private_content`]). Not a
/// reachability question, which only [`assert_reachable`] answers: the caller
/// has already been admitted to every id here.
pub fn private_among(root: &Path, ids: impl IntoIterator<Item = String>) -> Vec<String> {
    ids.into_iter().filter(|id| is_private(root, id)).collect()
}

/// Names no base, no page and no snippet. Constant, so a model that retries sees
/// the same string and stops rather than looping.
pub const KB_PRIVATE_REFUSAL: &str = "\
This knowledge base is private: a session running an institutional or self-hosted model has \
ingested into it, so only a private model may read or write it. This session is running on a \
public model. Ask the user to switch this chat to a private model (Settings > Models, or the \
model chip in the composer) and try again. Do not retry with a different knowledge base id, \
through an export, or through a raw-source search; the boundary is the same everywhere.";

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct Store {
    schema: u32,
    /// kb id -> "public" | "private". An id ABSENT from a store that exists,
    /// for a directory that DOES exist, is unknown provenance and reads
    /// PRIVATE; the whole file being absent means the migration has not run.
    pub(super) bases: BTreeMap<String, String>,
    /// kb id -> how the value in `bases` got there, for the entries a USER moved
    /// (issue #56 DR-18 / Task 29A). Absent for every base the ratchet placed,
    /// which is the common case and carries no provenance worth writing down.
    ///
    /// ⚠ **A second map rather than a richer value type, and the reason is
    /// downgrade safety.** Turning `bases`' value into `{tier, reason,
    /// changed_at}` would make a store written by this version unparseable to
    /// every older binary — and `load` reads unparseable as PRIVATE, so
    /// installing yesterday's build would lock the user out of every knowledge
    /// base at once. Serde ignores unknown fields, so an older binary reads the
    /// `bases` map exactly as before and simply never sees this one. The pairing
    /// is maintained by both writers: [`raise_unlocked`] and [`forget_unlocked`]
    /// drop the entry here whenever they move or remove the tier, so a stale
    /// reason can never outlive the value it describes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) provenance: BTreeMap<String, Provenance>,
    /// kb id -> the institutions whose content this base holds (issue #56,
    /// DR-26 / Task 50 Step 1). The **union of what it ingested**, and monotone:
    /// [`raise_affiliation_unlocked`] only ever adds, and only
    /// [`forget_unlocked`] removes, when the base itself goes away.
    ///
    /// ⚠ **Not an allowlist.** Every id here is an OWNER the reader has to be
    /// covered by, so a base holding two institutions' content is reachable from
    /// neither of their models — see
    /// [`crate::knowledge::affiliation::reachable`]. Reading it as "who may
    /// reach this", which is what the extension side of DR-26 means by a set, is
    /// the mistake that fails open.
    ///
    /// A THIRD sibling map for the reason [`Self::provenance`] is a second one:
    /// an older binary parses `bases` unchanged and simply never sees this
    /// field, where a richer value type would make the whole file unparseable to
    /// it — and [`load`] reads unparseable as PRIVATE for every base at once.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) affiliations: BTreeMap<String, BTreeSet<String>>,
}

/// How a base's tier came to hold the value it holds — written only when a
/// **user** moved it, so a released base is never indistinguishable from one
/// that was always public.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Provenance {
    /// `publicized_by_user` / `privatized_by_user`. A word, not a sentence: it
    /// is read by a support conversation six months later and by nothing else.
    pub(super) reason: String,
    /// RFC 3339, UTC.
    pub(super) changed_at: String,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            bases: BTreeMap::new(),
            provenance: BTreeMap::new(),
            affiliations: BTreeMap::new(),
        }
    }
}

/// One row of `.kb-tiers` as a reader sees it: the tier every gate enforces,
/// plus the provenance if a user put it there.
///
/// The tier is taken from [`is_private`] rather than from the raw word, so the
/// fail-closed rules have exactly one implementation and this cannot come to
/// disagree with the barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierEntry {
    pub tier: KbTier,
    pub reason: Option<String>,
    pub changed_at: Option<String>,
}

/// Read one base's tier together with its provenance.
///
/// Lock-free, like every other reader here: the store is only ever replaced by
/// `rename`, so a read sees the old file or the new one and never a torn one.
pub fn entry(root: &Path, kb_id: &str) -> TierEntry {
    let prov = match load(root) {
        StoreState::Parsed(store) => store.provenance.get(kb_id).cloned(),
        StoreState::Missing | StoreState::Unreadable => None,
    };
    TierEntry {
        tier: KbTier::from_is_private(is_private(root, kb_id)),
        reason: prov.as_ref().map(|p| p.reason.clone()),
        changed_at: prov.map(|p| p.changed_at),
    }
}

/// What a read of `.kb-tiers` found. `Missing` and `Unreadable` are deliberately
/// distinct: the first means the migration has not run (⇒ everything public,
/// AR-2's accepted fail-open direction), the second means the answer is unknown
/// (⇒ fail closed).
enum StoreState {
    Missing,
    Parsed(Store),
    Unreadable,
}

fn load(root: &Path) -> StoreState {
    let p = super::paths::kb_tiers_path(root);
    let raw = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StoreState::Missing,
        Err(e) => {
            tracing::error!(
                "knowledge: cannot read {} ({e}); every base reads PRIVATE until it is readable",
                p.display()
            );
            return StoreState::Unreadable;
        }
    };
    match serde_json::from_str::<Store>(&raw) {
        Ok(store) => StoreState::Parsed(store),
        Err(e) => {
            tracing::error!(
                "knowledge: {} is not parseable ({e}); every base reads PRIVATE until it is",
                p.display()
            );
            StoreState::Unreadable
        }
    }
}

/// The store as it must be before a write.
///
/// A missing file is an empty store. An **unreadable** one is refused: silently
/// replacing a store we cannot parse would erase every ratchet in it, which is
/// the one direction this module exists to prevent.
pub(super) fn load_for_write(root: &Path) -> Result<Store> {
    match load(root) {
        StoreState::Missing => Ok(Store::default()),
        StoreState::Parsed(mut store) => {
            // The schema-1 -> 2 migration, and the whole of it: the next write
            // stamps the current version. Nothing branches on the number (see
            // [`SCHEMA`]), so this is a label being brought up to date rather
            // than a transformation — the new axis's absence already means
            // "no owners recorded", which is the correct reading of every store
            // written before it existed.
            store.schema = SCHEMA;
            Ok(store)
        }
        StoreState::Unreadable => anyhow::bail!(
            "the knowledge-base tier store is unreadable, so it cannot be updated without \
             erasing the classifications already in it. Repair or remove {}",
            super::paths::kb_tiers_path(root).display()
        ),
    }
}

/// `manifest::save`'s idiom: write a sibling tmp file and `rename` over the
/// target, so a reader sees the old file or the new one, never a torn one.
///
/// `pub(super)` — visible inside `crate::knowledge` and nowhere else — because
/// [`crate::knowledge::tier_user::set_unlocked`], the one writer permitted to
/// LOWER a tier, cannot go through [`raise_unlocked`] (which is monotone by
/// construction and must stay that way) and so needs the raw write. That reach
/// is pinned at exactly one caller by
/// `tier_user::tests::exactly_one_writer_outside_the_ratchet_saves_the_tier_store`.
pub(super) fn save(root: &Path, store: &Store) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let path = super::paths::kb_tiers_path(root);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(store)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn tier_word(caller_is_private: bool) -> &'static str {
    if caller_is_private {
        PRIVATE
    } else {
        PUBLIC
    }
}

/// The value the daemon writes under [`CAPABILITY_TIER_META_KEY`].
///
/// The KEY was already shared so the two sides cannot drift; the VALUE was not —
/// `mcp_client.rs` spelled "private"/"public" itself and [`caller_is_private`]
/// compared against this module's own consts, and a test on each side asserting
/// its own literal would have caught neither half of a drift. One spelling, one
/// function, both sides.
pub fn capability_meta_value(caller_is_private: bool) -> &'static str {
    tier_word(caller_is_private)
}

/// The caller's capability, PUBLIC unless the meta says private.
///
/// Absent means one of: an older daemon, a non-built-in transport, or a direct
/// unit-test construction. All three are "unknown", and unknown must be the
/// restrictive answer for the reads Task 10C gates.
pub fn caller_is_private(meta: &rmcp::model::Meta) -> bool {
    meta.0
        .get(CAPABILITY_TIER_META_KEY)
        .and_then(|v| v.as_str())
        == Some(PRIVATE)
}

/// One-time migration. Every base that exists when this first runs becomes
/// PUBLIC (fail-open, DR-10 and AR-2). Guarded by the file's absence: re-running
/// it on every startup would re-add a base whose entry a later `forget` removed,
/// and would race the ratchet.
pub fn ensure_migrated_unlocked(root: &Path) -> Result<()> {
    if super::paths::kb_tiers_path(root).exists() {
        return Ok(());
    }
    let mut store = Store::default();
    if root.exists() {
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // The sidecar directories (`.hidden-kb-sessions`, …) all start with
            // a dot and so fail id validation; nothing else in the root is a
            // knowledge base.
            if super::paths::validate_kb_id(&name).is_err() {
                continue;
            }
            store.bases.insert(name, PUBLIC.to_string());
        }
    }
    save(root, &store)
}

/// PRIVATE unless the store says otherwise — with one exception that is the
/// point of decision (3): a kb id with **no entry and no directory under
/// `root`** is not private, because there is nothing there to leak and refusing
/// would ban a public session from creating or importing a base.
///
/// An explicit entry always wins, directory or not: `raise` registers ids that
/// have not been created yet (an import lands on one), and a registration that
/// a missing directory could override would not be a ratchet.
///
/// Otherwise fail-closed on: no entry for a directory that exists, an
/// unparseable file, an unreadable file. Each of the last two logs at `error!`.
/// Lock-free: the store is only ever replaced by `rename`, so a reader sees the
/// old file or the new one, never a torn one.
pub fn is_private(root: &Path, kb_id: &str) -> bool {
    match load(root) {
        // The migration has not run, so every base is public (AR-2).
        StoreState::Missing => false,
        StoreState::Parsed(store) => match store.bases.get(kb_id) {
            // Only the exact word `public` permits. Anything else parseable but
            // unrecognised — a hand-edit, a future schema's word read by an
            // older binary — is unknown, and unknown fails in the same direction
            // as an unparseable file.
            Some(tier) => tier != PUBLIC,
            // Unknown provenance if the base is really there; nothing to leak
            // if it is not.
            None => super::paths::kb_root(root, kb_id).exists(),
        },
        StoreState::Unreadable => super::paths::kb_root(root, kb_id).exists(),
    }
}

/// Whose content this base holds (issue #56, DR-26 / Task 50 Step 1).
///
/// Lock-free, like every other reader here.
///
/// The three states of [`load`] map onto DR-26's discipline exactly as the tier
/// does, and the mapping is the whole of Step 0's "fail closed" warning:
///
/// * **Missing** — the store has never been written, so nothing was ever
///   recorded. `Owners(∅)`: no institution claims this content, and inventing a
///   constraint nobody wrote down would lock a user out of their own notes.
///   Matches the tier's fail-open migration (AR-2).
/// * **Parsed** — whatever the ratchet recorded, `Owners(∅)` for a base it never
///   touched.
/// * **Unreadable** — the answer is *unknown*, which is emphatically not "no
///   constraint": [`KbAffiliation::Unknown`], reachable only from a local model.
///   The one exception is a base with no directory, which has nothing to leak
///   and must stay creatable — decision (3)'s third direction, the same
///   exception [`is_private`] makes.
pub fn affiliation(root: &Path, kb_id: &str) -> KbAffiliation {
    match load(root) {
        StoreState::Missing => KbAffiliation::Owners(BTreeSet::new()),
        StoreState::Parsed(store) => KbAffiliation::Owners(
            store
                .affiliations
                .get(kb_id)
                .cloned()
                .unwrap_or_else(BTreeSet::new),
        ),
        StoreState::Unreadable => {
            if super::paths::kb_root(root, kb_id).exists() {
                KbAffiliation::Unknown
            } else {
                KbAffiliation::Owners(BTreeSet::new())
            }
        }
    }
}

/// Monotone, on DR-26's third axis. Adds the caller's institution to the set of
/// institutions whose content this base holds, and can never remove one.
///
/// ⚠ **It ratchets wherever [`raise_unlocked`] does, and that pairing is the
/// control.** A write that raised the tier and not the affiliation would put an
/// institution's content into a base that no institution is recorded as owning
/// — which reads as unclaimed and is therefore reachable from every other
/// institution's model.
///
/// Production has **three** `raise_unlocked` sites and each is paired by a
/// different mechanism, which is worth stating because an earlier version of
/// this comment claimed a single test covered all of them and shipped two
/// unpaired:
///
/// 1. `KnowledgeServer::call_tool`'s ratchet, for the seventeen tools that name
///    an existing base — pinned through production by
///    `server::tests::every_tool_that_ratchets_the_tier_also_records_the_callers_institution`,
///    which drives the SAME probe table the tier ratchet is proved with. (A grep
///    census over call sites was considered and rejected there: it would have
///    counted test constructions and said nothing about whether the raise
///    actually ran.)
/// 2. `create_base_as` and 3. `import_brkb`, the two tools whose subject id is
///    minted BY the call and which that parameterisation is therefore
///    structurally blind to — `every_tool_the_router_exposes_is_classified_by_the_probe_table`
///    chains both past the table by name. They are paired **structurally**
///    instead, by `KnowledgeService::stamp_base_unlocked`: one function that
///    does both, so there is no second call site to forget.
///
/// [`CallerAffiliation::Local`] and [`CallerAffiliation::Unstated`] add nothing,
/// for two different reasons that happen to agree:
///
/// * `Local` genuinely contributes no institution's data — the content came off
///   this machine, and no agreement governs it.
/// * `Unstated` is the absence of an answer, and the alternative (recording a
///   sentinel owner) would make the base **permanently** unreachable from every
///   institutional model with no declassification path, which AR-1 rejects. It
///   is the same direction the tier ratchet already takes for an absent `_meta`
///   key, where the caller collapses to public and nothing is raised. What
///   protects the base is the barrier, not this: an unstated caller cannot READ
///   a base any institution claims.
///
/// DR-15 / AR-7: with the master toggle off, nothing ratchets — a ratchet that
/// kept firing would be a deferred, permanent impact on a feature the user
/// turned off, and this axis is monotone in exactly the way the tier is.
pub fn raise_affiliation_unlocked(
    root: &Path,
    kb_id: &str,
    caller: &CallerAffiliation,
) -> Result<()> {
    add_owners_unlocked(
        root,
        kb_id,
        crate::knowledge::affiliation::contributed_owners(caller),
    )
}

/// Whether the two classification ratchets run at all (DR-15 / AR-7's master
/// switch).
///
/// ⚠ **One spelling, three readers, and the third is why it exists.**
/// [`raise_unlocked`] and [`add_owners_unlocked`] are the two writes; the
/// KB-to-KB merge's **dry run** is a reader of the same decision — it has to
/// report what those two would do without doing it
/// (`KnowledgeService::projected_classification`). A preview that read the
/// atomic itself would be a third read of a process-global that another thread
/// can flip between them, and a preview that disagrees with the write it
/// previews is exactly the failure a preview exists to prevent.
///
/// It is deliberately **not** the read in [`assert_reachable`]. That one answers
/// "may this caller reach this base", which is a different question with a
/// different failure direction; folding the two would let one edit silently
/// change both the barrier and the ratchet.
pub fn ratchets_are_live() -> bool {
    crate::privacy_toggle::privacy_tiers_enabled()
}

/// The same ratchet, given the owners directly rather than a caller to derive
/// one from — for `import_brkb`, where the institutions being added are the
/// ones the **archive** carried and there is no single caller to name.
///
/// One writer for the map, so "does a stamp ever remove an owner?" is decided in
/// exactly one place; [`raise_affiliation_unlocked`] is the one-owner case of
/// this function and nothing else touches [`Store::affiliations`] except
/// [`forget_unlocked`], which drops the whole row when the base goes away.
pub fn add_owners_unlocked(root: &Path, kb_id: &str, owners: BTreeSet<String>) -> Result<()> {
    // DR-15 / AR-7: with the master toggle off, nothing ratchets — see
    // [`raise_affiliation_unlocked`]'s closing paragraph.
    if !ratchets_are_live() {
        return Ok(());
    }
    if owners.is_empty() {
        return Ok(());
    }
    let mut store = load_for_write(root)?;
    let recorded = store.affiliations.entry(kb_id.to_string()).or_default();
    let before = recorded.len();
    recorded.extend(owners);
    if recorded.len() == before {
        // Every owner was already recorded. Skipping the write keeps a
        // read-heavy path from rewriting the store on every tool call.
        return Ok(());
    }
    save(root, &store)
}

/// Apply the monotone tier and owner ratchets in one atomic store replacement.
///
/// A classification is one security decision even though the downgrade-safe
/// on-disk schema keeps its axes in sibling maps. Loading and saving once means
/// lock-free readers cannot observe a tier without the owners contributed by
/// the same write, or vice versa.
pub fn stamp_unlocked(
    root: &Path,
    kb_id: &str,
    caller_is_private: bool,
    owners: BTreeSet<String>,
) -> Result<()> {
    let ratchets_live = ratchets_are_live();
    let caller_is_private = caller_is_private && ratchets_live;
    let mut store = load_for_write(root)?;
    let mut changed = false;

    match store.bases.get(kb_id) {
        Some(current) if current != PUBLIC => {}
        Some(_) if caller_is_private => {
            store.bases.insert(kb_id.to_string(), PRIVATE.to_string());
            store.provenance.remove(kb_id);
            changed = true;
        }
        Some(_) => {}
        None => {
            store
                .bases
                .insert(kb_id.to_string(), tier_word(caller_is_private).to_string());
            store.provenance.remove(kb_id);
            changed = true;
        }
    }

    if ratchets_live && !owners.is_empty() {
        let recorded = store.affiliations.entry(kb_id.to_string()).or_default();
        let before = recorded.len();
        recorded.extend(owners);
        changed |= recorded.len() != before;
    }

    if changed {
        save(root, &store)
    } else {
        Ok(())
    }
}

/// The words a knowledge-base cross-institutional mismatch is stated in.
///
/// DR-26 makes the warning the product: "this may be a compliance risk" is a
/// shrug, and a user can only accept a risk that was stated to them. So it names
/// the institution(s) whose content the base holds and the institution whose
/// agreements cover the bound model — and, like [`KB_PRIVATE_REFUSAL`], no base
/// id, no page and no snippet (§14.4).
///
/// ⚠ **The remedy it offers is to change the model, not to approve the flow, and
/// that is a narrowing of DR-26 recorded rather than hidden.** The
/// cross-affiliation grant (`biorouter::privacy::grant`) is keyed on
/// (session, EXTENSION, model affiliation) and lives in the session database,
/// which this crate cannot reach — `assert_reachable` is a synchronous function
/// in `biorouter-mcp` with no `SessionManager`, by the same crate-boundary
/// constraint this whole task exists to widen. A KB-scoped grant needs a fourth
/// carrier across that boundary and a subject that is a base rather than a
/// connector; until it exists, the actionable statement is the one the model can
/// act on, which is the shape [`KB_PRIVATE_REFUSAL`] already takes.
fn cross_affiliation_refusal(owners: &BTreeSet<String>, caller: &CallerAffiliation) -> String {
    let held_by = if owners.is_empty() {
        "an institution this build cannot determine, because the classification store could not be read"
            .to_string()
    } else {
        owners.iter().cloned().collect::<Vec<_>>().join(", ")
    };
    let model_side = match caller {
        CallerAffiliation::Institution(id) => {
            format!("this chat is bound to a model covered by {id}'s agreements")
        }
        // Naming an institution here would be a lie, and "we cannot tell" is
        // what the user can act on. Same position `affiliation::unstated_model`
        // takes on the extension side.
        CallerAffiliation::Unstated => {
            "this chat is bound to a private model that does not state whose agreements cover it"
                .to_string()
        }
        // Unreachable: a local caller clears every base (`reachable` returns
        // before comparing). Written as a real arm rather than `unreachable!()`
        // because a refusal composer is reached from a refusal path, and a panic
        // there converts a control into an outage.
        CallerAffiliation::Local => "this chat is bound to a local model".to_string(),
    };
    format!(
        "Cross-institutional data flow. This knowledge base holds content belonging to {held_by}, \
         and {model_side}. Reading or writing it would send that content across an institutional \
         boundary. Compliance does not transfer between institutions: a model approved at one has \
         no permission over another's data unless a BAA, DUA or IRB approval covers this specific \
         flow. Ask the user to switch this chat to a model covered by that institution's \
         agreements: Settings > Models, or the model chip in the composer. Do not retry with a \
         different knowledge base id, through an export, or through a raw-source search; the \
         boundary is the same everywhere."
    )
}

/// The single refusal for CP1..CP4. `Ok(())` permits.
///
/// DR-15's master opt-out is read HERE, at the choke point, and nowhere above
/// it. Every caller — the three knowledge macros, `KnowledgeServer`'s own
/// `assert_kb_reachable`, the `/knowledge/*` routes, the apps runtime's KB frame
/// — delegates the whole decision to this function, so one read governs all of
/// them and none of them needs a second. A caller that read the toggle itself
/// *and* called this would be the two-reads race in miniature.
///
/// A direct read of the process-global rather than a `CallCapability`: this
/// crate cannot see `biorouter`, which is exactly why the atomic lives in
/// `crate::privacy_toggle` (see that module).
pub fn assert_reachable(
    root: &Path,
    kb_id: &str,
    caller_is_private: bool,
    caller_affiliation: &CallerAffiliation,
) -> Result<()> {
    // DR-15's master opt-out, read ONCE for both axes. Neither the tier check
    // below nor DR-26's runs with the feature off, and reading the toggle twice
    // is the two-reads race in miniature.
    if !crate::privacy_toggle::privacy_tiers_enabled() {
        return Ok(());
    }
    if !caller_is_private {
        // The tier axis has the whole answer for a public caller: it is refused
        // outright if the base is private, and if the base is public there is no
        // institutional boundary to cross either. Stating a second, different
        // reason for one crossing is what
        // `gate_cross_affiliation`'s tier guard avoids on the extension side.
        return if is_private(root, kb_id) {
            anyhow::bail!(KB_PRIVATE_REFUSAL)
        } else {
            Ok(())
        };
    }
    // Issue #56 DR-26 / Task 50 Step 1: the caller is private, so the tier axis
    // permits and the remaining question is *under whose agreements*.
    let held = affiliation(root, kb_id);
    if crate::knowledge::affiliation::reachable(caller_affiliation, &held) {
        return Ok(());
    }
    anyhow::bail!(cross_affiliation_refusal(
        held.owners().unwrap_or(&EMPTY_OWNERS),
        caller_affiliation
    ))
}

/// The owner set a refusal names when there is none to name — an unreadable
/// store, where [`KbAffiliation::Unknown`] carries no institutions.
static EMPTY_OWNERS: std::sync::LazyLock<BTreeSet<String>> =
    std::sync::LazyLock::new(BTreeSet::new);

/// Monotone. Registers `kb_id` at the caller's tier if absent, raises it to
/// private if the caller is private, and can never lower an existing ENTRY.
///
/// ⚠ "Absent entry" is not the same as "public". A base with an entry-less
/// **directory** reads private by inference (decision 3, and
/// `a_base_that_never_went_through_create_or_import_reads_private`), so a
/// public caller registering it here does move it from private to public. That
/// is deliberate for the one caller that owns the directory it is stamping —
/// `create_base_as` creates it in the same transaction, and refusing there
/// would lock a public session out of the base it just made — and it is closed
/// for the callers that do not: Task 10C's `assert_reachable` sits on the line
/// above every ratchet (CP1, CP2, CP3), so a public session's write to a base
/// that reads private is refused before it can reach this function.
/// **10C must keep that ordering**; a raise placed above the barrier at any
/// choke point re-opens it.
///
/// DR-15 / AR-7: with the master toggle off, nothing ratchets. What stops is the
/// **raise**; registering an absent base at PUBLIC continues, because that is the
/// fail-open default (AR-2) and skipping it would leave a base created while the
/// feature was off reading PRIVATE the moment it is turned back on — a
/// classification nobody asked for, written by the absence of a write. Collapsing
/// the caller to public is the whole of the change, so the monotonicity argument
/// above is untouched: an existing private entry is still never lowered.
pub fn raise_unlocked(root: &Path, kb_id: &str, caller_is_private: bool) -> Result<()> {
    let caller_is_private = caller_is_private && ratchets_are_live();
    let mut store = load_for_write(root)?;
    match store.bases.get(kb_id) {
        // Anything that is not the exact word `public` already reads private
        // (see `is_private`), so a raise is a no-op and must not overwrite it —
        // replacing an unrecognised word with `public` would be a lowering.
        Some(current) if current != PUBLIC => return Ok(()),
        Some(_) if !caller_is_private => return Ok(()),
        _ => {}
    }
    store
        .bases
        .insert(kb_id.to_string(), tier_word(caller_is_private).to_string());
    // The ratchet has just decided this base's tier, so any `publicized_by_user`
    // a user left behind now describes a value that is gone. Publicizing is not
    // an exemption from the ratchet
    // (`tier_user::tests::the_ratchet_still_ratchets_after_a_publicize`), and an
    // entry reading Private under `publicized_by_user` would be an audit trail
    // that says the opposite of what happened.
    store.provenance.remove(kb_id);
    save(root, &store)
}

/// Register `kb_id` as PUBLIC **only if it has no entry** (decision 5a): a base
/// with no entry reads private by decision (3), so an unregistered base would
/// lock its own creator out. Never lowers.
///
/// Exactly [`raise_unlocked`] with `caller_is_private = false`, which is what
/// `create_base` and `import_brkb` now call — they need the *private* case too,
/// and one primitive means one place where "does a stamp lower an entry?" is
/// decided. This one is kept as the name for the intent, and
/// `register_public_never_lowers_an_already_private_base` is what pins the two
/// to the same answer.
pub fn register_public_if_absent_unlocked(root: &Path, kb_id: &str) -> Result<()> {
    let mut store = load_for_write(root)?;
    if store.bases.contains_key(kb_id) {
        return Ok(());
    }
    store.bases.insert(kb_id.to_string(), PUBLIC.to_string());
    save(root, &store)
}

/// Whether the store already carries an entry for `kb_id`, whether or not the
/// base exists on disk.
///
/// `brkb::import`'s collision loop asks this alongside its directory test: an
/// entry can exist without a directory (`raise_unlocked` registers ids that have
/// not been created — decision (3)'s third direction), and an import that landed
/// on one would take its classification from a base that never existed.
pub fn has_entry_unlocked(root: &Path, kb_id: &str) -> bool {
    match load(root) {
        StoreState::Missing => false,
        StoreState::Parsed(store) => store.bases.contains_key(kb_id),
        // NOT `true`: this answer drives a `while` loop, and "every id is taken"
        // spins forever. An unreadable store fails closed at the only place it
        // matters anyway — `is_private` reads any existing directory as private
        // when it cannot parse the file, and the import has just created one.
        StoreState::Unreadable => false,
    }
}

pub(super) fn has_metadata_unlocked(root: &Path, kb_id: &str) -> Result<bool> {
    match load(root) {
        StoreState::Missing => Ok(false),
        StoreState::Parsed(store) => Ok(store.bases.contains_key(kb_id)
            || store.provenance.contains_key(kb_id)
            || store.affiliations.contains_key(kb_id)),
        StoreState::Unreadable => anyhow::bail!(
            "the knowledge-base tier store is unreadable, so deletion coherence cannot be verified"
        ),
    }
}

/// Carry a base's classification across a RENAME, on all three axes.
///
/// ⚠ **Without this, renaming a knowledge base declassifies it.** Every map
/// here is keyed by kb id, and `KnowledgeService::update_base` moves the
/// directory, the registry entry, the persisted primary and the hidden-KB
/// pointers — so a rename left the tier row stranded under the old id and the
/// new id carrying nothing.
///
/// The tier itself survived that by luck rather than design: [`is_private`]
/// reads an id it does not know, whose directory exists, as PRIVATE. The
/// AFFILIATION did not. [`affiliation`] answers `Owners(∅)` for an unknown id,
/// and an empty owner set is *unclaimed* rather than *nobody's*, which
/// `affiliation::reachable` admits from every private model. So renaming a base
/// that held one institution's data made it readable by another institution's
/// private model, with nothing on screen to mark the change — DR-26's barrier
/// undone by an edit to a display name. The provenance row went with it, taking
/// the record of who set the tier and why.
///
/// Merges rather than overwrites when the destination already carries a row,
/// and in the fail-safe direction on both axes: private wins over public, and
/// the owner sets are unioned. A rename onto an occupied id should not be
/// reachable — `update_base` refuses a target that already exists — but the
/// alternative to merging here is silently choosing one classification over
/// another, and this is not the place to make that choice.
pub fn rename_unlocked(root: &Path, from: &str, to: &str) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let mut store = load_for_write(root)?;

    let moved_tier = store.bases.remove(from);
    let moved_provenance = store.provenance.remove(from);
    let moved_owners = store.affiliations.remove(from);
    if moved_tier.is_none() && moved_provenance.is_none() && moved_owners.is_none() {
        return Ok(());
    }

    if let Some(tier) = moved_tier {
        // Private wins: the destination is almost always absent, and when it is
        // not, the more restrictive answer is the only safe one.
        let keep_private = tier == PRIVATE || store.bases.get(to).is_some_and(|t| t == PRIVATE);
        store.bases.insert(
            to.to_string(),
            if keep_private { PRIVATE } else { &tier }.to_string(),
        );
    }
    if let Some(provenance) = moved_provenance {
        store.provenance.entry(to.to_string()).or_insert(provenance);
    }
    if let Some(owners) = moved_owners {
        store
            .affiliations
            .entry(to.to_string())
            .or_default()
            .extend(owners);
    }
    save(root, &store)
}

/// Drop the entry when the base is deleted, so a later base reusing the id is
/// classified by its own creator rather than by a base that no longer exists.
///
/// ⚠ This is also the ONLY thing that removes an owning institution: nothing
/// else writes `store.affiliations` downward, so an affiliation stamp is
/// permanent for the life of the base. That asymmetry is why a raise must land
/// only where a write actually happens — see `macros::lint`, where a read-only
/// scan used to stamp a base it never modified.
pub fn forget_unlocked(root: &Path, kb_id: &str) -> Result<()> {
    let mut store = load_for_write(root)?;
    // Both halves, and the provenance one FIRST so the early return below cannot
    // strand it: an id can carry a user's reason with no tier entry only if
    // something went wrong, and leaving it would hand a base created later under
    // the same id a history that is not its own.
    let had_provenance = store.provenance.remove(kb_id).is_some();
    // Issue #56 DR-26 / Task 50 Step 1: the third map, for the reason the second
    // one is dropped here. An id reused after a delete must be classified by its
    // own creator on BOTH axes — inheriting the owners of a base that no longer
    // exists would make a fresh base unreachable from the model that just made
    // it, with nothing on screen to explain why.
    let had_affiliation = store.affiliations.remove(kb_id).is_some();
    if store.bases.remove(kb_id).is_none() && !had_provenance && !had_affiliation {
        return Ok(());
    }
    save(root, &store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Returns the `TempDir` alongside the root: dropping it deletes the
    /// directory, so the plan's `let root = tempdir_with_bases(..)` would unlink
    /// the tree before the first assertion ran.
    fn tempdir_with_bases(ids: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_path_buf();
        for id in ids {
            let kb_root = root.join(id);
            std::fs::create_dir_all(kb_root.join("knowledge")).unwrap();
            std::fs::write(kb_root.join("manifest.yaml"), current_okf_manifest(id)).unwrap();
            std::fs::write(kb_root.join("schema.md"), "# Schema\n").unwrap();
            std::fs::write(kb_root.join("index.md"), "# Index\n").unwrap();
            std::fs::write(kb_root.join("log.md"), "# Log\n").unwrap();
        }
        (d, root)
    }

    fn current_okf_manifest(id: &str) -> String {
        serde_yaml::to_string(&serde_json::json!({
            "id": id,
            "name": id,
            "schema_version": crate::knowledge::types::CURRENT_SCHEMA_VERSION,
            "format": "okf",
        }))
        .unwrap()
    }

    fn tiers_file_exists(root: &Path) -> bool {
        crate::knowledge::paths::kb_tiers_path(root).exists()
    }

    fn seed_page(root: &Path, kb_id: &str, rel: &str, body: &str) {
        let p = root.join(kb_id).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn page_body(root: &Path, kb_id: &str, rel: &str) -> String {
        std::fs::read_to_string(root.join(kb_id).join(rel)).unwrap()
    }

    fn export_brkb_with_provenance(root: &Path, kb_id: &str) -> Vec<u8> {
        crate::knowledge::service::KnowledgeService::new(root.to_path_buf())
            .export_brkb(kb_id)
            .unwrap()
    }

    fn import_brkb_as(root: &Path, bytes: &[u8], importer_is_private: bool) -> String {
        import_brkb_as_full(
            root,
            bytes,
            importer_is_private,
            &CallerAffiliation::Unstated,
        )
    }

    fn import_brkb_as_full(
        root: &Path,
        bytes: &[u8],
        importer_is_private: bool,
        importer_affiliation: &CallerAffiliation,
    ) -> String {
        crate::knowledge::service::KnowledgeService::new(root.to_path_buf())
            .import_brkb(bytes, importer_is_private, importer_affiliation)
            .unwrap()
    }

    fn zip_names(bytes: &[u8]) -> Vec<String> {
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        (0..a.len())
            .map(|i| a.by_index(i).unwrap().name().to_string())
            .collect()
    }

    fn archive(id: &str, marker: Option<bool>) -> Vec<u8> {
        archive_inner(
            id,
            marker.map(|private| {
                serde_json::json!({
                    "schema": 3,
                    "tier": if private { "private" } else { "public" },
                    "owners": [],
                    "format": "okf",
                })
                .to_string()
            }),
        )
    }

    fn archive_with_raw_marker(id: &str, raw: &str) -> Vec<u8> {
        archive_inner(id, Some(raw.to_string()))
    }

    fn archive_inner(id: &str, marker: Option<String>) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file(format!("{id}/knowledge/x.md"), opts)
                .unwrap();
            zip.write_all(b"body").unwrap();
            for required in ["schema.md", "index.md", "log.md"] {
                zip.start_file(format!("{id}/{required}"), opts).unwrap();
                zip.write_all(b"current OKF fixture").unwrap();
            }
            zip.start_file(format!("{id}/manifest.yaml"), opts).unwrap();
            zip.write_all(current_okf_manifest(id).as_bytes()).unwrap();
            if let Some(m) = marker {
                zip.start_file(format!("{id}/.brkb-provenance"), opts)
                    .unwrap();
                zip.write_all(m.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn an_unmigrated_root_migrates_every_existing_base_to_public_exactly_once() {
        // AR-2, made executable. The tree keeps no record of which session wrote
        // which page — the git author is "Biorouter", not a session id — so there
        // is nothing to reason from and the migration fails OPEN, like the session
        // backfill (DR-10). It must run ONCE: a startup-repeating "add any base
        // missing from the file as public" would un-ratchet a base the day after a
        // private session raised it, which is Task 38's bug in a different store.
        let (_d, root) = tempdir_with_bases(&["default", "omop"]);
        assert!(!tiers_file_exists(&root));

        ensure_migrated_unlocked(&root).unwrap();
        assert!(!is_private(&root, "default"));
        assert!(!is_private(&root, "omop"));

        raise_unlocked(&root, "omop", /* caller_is_private */ true).unwrap();
        ensure_migrated_unlocked(&root).unwrap(); // second launch
        assert!(
            is_private(&root, "omop"),
            "the migration re-ran and lowered a tier"
        );
    }

    #[test]
    fn a_base_that_never_went_through_create_or_import_reads_private() {
        // Fail-closed, and it is the difference between "known public" and
        // "unknown". A store that listed only the private ids could not tell them
        // apart, which is why the file is a map and not a list like `.hidden-kbs`.
        let (_d, root) = tempdir_with_bases(&["default"]);
        ensure_migrated_unlocked(&root).unwrap();
        std::fs::create_dir_all(root.join("dropped-in-by-hand")).unwrap();
        assert!(is_private(&root, "dropped-in-by-hand"));
        assert!(!is_private(&root, "default"));
    }

    #[test]
    fn a_base_that_does_not_exist_on_disk_is_reachable_by_anyone() {
        // Decision (3), third direction — and the bug the sixteen-site enumeration
        // encoded: `kb_create_base` and `kb_import` name a base that does not exist
        // yet, so a barrier that reads "no entry ⇒ private" bans a public session
        // from ever creating or importing one. There is no content to leak from a
        // directory that is not there.
        let (_d, root) = tempdir_with_bases(&["default"]);
        ensure_migrated_unlocked(&root).unwrap();
        assert!(!is_private(&root, "not-created-yet"));
        assert_reachable(
            &root,
            "not-created-yet",
            /* caller_is_private */ false,
            &crate::knowledge::affiliation::CallerAffiliation::Unstated,
        )
        .unwrap();
    }

    #[test]
    fn raise_is_monotone_and_registers_an_absent_base_at_the_callers_tier() {
        let (_d, root) = tempdir_with_bases(&[]);
        ensure_migrated_unlocked(&root).unwrap();

        raise_unlocked(&root, "fresh", false).unwrap(); // created from a public chat
        assert!(!is_private(&root, "fresh"));
        raise_unlocked(&root, "fresh", true).unwrap(); // a private chat writes to it
        assert!(is_private(&root, "fresh"));
        raise_unlocked(&root, "fresh", false).unwrap(); // and a public chat writes again
        assert!(
            is_private(&root, "fresh"),
            "a public write lowered the tier"
        );

        raise_unlocked(&root, "born-private", true).unwrap();
        assert!(is_private(&root, "born-private"));
    }

    #[test]
    fn register_public_never_lowers_an_already_private_base() {
        // `create_base` registers unconditionally (decision 5a). If that registration
        // could overwrite, then re-creating a deleted id — or any future caller that
        // registers twice — would launder a private base to public.
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop", true).unwrap();
        register_public_if_absent_unlocked(&root, "omop").unwrap();
        assert!(
            is_private(&root, "omop"),
            "registration lowered a ratcheted base"
        );
    }

    #[test]
    fn deleting_a_base_forgets_its_tier_so_the_id_can_be_reused() {
        // Otherwise `kb_create_base("omop")` from a public chat, after a private
        // `omop` was deleted, silently inherits Private and the user cannot see why.
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop", true).unwrap();
        forget_unlocked(&root, "omop").unwrap();
        raise_unlocked(&root, "omop", false).unwrap();
        assert!(!is_private(&root, "omop"));
    }

    #[test]
    fn the_store_is_written_atomically_and_never_leaves_a_tmp_file() {
        // manifest.rs:17-24's idiom. A torn write here reads as "no entry", which
        // fails CLOSED and locks the user out of their own knowledge base.
        let (_d, root) = tempdir_with_bases(&["default"]);
        ensure_migrated_unlocked(&root).unwrap();

        // ⚠ The final-state half below — the file is there, no `*.tmp` beside it
        // — is satisfied just as well by a plain `fs::write` straight to the
        // target, which is the very implementation this test exists to reject.
        // The inode is what tells the two apart: `rename` puts a DIFFERENT file
        // at the path, while `fs::write` truncates the one already there and
        // keeps its inode. That is the atomicity, observed rather than described.
        #[cfg(unix)]
        let before = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(crate::knowledge::paths::kb_tiers_path(&root))
                .unwrap()
                .ino()
        };

        raise_unlocked(&root, "default", true).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let after = std::fs::metadata(crate::knowledge::paths::kb_tiers_path(&root))
                .unwrap()
                .ino();
            assert_ne!(
                before, after,
                "the store was written in place: a reader can see a half-written file"
            );
        }

        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().any(|n| n == ".kb-tiers"));
        assert!(!names.iter().any(|n| n.ends_with(".tmp")));
    }

    #[test]
    fn a_private_export_cannot_be_laundered_by_importing_it_into_a_public_chat() {
        // The two-call bypass, end to end and in the ONE direction the previous
        // tests never ran: export PRIVATE, import PUBLIC. Before decision (2)'s
        // marker the imported base takes the importer's tier and every page of a
        // private base is readable by a public model, with no gate crossed —
        // `kb_export` is permitted (the base is the private caller's own) and
        // `kb_import` is permitted (the id it creates does not exist yet).
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop", true).unwrap();
        seed_page(&root, "omop", "knowledge/x.md", "SENTINEL-COHORT-N-412");

        let bytes = export_brkb_with_provenance(&root, "omop"); // private base
        let new_id = import_brkb_as(&root, &bytes, /* importer_is_private */ false);

        assert_ne!(new_id, "omop", "the collision loop must land on a fresh id");
        assert!(
            is_private(&root, &new_id),
            "a private base was laundered into a public one by export/import"
        );
        // And the content really did arrive — otherwise the assertion above is
        // satisfied by an import that failed.
        assert!(page_body(&root, &new_id, "knowledge/x.md").contains("SENTINEL-COHORT-N-412"));
    }

    #[test]
    fn the_provenance_marker_can_only_raise_and_unverifiable_archives_are_refused() {
        // Decision (2)'s whole safety argument, as three rows. The marker is
        // attacker-supplied by construction — it rides inside the zip — and is safe
        // ONLY because it is read as a floor.
        let (_d, root) = tempdir_with_bases(&[]);
        ensure_migrated_unlocked(&root).unwrap();

        // (a) marker private + importer public  -> private   (the laundering case)
        assert!(is_private(
            &root,
            &import_brkb_as(&root, &archive("a", Some(true)), false)
        ));
        // (b) marker PUBLIC + importer private  -> private   (a hostile archive
        //     claiming "public" cannot lower the importing session's own tier)
        assert!(is_private(
            &root,
            &import_brkb_as(&root, &archive("b", Some(false)), true)
        ));
        // (c/d) A missing or malformed marker cannot prove either tier or owner
        // set, so importing it as public/unowned would be a laundering path.
        for bytes in [archive("c", None), archive_with_raw_marker("d", "yes")] {
            let error = crate::knowledge::service::KnowledgeService::new(root.clone())
                .import_brkb(&bytes, false, &CallerAffiliation::Unstated)
                .unwrap_err();
            assert!(
                error
                    .downcast_ref::<crate::knowledge::brkb::InvalidKnowledgeArchive>()
                    .is_some(),
                "{error:#}"
            );
        }
    }

    #[test]
    fn the_marker_rides_in_the_archive_and_never_lands_on_disk() {
        // It is written straight into the ZipWriter after `walk` (brkb.rs:16-17),
        // so the KB's git tree does not gain an untracked file — and it goes INSIDE
        // the single top-level directory, because `import` (:70-87) bails unless
        // there is exactly one and a sibling entry would break every archive.
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop", true).unwrap();
        let bytes = export_brkb_with_provenance(&root, "omop");
        assert!(zip_names(&bytes).contains(&"omop/.brkb-provenance".to_string()));
        assert!(
            !root.join("omop/.brkb-provenance").exists(),
            "the marker was written to disk"
        );
        // …and it is not extracted back out into the imported base either.
        let new_id = import_brkb_as(&root, &bytes, false);
        assert!(!root.join(&new_id).join(".brkb-provenance").exists());
    }

    /// Issue #56 DR-26 / Task 50. The store side of the export/import laundering
    /// path: an archive of a claimed base carries the claim, and importing it
    /// UNIONS it with the importer's own institution.
    ///
    /// ⚠ The union, not the archive's set alone and not the importer's alone. A
    /// Stanford chat importing a UCSF archive lands a base owned by both, which
    /// is out of reach of both their models — the fail-closed direction, and the
    /// same disjunction the tier takes one axis over.
    #[test]
    fn an_archive_carries_its_owners_and_an_import_unions_them_with_the_importers() {
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop", true).unwrap();
        raise_affiliation_unlocked(&root, "omop", &bound("ucsf")).unwrap();
        let bytes = export_brkb_with_provenance(&root, "omop");

        // Imported by a Stanford chat: both institutions now own it.
        let landed = import_brkb_as_full(&root, &bytes, true, &bound("stanford"));
        let owners = affiliation(&root, &landed);
        let owners = owners.owners().expect("a readable store");
        assert!(
            owners.contains("ucsf") && owners.contains("stanford"),
            "{owners:?}"
        );
        assert!(!crate::knowledge::affiliation::reachable(
            &bound("stanford"),
            &affiliation(&root, &landed)
        ));

        // …and imported by a UCSF chat it is still just UCSF's, or the union is
        // indistinguishable from "claim everything".
        let mine = import_brkb_as_full(&root, &bytes, true, &bound("ucsf"));
        let owners = affiliation(&root, &mine);
        assert_eq!(
            owners.owners().expect("a readable store"),
            &["ucsf".to_string()].into_iter().collect::<BTreeSet<_>>()
        );
        assert!(crate::knowledge::affiliation::reachable(
            &bound("ucsf"),
            &affiliation(&root, &mine)
        ));
    }

    /// The one case with nothing honest to stamp into an archive: an unreadable
    /// store, where whose content the base holds is *unknown* rather than
    /// "nobody's". Writing an empty owner set there would let a corrupt store
    /// launder every base on the machine, so the export refuses.
    #[test]
    fn a_base_whose_owners_cannot_be_read_is_not_exportable() {
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        std::fs::write(crate::knowledge::paths::kb_tiers_path(&root), "{ not json").unwrap();
        assert_eq!(affiliation(&root, "omop"), KbAffiliation::Unknown);
        let err = crate::knowledge::service::KnowledgeService::new(root.clone())
            .export_brkb("omop")
            .expect_err("a base whose owners cannot be established was packaged anyway");
        let msg = format!("{err:#}");
        assert!(msg.contains("unreadable"), "{msg}");
    }

    #[test]
    fn an_unrecognised_tier_word_reads_private_and_no_public_write_replaces_it() {
        // The file is machine-local and user-writable, which the task accepts —
        // but "parseable JSON carrying a word we do not recognise" must fail in
        // the same direction as "unparseable file" (PRIVATE), not the opposite
        // one. `tier == PRIVATE` read a typo as public; `tier != PUBLIC` does
        // not, and only the exact word `public` can ever permit.
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        std::fs::write(
            crate::knowledge::paths::kb_tiers_path(&root),
            r#"{"schema":1,"bases":{"omop":"privte"}}"#,
        )
        .unwrap();

        assert!(is_private(&root, "omop"), "a typo'd tier word read PUBLIC");
        // …and the ratchet treats it as already at least as restrictive as
        // private, so a public write cannot overwrite it into a permit.
        raise_unlocked(&root, "omop", false).unwrap();
        assert!(
            is_private(&root, "omop"),
            "a public write replaced an unrecognised word"
        );
    }

    #[test]
    fn an_import_never_lands_on_an_id_that_only_the_tier_store_still_knows() {
        // `brkb::import`'s collision loop suffixes while the DIRECTORY exists,
        // and the plan's justification for stamping a tier after the import
        // ("it always lands on a fresh id") is true for directories and false
        // for entries. `raise_unlocked` registers ids that have no directory —
        // that is decision (3)'s third direction and Task 10B will do it at the
        // choke points — so an entry can outlive, or precede, any base. Landing
        // on one would classify freshly imported content by a base that never
        // existed: fail-closed if the ghost is private, but decided by the wrong
        // thing either way.
        let (_d, root) = tempdir_with_bases(&[]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "orig", true).unwrap(); // an entry, no directory
        assert!(!root.join("orig").exists());

        let new_id = import_brkb_as(&root, &archive("orig", Some(false)), false);
        assert_ne!(new_id, "orig", "the import landed on a ghost entry's id");
        assert!(
            !is_private(&root, &new_id),
            "a fresh public import was classified by a base that never existed"
        );
        // The ghost is untouched — the import neither adopted nor cleared it.
        assert!(is_private(&root, "orig"));
    }

    #[test]
    fn the_refusal_names_no_base_and_no_page() {
        // One string serves CP1..CP4, so it is asserted once, here.
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop", true).unwrap();
        let s = assert_reachable(
            &root,
            "omop",
            false,
            &crate::knowledge::affiliation::CallerAffiliation::Unstated,
        )
        .unwrap_err()
        .to_string();
        assert!(s.contains("private model"));
        assert!(!s.contains("omop"), "the refusal named the base: {s}");
    }

    // ------------------------------------------------- DR-26 / Task 50 Step 1:
    // a base's affiliation is the union of what it ingested, and it ratchets.

    use crate::knowledge::affiliation::{CallerAffiliation, KbAffiliation};

    fn bound(name: &str) -> CallerAffiliation {
        CallerAffiliation::Institution(name.to_string())
    }

    fn owners_of(root: &Path, kb_id: &str) -> Vec<String> {
        match affiliation(root, kb_id) {
            KbAffiliation::Owners(o) => o.into_iter().collect(),
            KbAffiliation::Unknown => panic!("the owners of {kb_id} were unreadable"),
        }
    }

    /// ⚠ **Renaming a knowledge base must not declassify it.**
    ///
    /// Every map here is keyed by kb id, and `update_base` moves the directory,
    /// the registry, the primary pointer and the hidden-KB refs. It did not move
    /// this store, and the damage was not where it looks. The TIER survived by
    /// accident: `is_private` reads an unknown id whose directory exists as
    /// private. The AFFILIATION did not — `affiliation` answers `Owners(∅)` for
    /// an unknown id, and an empty owner set is *unclaimed*, which
    /// `affiliation::reachable` admits from EVERY private model.
    ///
    /// So the assertion that matters is the owners one. A test that checked only
    /// `is_private` would have passed against the bug.
    #[test]
    fn renaming_a_base_carries_its_tier_and_its_owners_with_it() {
        let (_d, root) = tempdir_with_bases(&["omop-cohort"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop-cohort", true).unwrap();
        add_owners_unlocked(&root, "omop-cohort", ["ucsf".to_string()].into()).unwrap();

        std::fs::create_dir_all(root.join("omop-cohort-2024").join("knowledge")).unwrap();
        rename_unlocked(&root, "omop-cohort", "omop-cohort-2024").unwrap();

        assert!(
            is_private(&root, "omop-cohort-2024"),
            "the renamed base must still be private"
        );
        assert_eq!(
            owners_of(&root, "omop-cohort-2024"),
            vec!["ucsf".to_string()],
            "and must still be claimed by the institution whose data it holds; an empty \
             owner set is UNCLAIMED, which every private model may reach"
        );
    }

    /// The old id must not keep a classification the base no longer has, or a
    /// base created later under that id inherits a history that is not its own.
    /// This is the same reason `forget_unlocked` drops all three maps.
    #[test]
    fn a_rename_leaves_nothing_behind_under_the_old_id() {
        let (_d, root) = tempdir_with_bases(&["before"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "before", true).unwrap();
        add_owners_unlocked(&root, "before", ["ucsf".to_string()].into()).unwrap();

        std::fs::create_dir_all(root.join("after").join("knowledge")).unwrap();
        rename_unlocked(&root, "before", "after").unwrap();
        // The directory is gone in production; remove it so `is_private`'s
        // "unknown id with a live directory reads private" rule cannot mask the
        // question being asked.
        std::fs::remove_dir_all(root.join("before")).unwrap();

        assert!(
            !is_private(&root, "before"),
            "the old id must carry no tier"
        );
        assert!(owners_of(&root, "before").is_empty(), "and no owners");
    }

    /// DR-26's sentence, made executable: "a knowledge base's affiliation is the
    /// **union** of what they ingested and ratchets like classification".
    ///
    /// The headline row of Task 50's gate is the last assertion — a base holding
    /// two institutions' content is reachable from **neither** of their models,
    /// because a model matching only one of the owners is a mismatch. That is
    /// the opposite of the extension side's allowlist, and it is what an
    /// implementation reaching for `contains` gets wrong.
    #[test]
    fn a_bases_affiliation_is_the_union_of_the_institutions_that_wrote_into_it() {
        let (_d, root) = tempdir_with_bases(&["shared"]);
        ensure_migrated_unlocked(&root).unwrap();

        raise_affiliation_unlocked(&root, "shared", &bound("ucsf")).unwrap();
        assert_eq!(owners_of(&root, "shared"), vec!["ucsf".to_string()]);
        assert_reachable(&root, "shared", true, &bound("ucsf")).unwrap();

        raise_affiliation_unlocked(&root, "shared", &bound("stanford")).unwrap();
        assert_eq!(
            owners_of(&root, "shared"),
            vec!["stanford".to_string(), "ucsf".to_string()]
        );

        assert!(
            assert_reachable(&root, "shared", true, &bound("ucsf")).is_err(),
            "a UCSF model reached a base that also holds Stanford's content"
        );
        assert!(assert_reachable(&root, "shared", true, &bound("stanford")).is_err());
        // …and a local model still reaches it, because no transfer occurs.
        assert_reachable(&root, "shared", true, &CallerAffiliation::Local).unwrap();
    }

    /// The ratchet is monotone on this axis too: nothing a later caller does
    /// removes an institution already recorded. A base that has held UCSF
    /// content holds it permanently.
    #[test]
    fn the_affiliation_ratchet_never_drops_an_institution() {
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_affiliation_unlocked(&root, "omop", &bound("ucsf")).unwrap();

        // A local model writes next: it contributes no institution, and it must
        // not clear the one already there.
        raise_affiliation_unlocked(&root, "omop", &CallerAffiliation::Local).unwrap();
        // …nor does a caller whose affiliation never reached this crate.
        raise_affiliation_unlocked(&root, "omop", &CallerAffiliation::Unstated).unwrap();
        assert_eq!(owners_of(&root, "omop"), vec!["ucsf".to_string()]);
        assert!(assert_reachable(&root, "omop", true, &bound("stanford")).is_err());
    }

    /// A base nobody has ratcheted — every base that existed before this axis
    /// did — carries no owners and is reachable from every private model.
    ///
    /// This is the **Missing** direction of Step 0's fail-closed rule, not the
    /// Unreadable one: the affiliations map being absent is a fact ("this build
    /// never recorded any"), and inventing constraints nobody wrote down is how
    /// a feature locks a user out of their own notes. It matches the tier
    /// migration, which made every pre-existing base public (AR-2).
    #[test]
    fn a_base_that_predates_this_axis_carries_no_owners() {
        let (_d, root) = tempdir_with_bases(&["default"]);
        // A schema-1 store, exactly as an older build wrote it.
        std::fs::write(
            crate::knowledge::paths::kb_tiers_path(&root),
            r#"{"schema":1,"bases":{"default":"private"}}"#,
        )
        .unwrap();

        assert!(is_private(&root, "default"), "the tier still reads");
        assert_eq!(owners_of(&root, "default"), Vec::<String>::new());
        for caller in [
            CallerAffiliation::Local,
            bound("ucsf"),
            CallerAffiliation::Unstated,
        ] {
            assert_reachable(&root, "default", true, &caller).unwrap();
        }
    }

    /// The **Unreadable** direction, which is the one Step 0's ⚠ is about: a
    /// store this build cannot parse means the owners are unknown, and unknown
    /// must not read as "no constraint". Only a local model — which transfers
    /// nothing — gets through.
    #[test]
    fn an_unreadable_store_makes_an_existing_bases_owners_unknown() {
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        std::fs::write(
            crate::knowledge::paths::kb_tiers_path(&root),
            "{not json at all",
        )
        .unwrap();

        assert_eq!(affiliation(&root, "omop"), KbAffiliation::Unknown);
        assert!(assert_reachable(&root, "omop", true, &bound("ucsf")).is_err());
        assert!(assert_reachable(&root, "omop", true, &CallerAffiliation::Unstated).is_err());
        assert_reachable(&root, "omop", true, &CallerAffiliation::Local).unwrap();

        // …but a base that does not exist on disk has nothing to leak, so an
        // unreadable store must not ban creating or importing one — decision
        // (3)'s third direction, which `is_private` already keeps.
        assert_eq!(
            affiliation(&root, "not-created-yet"),
            KbAffiliation::Owners(Default::default())
        );
        assert_reachable(&root, "not-created-yet", true, &bound("ucsf")).unwrap();
    }

    /// The refusal is DR-26's product: it must name the institution that owns
    /// the content and the institution whose model is bound, because a user can
    /// only act on a risk that was stated to them. It still names no base and no
    /// page — that half of §14.4 is unchanged.
    #[test]
    fn the_cross_institutional_refusal_names_both_sides_and_no_content() {
        let (_d, root) = tempdir_with_bases(&["omop-cohort-412"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_affiliation_unlocked(&root, "omop-cohort-412", &bound("ucsf")).unwrap();

        let s = assert_reachable(&root, "omop-cohort-412", true, &bound("stanford"))
            .unwrap_err()
            .to_string();
        assert!(s.contains("ucsf"), "the owning institution: {s}");
        assert!(s.contains("stanford"), "the bound institution: {s}");
        assert!(
            s.contains("does not transfer"),
            "why compliance does not carry over: {s}"
        );
        assert!(!s.contains("omop-cohort-412"), "named the base: {s}");

        // The unstated-model arm names no institution for the model side rather
        // than inventing one — "we cannot tell" is the actionable statement.
        let unstated =
            assert_reachable(&root, "omop-cohort-412", true, &CallerAffiliation::Unstated)
                .unwrap_err()
                .to_string();
        assert!(unstated.contains("ucsf"), "{unstated}");
        assert!(
            unstated.contains("does not state"),
            "the unstated arm must say so: {unstated}"
        );
    }

    /// Deleting a base drops BOTH axes, so an id reused later is classified by
    /// its own creator rather than by a base that no longer exists — the
    /// affiliation half of `deleting_a_base_forgets_its_tier_so_the_id_can_be_reused`.
    #[test]
    fn deleting_a_base_forgets_its_affiliation_too() {
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_affiliation_unlocked(&root, "omop", &bound("ucsf")).unwrap();
        forget_unlocked(&root, "omop").unwrap();
        assert_eq!(owners_of(&root, "omop"), Vec::<String>::new());
        assert_reachable(&root, "omop", true, &bound("stanford")).unwrap();
    }

    /// A store this build writes carries schema 2, and an older build still
    /// reads its `bases` map — the downgrade-safety argument on [`Store`],
    /// which is why the new axis is a sibling map rather than a richer value
    /// type. A downgrade that could not parse the file at all would read every
    /// base PRIVATE and lock the user out of all of them at once.
    #[test]
    fn a_schema_two_store_is_still_readable_by_a_reader_that_knows_only_bases() {
        let (_d, root) = tempdir_with_bases(&["omop"]);
        ensure_migrated_unlocked(&root).unwrap();
        raise_unlocked(&root, "omop", true).unwrap();
        raise_affiliation_unlocked(&root, "omop", &bound("ucsf")).unwrap();

        let raw = std::fs::read_to_string(crate::knowledge::paths::kb_tiers_path(&root)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["schema"], 2);
        assert_eq!(parsed["bases"]["omop"], "private");
        assert_eq!(parsed["affiliations"]["omop"][0], "ucsf");
    }
}
