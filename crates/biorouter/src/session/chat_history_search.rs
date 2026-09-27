use crate::conversation::message::MessageContent;
use crate::privacy::ProviderTier;
use crate::session::chat_fts;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ChatRecallResult {
    pub session_id: String,
    pub session_description: String,
    pub session_working_dir: String,
    pub last_activity: DateTime<Utc>,
    pub total_messages_in_session: usize,
    pub messages: Vec<ChatRecallMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRecallMessage {
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ChatRecallResults {
    pub results: Vec<ChatRecallResult>,
    pub total_matches: usize,
    /// Rows SQL returned, before rendering dropped any it could not deserialize.
    ///
    /// ⚠ This, not `total_matches`, is what says whether `LIMIT` was reached. A
    /// row whose `content_json` will not parse is skipped when the results are
    /// built, so a search that DID hit its cap can end up with
    /// `total_matches < limit` and silently lose its truncation warning.
    pub rows_examined: usize,
    /// Matching SQL rows that could not be rendered because their stored content
    /// was malformed or contained no supported recall text. These are matches,
    /// not evidence that the query found nothing.
    pub unrenderable_matches: usize,
}

type SqlQueryRow = (
    String,
    String,
    String,
    DateTime<Utc>,
    String,
    String,
    DateTime<Utc>,
);

type SessionMessageGroup = (
    String,
    String,
    DateTime<Utc>,
    Vec<(String, String, DateTime<Utc>)>,
);

pub struct ChatHistorySearch<'a> {
    pool: &'a Pool<Sqlite>,
    query: &'a str,
    limit: usize,
    after_date: Option<DateTime<Utc>>,
    before_date: Option<DateTime<Utc>>,
    exclude_session_id: Option<String>,
    /// Issue #56 Gate D. The reach this search runs with. `Public` filters
    /// private sessions out **in SQL**; `Private` is full reach.
    ///
    /// Deliberately a bare [`ProviderTier`] and not a `CallCapability`: this
    /// type sits in the session layer and is constructed from
    /// `crates/biorouter/tests/`, where `CallCapability`'s test constructor is
    /// not reachable. The one caller that holds a capability collapses it to a
    /// reach — see `chatrecall_extension.rs`'s SEARCH arm, which is also where
    /// DR-15's master opt-out is applied.
    caller_capability: ProviderTier,
    /// Issue #56 DR-26 / Task 50 Step 3. Whose agreements cover the searching
    /// model — the third axis, filtered in SQL beside the tier.
    ///
    /// A private chat that reached the UCSF OMOP connector holds UCSF's data in
    /// its transcript, and a snippet of it is exactly what this search returns.
    /// Both endpoints being Private means every tier gate says yes, so without
    /// this clause the operator's case leaks through search rather than through
    /// a tool call.
    ///
    /// `None` is a public model (already handled by the tier clause) or a
    /// private one that states nothing; both reach only chats that touched no
    /// institution at all.
    caller_affiliation: Option<crate::privacy::affiliation::ModelAffiliation>,
}

/// The reach one chat-recall search runs with: DR-26's two axes together.
///
/// One value rather than two parameters because they are one caller's identity
/// — a signature that takes them apart invites a call site that passes one
/// model's tier and another's institution, and the wrong one compiles. (It also
/// keeps `new` inside clippy's argument cap, which is the same pressure pointing
/// the same way.)
#[derive(Clone, Copy)]
pub struct SearchReach {
    /// `Public` filters private sessions out **in SQL**; `Private` is full
    /// reach on this axis.
    pub tier: ProviderTier,
    /// Whose agreements cover the searching model. `None` is a public model, or
    /// a private one that states nothing.
    pub affiliation: Option<crate::privacy::affiliation::ModelAffiliation>,
}

impl SearchReach {
    /// A search that asks only the tier question.
    ///
    /// ⚠ **Measured, not assumed: nothing in production uses this today.**
    /// `grep -rn 'search_chat_history' crates/` finds exactly one production
    /// caller, chatrecall's SEARCH arm, and it passes a real affiliation off its
    /// admitted capability. This exists for the fixtures that assert the tier
    /// axis, and for any future caller genuinely outside a chat — a human
    /// searching their own history through the GUI is entitled to see it, and
    /// that is a different question from what a MODEL may read.
    ///
    /// `Local` and not `None`, so the clause is absent entirely rather than
    /// merely satisfied: a fixture must not be silently narrowed by an axis it
    /// is not about, and `None` — an unstated model — reaches only unaffiliated
    /// chats, which would do exactly that.
    pub const fn tier_only(tier: ProviderTier) -> Self {
        Self {
            tier,
            affiliation: Some(crate::privacy::affiliation::ModelAffiliation::Local),
        }
    }
}

impl<'a> ChatHistorySearch<'a> {
    pub fn new(
        pool: &'a Pool<Sqlite>,
        query: &'a str,
        limit: Option<usize>,
        after_date: Option<DateTime<Utc>>,
        before_date: Option<DateTime<Utc>>,
        exclude_session_id: Option<String>,
        reach: SearchReach,
    ) -> Self {
        Self {
            pool,
            query,
            limit: limit.unwrap_or(10),
            after_date,
            before_date,
            exclude_session_id,
            caller_capability: reach.tier,
            caller_affiliation: reach.affiliation,
        }
    }

    /// Issue #56 DR-26 / Task 50 Step 3. The affiliation clause and the value it
    /// binds, or `None` when this search needs no clause.
    ///
    /// ⚠ **This one DOES take a placeholder, unlike the tier clause beside it,
    /// and the ordinal discipline that comment describes is what makes it
    /// safe**: it is appended last, immediately before `LIMIT ?`, and both
    /// builders bind it in exactly that position — after `before_date`, before
    /// the limit. It cannot be a literal: an institution id is
    /// `name_to_key`-normalised, which lowercases and strips whitespace and
    /// removes nothing else, so a quote survives into the slug.
    ///
    /// The three arms are DR-26's rows, not a policy of their own:
    ///
    /// * `Local` — reaches everything; no clause at all, because no transfer
    ///   occurs.
    /// * `Institution(X)` — reaches a chat only if **every** institution it
    ///   touched is X. `NOT EXISTS (… WHERE value <> ?)` is the union rule
    ///   (`affiliation::owners_compatible`) expressed in SQL; `EXISTS (… = ?)`
    ///   would be the allowlist rule and would return a chat that also reached
    ///   somebody else.
    /// * unstated — reaches only chats that touched no institution.
    fn affiliation_clause(&self) -> Option<(String, Option<String>)> {
        use crate::privacy::affiliation::ModelAffiliation;
        if !self.filters_by_affiliation() {
            return None;
        }
        const TOUCHED: &str = "json_each(COALESCE(NULLIF(s.session_affiliations, ''), '[]'))";
        let unclaimed_only = || {
            Some((
                format!(" AND NOT EXISTS (SELECT 1 FROM {TOUCHED})"),
                None::<String>,
            ))
        };
        match self.caller_affiliation {
            Some(ModelAffiliation::Local) => None,
            // A model covered by exactly one institution.
            other => match other.and_then(|m| m.sole_institution()) {
                Some(id) => Some((
                    format!(" AND NOT EXISTS (SELECT 1 FROM {TOUCHED} WHERE value <> ?)"),
                    Some(id.as_str().to_string()),
                )),
                // Unstated, or covered by two or more. ⚠ The two land on one
                // clause because the union rule makes them the same answer, not
                // because a spanning model is treated as unknown: a chat is
                // reachable only if EVERY institution it touched covers the
                // reader, and a reader covered by two institutions is covered by
                // neither alone — so no claimed chat qualifies, and only the
                // unclaimed ones do. That is exactly `owners_compatible`'s
                // verdict for both, and this module's
                // `the_affiliation_clause_is_owners_compatible_in_sql` drives
                // them against it rather than trusting this comment.
                None => unclaimed_only(),
            },
        }
    }

    /// DR-15's master opt-out for the affiliation clause, plus the tier
    /// narrowing DR-26 puts on the whole axis: affiliation is asked only of a
    /// PRIVATE searcher. A public one is already confined to public chats by the
    /// clause above, and a public chat holds no institution's data whatever a
    /// stale row beside it says — stating a second, different reason for one
    /// crossing is what `gate_cross_affiliation` avoids.
    fn filters_by_affiliation(&self) -> bool {
        self.caller_capability == ProviderTier::Private && crate::privacy::privacy_tiers_enabled()
    }

    /// DR-15's master opt-out for Gate D.
    ///
    /// Read here rather than only at the chatrecall SEARCH arm because `execute`
    /// has other callers — `SessionManager::search_chat_history` serves the HTTP
    /// search route and the CLI, neither of which holds a `CallCapability`.
    /// It is not a second read on the chatrecall path: that arm collapses the
    /// TIER to `Private` when the capability says the feature is off, so this
    /// predicate is already false there whichever way it reads the flag.
    fn filters_private_sessions(&self) -> bool {
        self.caller_capability == ProviderTier::Public && crate::privacy::privacy_tiers_enabled()
    }

    pub async fn execute(self) -> Result<ChatRecallResults> {
        let empty = || ChatRecallResults {
            results: vec![],
            total_matches: 0,
            rows_examined: 0,
            unrenderable_matches: 0,
        };

        // Prefer the FTS5 index (relevance-ranked via bm25) when it exists;
        // fall back to the legacy substring `LIKE` scan for an un-migrated DB
        // so recall never errors, it only degrades (BR-17).
        let rows = if self.query.trim().is_empty() {
            self.fetch_rows_recent().await?
        } else if self.fts_available().await {
            let match_expr = chat_fts::sanitize_fts_query(self.query);
            if match_expr.is_empty() {
                return Ok(empty());
            }
            self.fetch_rows_fts(&match_expr).await?
        } else {
            let keywords = self.parse_keywords();
            if keywords.is_empty() {
                return Ok(empty());
            }
            self.fetch_rows_like(&keywords).await?
        };

        // ⚠ Count the rows BEFORE `process_rows` drops any it cannot parse:
        // "did SQL hit the LIMIT" is a question about the query, not about how
        // many of its rows happened to render.
        let rows_examined = rows.len();
        let (session_messages, order) = self.process_rows(rows);
        let session_totals = self.get_session_totals(&session_messages).await?;
        let results =
            self.convert_to_results(session_messages, order, session_totals, rows_examined);

        Ok(results)
    }

    /// One latest user-authored message from each recent session, newest first.
    ///
    /// Chat Recall's ordinary search cannot discover a chat whose vocabulary
    /// the caller does not already know. Meditation needs the opposite: list
    /// sessions after its successful-run cursor, then decide which are worth
    /// reading. An empty query selects this bounded recent-session mode.
    /// Privacy and affiliation clauses stay in SQL, before `LIMIT`, exactly as
    /// they do for both keyword-search builders.
    async fn fetch_rows_recent(&self) -> Result<Vec<SqlQueryRow>> {
        // Tool responses are persisted with role `user` too. Requiring a
        // top-level text part keeps recent mode on actual authored prompts.
        let authored = "COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER))";
        let mut sql = format!(
            r#"
            SELECT
                s.id as session_id,
                COALESCE(NULLIF(s.name, ''), NULLIF(s.description, ''), 'Untitled chat') as session_description,
                s.working_dir as session_working_dir,
                s.created_at as session_created_at,
                m.role,
                m.content_json,
                {authored} as timestamp
            FROM sessions s
            INNER JOIN messages m ON m.id = (
                SELECT recent.id
                FROM messages recent
                WHERE recent.session_id = s.id
                  AND recent.role = 'user'
                  AND EXISTS (
                      SELECT 1
                      FROM json_each(
                          CASE
                              WHEN json_valid(recent.content_json)
                              THEN recent.content_json
                              ELSE '[]'
                          END
                      ) part
                      WHERE json_extract(part.value, '$.type') = 'text'
                  )
                ORDER BY COALESCE(
                    NULLIF(recent.created_timestamp, 0),
                    CAST(strftime('%s', recent.timestamp) AS INTEGER)
                ) DESC, recent.id DESC
                LIMIT 1
            )
            WHERE s.session_type = 'user'
            "#
        );

        if self.exclude_session_id.is_some() {
            sql.push_str(" AND s.id != ?");
        }
        if self.after_date.is_some() {
            sql.push_str(&format!(" AND {authored} >= ?"));
        }
        if self.before_date.is_some() {
            sql.push_str(&format!(" AND {authored} <= ?"));
        }
        if self.filters_private_sessions() {
            sql.push_str(" AND s.privacy_tier = 'public'");
        }
        if let Some((clause, _)) = self.affiliation_clause() {
            sql.push_str(&clause);
        }
        sql.push_str(" AND s.id NOT IN (SELECT value FROM json_each(?))");
        sql.push_str(&format!(" ORDER BY {authored} DESC LIMIT ?"));

        let mut query_builder = sqlx::query_as::<_, SqlQueryRow>(&sql);
        if let Some(exclude_id) = &self.exclude_session_id {
            query_builder = query_builder.bind(exclude_id);
        }
        if let Some(after) = self.after_date {
            query_builder = query_builder.bind(after.timestamp());
        }
        if let Some(before) = self.before_date {
            query_builder = query_builder.bind(before.timestamp());
        }
        if let Some((_, Some(institution))) = self.affiliation_clause() {
            query_builder = query_builder.bind(institution);
        }
        let crew_ids = crate::crew::manager()?.scoped_session_ids().await;
        query_builder = query_builder.bind(serde_json::to_string(&crew_ids)?);
        query_builder = query_builder.bind(self.limit as i64);

        Ok(query_builder.fetch_all(self.pool).await?)
    }

    /// True when the FTS5 mirror table exists (migration arm 15, and re-created
    /// unconditionally by `reconcile_loop_schema` on every open).
    async fn fts_available(&self) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'",
        )
        .fetch_one(self.pool)
        .await
        .map(|c| c > 0)
        .unwrap_or(false)
    }

    /// Relevance-ranked recall over the FTS5 index. Rows come back best-first
    /// (`bm25` ascending) and, because the index only stores the flattened
    /// text, this joins back to `messages`/`sessions` so the downstream row
    /// shape and rendering stay identical to the `LIKE` path.
    async fn fetch_rows_fts(&self, match_expr: &str) -> Result<Vec<SqlQueryRow>> {
        let mut sql = String::from(
            r#"
            SELECT
                s.id as session_id,
                -- ⚠ `sessions.description` is a DEAD legacy column: nothing in
                -- production has written it since the title moved to
                -- `sessions.name`, so reading it alone rendered every recall hit
                -- as "Session:  (ID: …)" — an anonymous row the model cannot use
                -- to decide which chat to open. This COALESCE is the same
                -- fallback `session_manager.rs`'s summary query already uses, and
                -- keeping `description` in it is what preserves rows written
                -- before the rename. NO placeholder here on purpose: both
                -- builders bind strictly positionally, so a `?` would shift every
                -- later ordinal and mis-bind silently.
                COALESCE(NULLIF(s.name, ''), NULLIF(s.description, ''), 'Untitled chat') as session_description,
                s.working_dir as session_working_dir,
                s.created_at as session_created_at,
                m.role,
                m.content_json,
                -- ⚠ The AUTHORED time, not `m.timestamp`.
                --
                -- `messages.timestamp` is `DEFAULT CURRENT_TIMESTAMP` and is
                -- never bound on insert, and `replace_conversation_inner`
                -- DELETEs a session's rows and re-INSERTs them — so every
                -- compaction re-stamps the whole history to the moment of the
                -- rewrite. Measured on a real store: 1,023 messages whose
                -- `created_timestamp` and `timestamp` differ by more than a day,
                -- and 103 of 740 substantial sessions where every message
                -- carries one identical `timestamp`. `created_timestamp` is the
                -- epoch the message was actually written and it survives the
                -- rewrite, which is why the session-summary and activity queries
                -- already use it.
                --
                -- The COALESCE covers the handful of rows that predate it (86 in
                -- that store) by falling back to the write time. sqlx decodes an
                -- INTEGER column into `DateTime<Utc>` via `timestamp_opt`, so the
                -- row type is unchanged.
                COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER)) as timestamp
            FROM messages_fts f
            INNER JOIN messages m ON m.id = f.message_id
            INNER JOIN sessions s ON m.session_id = s.id
            WHERE messages_fts MATCH ?
        "#,
        );

        if self.exclude_session_id.is_some() {
            sql.push_str(" AND s.id != ?");
        }
        if self.after_date.is_some() {
            sql.push_str(" AND COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER)) >= ?");
        }
        if self.before_date.is_some() {
            sql.push_str(" AND COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER)) <= ?");
        }

        // Issue #56 Gate D. `sessions s` is already joined above, so this is one
        // clause. A SQL LITERAL, never a `?`: both builders bind strictly
        // positionally with the optional clauses in a fixed order, so an
        // inserted placeholder shifts every later ordinal and mis-binds
        // SILENTLY — no error, wrong results. The literal is a compile-time
        // constant of the code path, not user input.
        //
        // It sits BEFORE the `LIMIT ?` on purpose: SQLite applies the limit to
        // the filtered set, so a public caller gets its full quota of public
        // rows even when private rows would have outranked them. A Rust-side
        // post-filter after `execute()` returns is the same one-liner and is
        // wrong for exactly that reason.
        if self.filters_private_sessions() {
            sql.push_str(" AND s.privacy_tier = 'public'");
        }
        // Issue #56 DR-26 / Task 50 Step 3 — see `affiliation_clause`. LAST, so
        // its placeholder is the final one before `LIMIT ?`.
        if let Some((clause, _)) = self.affiliation_clause() {
            sql.push_str(&clause);
        }

        sql.push_str(" AND s.id NOT IN (SELECT value FROM json_each(?))");
        sql.push_str(" ORDER BY bm25(messages_fts) ASC LIMIT ?");

        let mut query_builder = sqlx::query_as::<_, SqlQueryRow>(&sql).bind(match_expr);

        if let Some(exclude_id) = &self.exclude_session_id {
            query_builder = query_builder.bind(exclude_id);
        }
        // ⚠ Epoch seconds, matching the INTEGER expression the clause compares
        // against. This used to bind a `DateTime` against the TEXT
        // `messages.timestamp`, which sqlx encodes as RFC3339 — and since 'T'
        // sorts after the space that column stores, the bound silently dropped
        // its own boundary day instead of erroring. Comparing integers removes
        // the whole class.
        if let Some(after) = self.after_date {
            query_builder = query_builder.bind(after.timestamp());
        }
        if let Some(before) = self.before_date {
            query_builder = query_builder.bind(before.timestamp());
        }
        // Issue #56 DR-26 / Task 50 Step 3. Bound HERE — after `before_date`,
        // before the limit — because the clause is appended in exactly that
        // position. Moving either without the other mis-binds silently.
        if let Some((_, Some(institution))) = self.affiliation_clause() {
            query_builder = query_builder.bind(institution);
        }
        let crew_ids = crate::crew::manager()?.scoped_session_ids().await;
        query_builder = query_builder.bind(serde_json::to_string(&crew_ids)?);
        query_builder = query_builder.bind(self.limit as i64);

        Ok(query_builder.fetch_all(self.pool).await?)
    }

    async fn fetch_rows_like(&self, keywords: &[String]) -> Result<Vec<SqlQueryRow>> {
        let sql = self.build_sql(keywords);
        let mut query_builder = sqlx::query_as::<_, SqlQueryRow>(&sql);

        for keyword in keywords {
            query_builder = query_builder.bind(keyword);
        }

        if let Some(exclude_id) = &self.exclude_session_id {
            query_builder = query_builder.bind(exclude_id);
        }

        // ⚠ Epoch seconds, matching the INTEGER expression the clause compares
        // against. This used to bind a `DateTime` against the TEXT
        // `messages.timestamp`, which sqlx encodes as RFC3339 — and since 'T'
        // sorts after the space that column stores, the bound silently dropped
        // its own boundary day instead of erroring. Comparing integers removes
        // the whole class.
        if let Some(after) = self.after_date {
            query_builder = query_builder.bind(after.timestamp());
        }
        if let Some(before) = self.before_date {
            query_builder = query_builder.bind(before.timestamp());
        }
        // See the twin in `fetch_rows_fts`.
        if let Some((_, Some(institution))) = self.affiliation_clause() {
            query_builder = query_builder.bind(institution);
        }

        let crew_ids = crate::crew::manager()?.scoped_session_ids().await;
        query_builder = query_builder.bind(serde_json::to_string(&crew_ids)?);
        query_builder = query_builder.bind(self.limit as i64);

        Ok(query_builder.fetch_all(self.pool).await?)
    }

    fn parse_keywords(&self) -> Vec<String> {
        self.query
            .split_whitespace()
            .map(|word| format!("%{}%", word.to_lowercase()))
            .collect()
    }

    fn build_sql(&self, keywords: &[String]) -> String {
        let mut sql = String::from(
            r#"
            SELECT
                s.id as session_id,
                -- The twin of the COALESCE in `fetch_rows_fts`; see the note
                -- there. The two builders must render the same title or a recall
                -- would be named on an FTS-backed store and anonymous on the
                -- `LIKE` fallback.
                COALESCE(NULLIF(s.name, ''), NULLIF(s.description, ''), 'Untitled chat') as session_description,
                s.working_dir as session_working_dir,
                s.created_at as session_created_at,
                m.role,
                m.content_json,
                -- ⚠ The AUTHORED time, not `m.timestamp`.
                --
                -- `messages.timestamp` is `DEFAULT CURRENT_TIMESTAMP` and is
                -- never bound on insert, and `replace_conversation_inner`
                -- DELETEs a session's rows and re-INSERTs them — so every
                -- compaction re-stamps the whole history to the moment of the
                -- rewrite. Measured on a real store: 1,023 messages whose
                -- `created_timestamp` and `timestamp` differ by more than a day,
                -- and 103 of 740 substantial sessions where every message
                -- carries one identical `timestamp`. `created_timestamp` is the
                -- epoch the message was actually written and it survives the
                -- rewrite, which is why the session-summary and activity queries
                -- already use it.
                --
                -- The COALESCE covers the handful of rows that predate it (86 in
                -- that store) by falling back to the write time. sqlx decodes an
                -- INTEGER column into `DateTime<Utc>` via `timestamp_opt`, so the
                -- row type is unchanged.
                COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER)) as timestamp
            FROM messages m
            INNER JOIN sessions s ON m.session_id = s.id
            WHERE EXISTS (
                SELECT 1 FROM json_each(m.content_json) 
                WHERE json_extract(value, '$.type') = 'text' 
                AND (
        "#,
        );

        for (i, _) in keywords.iter().enumerate() {
            if i > 0 {
                sql.push_str(" OR ");
            }
            sql.push_str("LOWER(json_extract(value, '$.text')) LIKE ?");
        }

        sql.push_str(
            r#"
                )
            )
        "#,
        );

        if self.exclude_session_id.is_some() {
            sql.push_str(" AND s.id != ?");
        }

        if self.after_date.is_some() {
            sql.push_str(" AND COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER)) >= ?");
        }
        if self.before_date.is_some() {
            sql.push_str(" AND COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER)) <= ?");
        }

        // Issue #56 Gate D — the same clause on the `LIKE` fallback. `execute`
        // branches on a `sqlite_master` probe for `messages_fts`, so filtering
        // only the FTS builder leaks on every un-migrated profile.
        if self.filters_private_sessions() {
            sql.push_str(" AND s.privacy_tier = 'public'");
        }
        // The same clause on the `LIKE` fallback, for the reason the tier clause
        // is duplicated here: `execute` branches on a `sqlite_master` probe, so
        // filtering only the FTS builder leaks on every un-migrated profile.
        if let Some((clause, _)) = self.affiliation_clause() {
            sql.push_str(&clause);
        }

        sql.push_str(" AND s.id NOT IN (SELECT value FROM json_each(?))");
        sql.push_str(" ORDER BY COALESCE(NULLIF(m.created_timestamp, 0), CAST(strftime('%s', m.timestamp) AS INTEGER)) DESC LIMIT ?");

        sql
    }

    /// Group matched rows by session, returning the sessions in the order they
    /// were first seen. Rows arrive ranked (bm25-best-first for FTS,
    /// most-recent-first for the `LIKE` fallback), so first-seen order carries
    /// the ranking down to the session level.
    fn process_rows(
        &self,
        rows: Vec<SqlQueryRow>,
    ) -> (HashMap<String, SessionMessageGroup>, Vec<String>) {
        let mut session_messages: HashMap<String, SessionMessageGroup> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

        for (
            session_id,
            session_description,
            session_working_dir,
            session_created_at,
            role,
            content_json,
            timestamp,
        ) in rows
        {
            if let Ok(content_vec) = serde_json::from_str::<Vec<MessageContent>>(&content_json) {
                let text_parts = chat_fts::searchable_parts(&content_vec);

                if !text_parts.is_empty() {
                    if !session_messages.contains_key(&session_id) {
                        order.push(session_id.clone());
                    }
                    let entry = session_messages.entry(session_id.clone()).or_insert((
                        session_description.clone(),
                        session_working_dir.clone(),
                        session_created_at,
                        Vec::new(),
                    ));
                    entry
                        .3
                        .push((role.clone(), text_parts.join("\n"), timestamp));
                }
            }
        }

        (session_messages, order)
    }

    async fn get_session_totals(
        &self,
        session_messages: &HashMap<String, SessionMessageGroup>,
    ) -> Result<HashMap<String, usize>> {
        let mut session_totals: HashMap<String, usize> = HashMap::new();
        if session_messages.is_empty() {
            return Ok(session_totals);
        }

        // Single grouped query instead of one COUNT(*) round-trip per session
        // (the previous N+1 issued a separate query for every matched session).
        let ids: Vec<&String> = session_messages.keys().collect();
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT session_id, COUNT(*) FROM messages WHERE session_id IN ({placeholders}) GROUP BY session_id"
        );
        let mut query = sqlx::query_as::<_, (String, i64)>(&sql);
        for id in &ids {
            query = query.bind(*id);
        }
        let rows = query.fetch_all(self.pool).await.unwrap_or_default();
        for (session_id, count) in rows {
            session_totals.insert(session_id, count as usize);
        }
        Ok(session_totals)
    }

    fn convert_to_results(
        &self,
        mut session_messages: HashMap<String, SessionMessageGroup>,
        order: Vec<String>,
        session_totals: HashMap<String, usize>,
        rows_examined: usize,
    ) -> ChatRecallResults {
        // Emit sessions in ranked order (best first) rather than re-sorting by
        // recency, so bm25 relevance is what the caller sees.
        let mut results: Vec<ChatRecallResult> = Vec::with_capacity(order.len());
        for session_id in order {
            let Some((description, working_dir, _created_at, messages)) =
                session_messages.remove(&session_id)
            else {
                continue;
            };

            let message_vec: Vec<ChatRecallMessage> = messages
                .into_iter()
                .map(|(role, content, timestamp)| ChatRecallMessage {
                    role,
                    content,
                    timestamp,
                })
                .collect();

            let last_activity = message_vec
                .iter()
                .map(|m| m.timestamp)
                .max()
                .unwrap_or_else(chrono::Utc::now);

            let total_messages_in_session = session_totals.get(&session_id).copied().unwrap_or(0);

            results.push(ChatRecallResult {
                session_id,
                session_description: description,
                session_working_dir: working_dir,
                last_activity,
                total_messages_in_session,
                messages: message_vec,
            });
        }

        let total_matches = results.iter().map(|r| r.messages.len()).sum();
        ChatRecallResults {
            results,
            total_matches,
            rows_examined,
            unrenderable_matches: rows_examined.saturating_sub(total_matches),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Issue #56 Gate D (SEARCH).
    //!
    //! This file had no `mod tests` before this task, so the pre-count of the
    //! `session::chat_history_search` filter is zero — "0 passed, exits 0" is
    //! the baseline, and the only meaningful number is the count of tests added
    //! here.

    use super::*;
    use crate::privacy::ProviderTier;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    /// `execute` picks its builder by probing `sqlite_master` for
    /// `messages_fts`, so the only honest way to exercise the `LIKE` fallback
    /// is to seed a database that genuinely lacks the table.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum QueryPath {
        Fts,
        LikeFallback,
    }

    #[derive(Clone)]
    struct Chat {
        /// Seeded into `sessions.name` — the column production writes. The
        /// legacy `sessions.description` is exercised only by
        /// `a_pre_rename_row_still_renders_its_legacy_description`.
        name: String,
        working_dir: String,
        private: bool,
        text: String,
    }

    fn chat(private: bool, text: &str) -> Chat {
        Chat {
            name: if private {
                "private chat".to_string()
            } else {
                "public chat".to_string()
            },
            working_dir: "/tmp/w".to_string(),
            private,
            text: text.to_string(),
        }
    }

    fn private_chat_containing(term: &str) -> Chat {
        chat(true, &format!("we discussed the {term} at length"))
    }

    fn public_chat_containing(term: &str) -> Chat {
        chat(false, &format!("we discussed the {term} at length"))
    }

    /// A short document with one hit: best bm25 (`ORDER BY bm25 ASC`). These are
    /// seeded first, and `seeded` gives earlier rows the newer timestamp, so the
    /// `LIKE` fallback's `ORDER BY m.timestamp DESC` reproduces the same ranking.
    fn private_chat_ranking_high(term: &str) -> Chat {
        chat(true, term)
    }

    /// The same term buried in a long document: worst bm25, and seeded last so
    /// it also has the oldest timestamp.
    fn public_chat_ranking_low(term: &str) -> Chat {
        let filler = "filler ".repeat(200);
        chat(false, &format!("{filler}{term} {filler}"))
    }

    fn private_chat_titled(description: &str, working_dir: &str, term: &str) -> Chat {
        Chat {
            name: description.to_string(),
            working_dir: working_dir.to_string(),
            private: true,
            text: format!("notes about the {term}"),
        }
    }

    fn vec_of(n: usize, c: Chat) -> Vec<Chat> {
        std::iter::repeat_n(c, n).collect()
    }

    /// The plan spelled this `vec_of(..).chain(vec_of(..))`; `Vec` is not an
    /// `Iterator`, so the concatenation is a free function.
    fn chain(a: Vec<Chat>, b: Vec<Chat>) -> Vec<Chat> {
        a.into_iter().chain(b).collect()
    }

    /// Does the clause this builder chose admit a chat whose stored
    /// `session_affiliations` column holds `stored`? **Answered by SQLite**,
    /// against a real row.
    ///
    /// ⚠ **The clause is a string this module hands to a database, so a Rust
    /// reader of it can only check the intent.** `json_each`, `COALESCE`,
    /// `NULLIF` and `<>` are SQLite's semantics, not ours, and the interesting
    /// failures live there: a legacy row holding `''` rather than `'[]'` makes
    /// `json_each` RAISE, which no decode of the clause's three shapes could
    /// ever notice and which would break recall for every reader rather than
    /// returning a wrong set.
    async fn clause_admits_stored(
        pool: &Pool<Sqlite>,
        clause: &Option<(String, Option<String>)>,
        stored: &str,
    ) -> bool {
        sqlx::query("DELETE FROM sessions")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sessions (id, session_affiliations) VALUES ('s', ?)")
            .bind(stored)
            .execute(pool)
            .await
            .unwrap();

        // `1 = 1` so the clause appends exactly as it does in both real
        // builders, which is where its leading ` AND ` comes from.
        let (sql, bound) = match clause {
            None => ("SELECT COUNT(*) FROM sessions s".to_string(), None),
            Some((clause, bound)) => (
                format!("SELECT COUNT(*) FROM sessions s WHERE 1 = 1{clause}"),
                bound.clone(),
            ),
        };
        let mut query = sqlx::query_scalar::<_, i64>(&sql);
        if let Some(value) = bound.as_deref() {
            query = query.bind(value);
        }
        query
            .fetch_one(pool)
            .await
            .expect("the affiliation clause must be valid SQL over a real sessions row")
            > 0
    }

    /// [`clause_admits_stored`] for the ordinary case: a chat that touched these
    /// institutions, encoded the way the store encodes them.
    async fn clause_admits(
        pool: &Pool<Sqlite>,
        clause: &Option<(String, Option<String>)>,
        touched: &[&str],
    ) -> bool {
        let stored = serde_json::to_string(touched).unwrap();
        clause_admits_stored(pool, clause, &stored).await
    }

    /// Task 56: **the SQL clause is `affiliation::owners_compatible` in SQLite**,
    /// held to it rather than described as agreeing with it — and *run* there
    /// rather than read back in Rust.
    ///
    /// The clause has three shapes and the model used to have three shapes, so
    /// the mapping was one-to-one and a comment was nearly enough. It is not any
    /// more: a model covered by TWO institutions takes the *same* clause as an
    /// unstated one. That is a genuine coincidence of the union rule — a reader
    /// covered by two institutions is covered by neither alone, so no claimed
    /// chat qualifies for it, which is exactly the unstated row — and a
    /// coincidence is the kind of thing a later edit breaks in silence. So it is
    /// asserted against the one comparison, over every model shape this
    /// vocabulary can produce.
    #[tokio::test]
    async fn the_affiliation_clause_is_owners_compatible_in_sql() {
        use crate::privacy::affiliation::{owners_compatible, InstitutionId, ModelAffiliation};
        use std::collections::BTreeSet;

        assert!(
            crate::privacy::privacy_tiers_enabled(),
            "the clause is only built with the feature on, so this test would be vacuous"
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new())
            .await
            .unwrap();
        // Only the column the clause reads. The point is what SQLite does with
        // the clause, not what the real schema looks like around it.
        sqlx::query(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, \
             session_affiliations TEXT NOT NULL DEFAULT '')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let inst = InstitutionId::new;
        let models = [
            None,
            Some(ModelAffiliation::Local),
            Some(ModelAffiliation::institution(inst("ucsf"))),
            Some(ModelAffiliation::institution(inst("stanford"))),
            Some(
                ModelAffiliation::institutions([inst("ucsf"), inst("stanford")])
                    .expect("a two-element set is not empty"),
            ),
        ];
        let touched_sets: [&[&str]; 5] = [
            &[],
            &["ucsf"],
            &["stanford"],
            &["ucsf", "stanford"],
            &["ucsf", "stanford", "broad"],
        ];

        for model in models {
            let search = ChatHistorySearch::new(
                &pool,
                "anything",
                None,
                None,
                None,
                None,
                SearchReach {
                    tier: ProviderTier::Private,
                    affiliation: model,
                },
            );
            let clause = search.affiliation_clause();
            for touched in touched_sets {
                let owners: BTreeSet<InstitutionId> = touched.iter().map(|o| inst(o)).collect();
                assert_eq!(
                    clause_admits(&pool, &clause, touched).await,
                    owners_compatible(model, &owners),
                    "the SQL clause disagrees with the union rule for {model:?} over {touched:?}"
                );
            }

            // The row `COALESCE(NULLIF(…, ''), '[]')` exists for: a session
            // persisted before this column held JSON stores `''`, and
            // `json_each('')` RAISES. Without the guard the clause is not
            // merely wrong for that row, it errors — and every recall by a
            // private reader fails. Only running the SQL can see this.
            assert_eq!(
                clause_admits_stored(&pool, &clause, "").await,
                owners_compatible(model, &BTreeSet::new()),
                "a legacy row with no JSON must read as a chat that touched no institution, \
                 for {model:?}"
            );
        }
    }

    struct Db {
        _temp: tempfile::TempDir,
        pool: Pool<Sqlite>,
    }

    async fn seeded(path: QueryPath, chats: &[Chat]) -> Db {
        let _temp = tempfile::TempDir::new().unwrap();
        let opts = SqliteConnectOptions::new()
            .filename(_temp.path().join("recall.db"))
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();

        // ⚠ This fixture must mirror PRODUCTION's `sessions` shape, which has
        // BOTH `name` (where the title actually lives) and the legacy, never
        // written `description`. It used to declare `description` only, so every
        // test here seeded a title into a column production leaves empty — which
        // is exactly why the empty-"Session:" bug passed a green suite for as
        // long as it existed. Seed titles into `name`; `description` is only
        // written by `a_pre_rename_row_still_renders_its_legacy_description`.
        sqlx::query(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', \
             description TEXT NOT NULL DEFAULT '', \
             working_dir TEXT NOT NULL DEFAULT '', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, \
             privacy_tier TEXT NOT NULL DEFAULT 'public', \
             session_type TEXT NOT NULL DEFAULT 'user')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, \
             role TEXT NOT NULL, content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL, \
             timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP)",
        )
        .execute(&pool)
        .await
        .unwrap();
        if path == QueryPath::Fts {
            sqlx::query(
                "CREATE VIRTUAL TABLE messages_fts USING fts5(text, session_id UNINDEXED, \
                 message_id UNINDEXED, tokenize = 'porter unicode61')",
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        let base = chrono::Utc::now();
        for (i, c) in chats.iter().enumerate() {
            let sid = format!("s{i}");
            let ts = base - chrono::Duration::seconds(i as i64);
            sqlx::query(
                "INSERT INTO sessions (id, name, working_dir, privacy_tier) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&sid)
            .bind(&c.name)
            .bind(&c.working_dir)
            .bind(if c.private { "private" } else { "public" })
            .execute(&pool)
            .await
            .unwrap();

            let content_json =
                serde_json::to_string(&vec![MessageContent::text(c.text.clone())]).unwrap();
            let inserted = sqlx::query(
                "INSERT INTO messages (session_id, role, content_json, created_timestamp, timestamp) \
                 VALUES (?, 'user', ?, ?, ?)",
            )
            .bind(&sid)
            .bind(&content_json)
            // ⚠ `created_timestamp` must AGREE with `timestamp`, and it must be
            // the real epoch of `ts`. It used to be the loop index (0, 1, 2 …),
            // i.e. 1970 — harmless while nothing read the column, and silently
            // ranking-inverting the moment recall started ordering by it, since
            // the index ascends while `ts` descends.
            .bind(ts.timestamp())
            // ⚠ `.naive_utc()`, because sqlx encodes a `DateTime` as RFC3339 and
            // production stores `%F %T`. A fixture that stored the RFC3339 form
            // would make any date-filter test here pass against a format the
            // real column never holds — which is how the date bug survived.
            .bind(ts.naive_utc())
            .execute(&pool)
            .await
            .unwrap();

            if path == QueryPath::Fts {
                sqlx::query(
                    "INSERT INTO messages_fts (text, session_id, message_id) VALUES (?, ?, ?)",
                )
                .bind(&c.text)
                .bind(&sid)
                .bind(inserted.last_insert_rowid())
                .execute(&pool)
                .await
                .unwrap();
            }
        }

        Db { _temp, pool }
    }

    /// Issue #56 DR-26: these tests assert the TIER filter, so the third axis
    /// must not narrow them. A local model reaches everything — the affiliation
    /// clause is absent entirely rather than merely satisfied, which is what
    /// keeps this fixture testing what it says it does.
    async fn search_with_limit(
        tier: ProviderTier,
        db: &Db,
        query: &str,
        limit: usize,
    ) -> ChatRecallResults {
        ChatHistorySearch::new(
            &db.pool,
            query,
            Some(limit),
            None,
            None,
            None,
            SearchReach::tier_only(tier),
        )
        .execute()
        .await
        .unwrap()
    }

    async fn search_as(tier: ProviderTier, db: &Db, query: &str) -> ChatRecallResults {
        ChatHistorySearch::new(
            &db.pool,
            query,
            None,
            None,
            None,
            None,
            SearchReach::tier_only(tier),
        )
        .execute()
        .await
        .unwrap()
    }

    /// Everything that reaches the model. Stricter than chatrecall's prose
    /// formatting: it covers every field of every result, not just the ones the
    /// current renderer happens to print.
    fn render_for_model(r: &ChatRecallResults) -> String {
        serde_json::to_string(r).unwrap()
    }

    #[tokio::test]
    async fn both_query_paths_filter_private_rows() {
        for path in [QueryPath::Fts, QueryPath::LikeFallback] {
            let db = seeded(
                path,
                &[
                    private_chat_containing("cohort"),
                    public_chat_containing("cohort"),
                ],
            )
            .await;
            assert_eq!(
                search_as(ProviderTier::Public, &db, "cohort")
                    .await
                    .results
                    .len(),
                1,
                "{path:?}"
            );
            assert_eq!(
                search_as(ProviderTier::Private, &db, "cohort")
                    .await
                    .results
                    .len(),
                2,
                "{path:?}"
            );
        }
    }

    #[tokio::test]
    async fn empty_query_lists_recent_sessions_without_guessing_their_vocabulary() {
        let db = seeded(
            QueryPath::Fts,
            &[
                private_chat_containing("secret-topic"),
                public_chat_containing("hash-prefixed-todo"),
                public_chat_containing("unrelated-clinic-dashboard"),
            ],
        )
        .await;

        let later = sqlx::query_scalar::<_, i64>("SELECT MAX(created_timestamp) FROM messages")
            .fetch_one(&db.pool)
            .await
            .unwrap()
            + 60;
        let assistant_content = serde_json::to_string(&vec![MessageContent::text(
            "assistant-summary-without-user-preference",
        )])
        .unwrap();
        sqlx::query(
            "INSERT INTO messages (session_id, role, content_json, created_timestamp, timestamp) \
             VALUES ('s1', 'assistant', ?, ?, datetime(?, 'unixepoch'))",
        )
        .bind(assistant_content)
        .bind(later)
        .bind(later)
        .execute(&db.pool)
        .await
        .unwrap();

        let tool_response_content = r#"[{"type":"toolResponse","id":"remember","toolResult":{"status":"success","value":{"content":[{"type":"text","text":"stored"}]}}}]"#;
        sqlx::query(
            "INSERT INTO messages (session_id, role, content_json, created_timestamp, timestamp) \
             VALUES ('s1', 'user', ?, ?, datetime(?, 'unixepoch'))",
        )
        .bind(tool_response_content)
        .bind(later + 60)
        .bind(later + 60)
        .execute(&db.pool)
        .await
        .unwrap();

        let public = search_with_limit(ProviderTier::Public, &db, "", 10).await;
        assert_eq!(public.results.len(), 2);
        let rendered = render_for_model(&public);
        assert!(rendered.contains("hash-prefixed-todo"), "{rendered}");
        assert!(
            rendered.contains("unrelated-clinic-dashboard"),
            "{rendered}"
        );
        assert!(!rendered.contains("secret-topic"), "{rendered}");
        assert!(!rendered.contains("assistant-summary-without-user-preference"));
        assert!(!rendered.contains("Tool Response"));
        assert!(!rendered.contains("stored"));
        assert!(public
            .results
            .iter()
            .flat_map(|result| &result.messages)
            .all(|message| message.role == "user"));

        let private = search_with_limit(ProviderTier::Private, &db, "", 10).await;
        assert_eq!(private.results.len(), 3);
        assert!(render_for_model(&private).contains("secret-topic"));

        sqlx::query("UPDATE sessions SET session_type = 'scheduled' WHERE id = 's0'")
            .execute(&db.pool)
            .await
            .unwrap();
        let user_only = search_with_limit(ProviderTier::Private, &db, "", 10).await;
        assert_eq!(user_only.results.len(), 2);
        assert!(!render_for_model(&user_only).contains("secret-topic"));

        let boundary = sqlx::query_scalar::<_, i64>(
            "SELECT created_timestamp FROM messages WHERE session_id = 's1'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let after = DateTime::<Utc>::from_timestamp(boundary, 0).unwrap();
        let since_boundary = ChatHistorySearch::new(
            &db.pool,
            "",
            Some(10),
            Some(after),
            None,
            None,
            SearchReach::tier_only(ProviderTier::Public),
        )
        .execute()
        .await
        .unwrap();
        assert_eq!(since_boundary.results.len(), 1);
        assert!(since_boundary.results[0].messages[0]
            .content
            .contains("hash-prefixed-todo"));
    }

    #[tokio::test]
    async fn recent_session_limit_is_applied_after_the_privacy_filter() {
        let db = seeded(
            QueryPath::Fts,
            &chain(
                vec_of(10, private_chat_ranking_high("private")),
                vec_of(3, public_chat_ranking_low("public")),
            ),
        )
        .await;

        let recent = search_with_limit(ProviderTier::Public, &db, "", 5).await;
        assert_eq!(recent.results.len(), 3);
        assert!(recent
            .results
            .iter()
            .all(|result| result.messages[0].content.contains("public")));
    }

    #[tokio::test]
    async fn the_limit_is_applied_after_the_filter_not_before() {
        // THE test. 10 private rows ranking above 3 public ones with limit=5: a
        // public caller must get all 3 public rows, not 0. A Rust-side post-filter
        // — the obvious implementation, and the one that needs no SQL change —
        // returns 0 here, silently and non-deterministically, with no error.
        // SQLite applies the `LIMIT ?` at the end of each builder.
        for path in [QueryPath::Fts, QueryPath::LikeFallback] {
            let db = seeded(
                path,
                &chain(
                    vec_of(10, private_chat_ranking_high("cohort")),
                    vec_of(3, public_chat_ranking_low("cohort")),
                ),
            )
            .await;
            let r = search_with_limit(ProviderTier::Public, &db, "cohort", 5).await;
            assert_eq!(
                r.results.len(),
                3,
                "post-filtered in Rust instead of in SQL ({path:?})"
            );
        }
    }

    /// Recall must read the AUTHORED time, not the row's write time.
    ///
    /// ⚠ `messages.timestamp` is re-stamped for a whole session on every
    /// compaction, so ordering and date-filtering on it silently treats an old
    /// chat as if it happened at the moment it was last rewritten. This
    /// simulates that: the write time is moved to the far future while
    /// `created_timestamp` stays put.
    #[test_case::test_case(QueryPath::Fts; "fts")]
    #[test_case::test_case(QueryPath::LikeFallback; "like_fallback")]
    #[tokio::test]
    async fn a_rewritten_row_is_dated_by_when_it_was_written_not_by_the_rewrite(path: QueryPath) {
        let db = seeded(path, &[public_chat_containing("ribosome")]).await;
        let authored = sqlx::query_scalar::<_, i64>("SELECT created_timestamp FROM messages")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET timestamp = '2099-01-01 00:00:00'")
            .execute(&db.pool)
            .await
            .unwrap();

        let r = search_as(ProviderTier::Private, &db, "ribosome").await;
        assert_eq!(
            r.results[0].messages[0].timestamp.timestamp(),
            authored,
            "the rewrite's timestamp was reported as the message's time ({path:?})"
        );
        assert!(
            r.results[0].last_activity.timestamp() == authored,
            "last_activity followed the rewrite rather than the authored time ({path:?})"
        );
    }

    /// The `NULLIF(s.description, '')` arm of the title expression.
    ///
    /// ⚠ Without this, that arm and the `'Untitled chat'` literal are dead
    /// weight in both builders: delete either and the whole suite stays green,
    /// and a session written before the title moved to `sessions.name` renders
    /// as `Session:  (ID: …)` again — the exact defect the COALESCE fixes.
    #[test_case::test_case(QueryPath::Fts; "fts")]
    #[test_case::test_case(QueryPath::LikeFallback; "like_fallback")]
    #[tokio::test]
    async fn a_pre_rename_row_still_renders_its_legacy_description(path: QueryPath) {
        let db = seeded(path, &[public_chat_containing("ribosome")]).await;
        // Move this row back to the pre-rename shape: no name, a description.
        sqlx::query("UPDATE sessions SET name = '', description = 'legacy title' WHERE id = 's0'")
            .execute(&db.pool)
            .await
            .unwrap();

        let r = search_as(ProviderTier::Private, &db, "ribosome").await;
        assert_eq!(
            r.results[0].session_description, "legacy title",
            "a row written before the rename must still be named ({path:?})"
        );
    }

    /// The `'Untitled chat'` arm: both columns empty must not render a blank.
    #[test_case::test_case(QueryPath::Fts; "fts")]
    #[test_case::test_case(QueryPath::LikeFallback; "like_fallback")]
    #[tokio::test]
    async fn a_row_with_no_title_at_all_renders_a_placeholder(path: QueryPath) {
        let db = seeded(path, &[public_chat_containing("ribosome")]).await;
        sqlx::query("UPDATE sessions SET name = '', description = '' WHERE id = 's0'")
            .execute(&db.pool)
            .await
            .unwrap();

        let r = search_as(ProviderTier::Private, &db, "ribosome").await;
        assert_eq!(
            r.results[0].session_description, "Untitled chat",
            "an untitled row must not render as an empty name ({path:?})"
        );
    }

    #[tokio::test]
    async fn no_content_field_of_a_private_row_survives() {
        // §11.4: session_description is the LLM-generated title, produced FROM the
        // conversation, and is the field most likely to be mislabelled as metadata.
        let db = seeded(
            QueryPath::Fts,
            &[private_chat_titled(
                "PHI cohort characterisation",
                "/data/phi/x",
                "cohort",
            )],
        )
        .await;
        let r = search_as(ProviderTier::Public, &db, "cohort").await;
        let rendered = render_for_model(&r);
        for leak in [
            "PHI cohort characterisation",
            "/data/phi",
            "cohort characterisation",
        ] {
            assert!(!rendered.contains(leak), "{leak} survived: {rendered}");
        }
    }
}
