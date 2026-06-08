//! SQLite-backed ledger implementation.

use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

use thymos_core::{
    canonical_json_bytes,
    commit::Commit,
    content_hash,
    ids::IntentId,
    proposal::{Proposal, RejectionReason},
    CommitId, ContentHash, Error, Result, TrajectoryId,
};

use crate::{build_entry, AuditEntry, Entry, EntryKind, EntryPayload};

pub struct SqliteLedger {
    conn: Mutex<Connection>,
}

impl SqliteLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).map_err(|e| Error::Ledger(e.to_string()))?;
        Self::bootstrap(&conn)?;
        Ok(SqliteLedger {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| Error::Ledger(e.to_string()))?;
        Self::bootstrap(&conn)?;
        Ok(SqliteLedger {
            conn: Mutex::new(conn),
        })
    }

    fn bootstrap(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous  = NORMAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS entries (
                id             BLOB PRIMARY KEY,
                trajectory_id  BLOB NOT NULL,
                parent_id      BLOB,
                seq            INTEGER NOT NULL,
                kind           TEXT NOT NULL,
                payload_bytes  BLOB NOT NULL,
                created_at     INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_entries_trajectory_seq
                ON entries(trajectory_id, seq);

            -- Hard invariant: at most one entry per (trajectory, seq). This is
            -- the structural guard against a forked chain when two writers race
            -- to append at the same sequence number — the losing INSERT fails
            -- rather than silently creating a second entry at that seq.
            CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_trajectory_seq_unique
                ON entries(trajectory_id, seq);

            CREATE TABLE IF NOT EXISTS heads (
                trajectory_id  BLOB NOT NULL,
                branch         TEXT NOT NULL,
                head_id        BLOB NOT NULL,
                head_seq       INTEGER NOT NULL,
                PRIMARY KEY (trajectory_id, branch)
            );
            "#,
        )
        .map_err(|e| Error::Ledger(e.to_string()))?;
        Ok(())
    }

    pub fn append_root(&self, trajectory_id: TrajectoryId, note: &str) -> Result<Entry> {
        let payload = EntryPayload::Root {
            trajectory_id,
            note: note.to_string(),
        };
        let entry = build_entry(trajectory_id, None, 0, EntryKind::Root, payload)?;
        self.insert_entry(&entry, true)?;
        Ok(entry)
    }

    pub fn append_commit(&self, commit: Commit) -> Result<Entry> {
        let mut conn = self.conn.lock().unwrap();
        // IMMEDIATE acquires the write lock up front, so the head read and the
        // entry insert are one atomic step against other connections/processes
        // — no other writer can advance the head between our read and write.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| Error::Ledger(e.to_string()))?;

        let (parent_id, parent_seq) = Self::current_head(&tx, commit.body.trajectory_id)?;

        let expected_parent_in_commit = match &commit.body.parent[..] {
            [single] => Some(single.0),
            [] => None,
            _ => {
                return Err(Error::Invariant(
                    "multi-parent commits not supported in Phase 1".into(),
                ));
            }
        };
        if expected_parent_in_commit != Some(parent_id) && expected_parent_in_commit.is_some() {
            return Err(Error::Invariant(format!(
                "commit parent does not match trajectory head: commit says {:?}, head is {:?}",
                expected_parent_in_commit, parent_id
            )));
        }
        if commit.body.seq != parent_seq + 1 {
            return Err(Error::Invariant(format!(
                "commit seq {} does not follow head seq {}",
                commit.body.seq, parent_seq
            )));
        }

        let trajectory_id = commit.body.trajectory_id;
        let seq = commit.body.seq;
        let payload = EntryPayload::Commit(commit);
        let entry = Entry {
            id: content_hash(&payload)?,
            trajectory_id,
            parent: Some(parent_id),
            seq,
            kind: EntryKind::Commit,
            payload,
        };
        Self::insert_entry_inner(&tx, &entry, true)?;
        tx.commit().map_err(|e| Error::Ledger(e.to_string()))?;
        Ok(entry)
    }

    pub fn append_rejection(
        &self,
        trajectory_id: TrajectoryId,
        intent_id: IntentId,
        reason: RejectionReason,
    ) -> Result<Entry> {
        let conn = self.conn.lock().unwrap();
        let (parent_id, parent_seq) = Self::current_head(&conn, trajectory_id)?;
        let payload = EntryPayload::Rejection { intent_id, reason };
        let entry = build_entry(
            trajectory_id,
            Some(parent_id),
            parent_seq + 1,
            EntryKind::Rejection,
            payload,
        )?;
        Self::insert_entry_inner(&conn, &entry, true)?;
        Ok(entry)
    }

    pub fn append_pending_approval(
        &self,
        trajectory_id: TrajectoryId,
        proposal: Proposal,
        channel: String,
        reason: String,
    ) -> Result<Entry> {
        let conn = self.conn.lock().unwrap();
        let (parent_id, parent_seq) = Self::current_head(&conn, trajectory_id)?;
        let payload = EntryPayload::PendingApproval {
            proposal,
            channel,
            reason,
        };
        let entry = build_entry(
            trajectory_id,
            Some(parent_id),
            parent_seq + 1,
            EntryKind::PendingApproval,
            payload,
        )?;
        Self::insert_entry_inner(&conn, &entry, true)?;
        Ok(entry)
    }

    pub fn append_delegation(
        &self,
        trajectory_id: TrajectoryId,
        child_trajectory_id: TrajectoryId,
        task: &str,
        final_answer: Option<String>,
    ) -> Result<Entry> {
        let conn = self.conn.lock().unwrap();
        let (parent_id, parent_seq) = Self::current_head(&conn, trajectory_id)?;
        let payload = EntryPayload::Delegation {
            child_trajectory_id,
            task: task.to_string(),
            final_answer,
        };
        let entry = build_entry(
            trajectory_id,
            Some(parent_id),
            parent_seq + 1,
            EntryKind::Delegation,
            payload,
        )?;
        Self::insert_entry_inner(&conn, &entry, true)?;
        Ok(entry)
    }

    pub fn append_skill_bound(
        &self,
        trajectory_id: TrajectoryId,
        skill: thymos_core::skill::SkillDef,
        params: Vec<(String, String)>,
    ) -> Result<Entry> {
        let conn = self.conn.lock().unwrap();
        let (parent_id, parent_seq) = Self::current_head(&conn, trajectory_id)?;
        let payload = EntryPayload::SkillBound {
            skill_id: skill.id(),
            skill,
            params,
        };
        let entry = build_entry(
            trajectory_id,
            Some(parent_id),
            parent_seq + 1,
            EntryKind::SkillBound,
            payload,
        )?;
        Self::insert_entry_inner(&conn, &entry, true)?;
        Ok(entry)
    }

    pub fn append_branch_root(
        &self,
        new_trajectory_id: TrajectoryId,
        source_trajectory_id: TrajectoryId,
        source_commit_id: CommitId,
        note: &str,
    ) -> Result<Entry> {
        let payload = EntryPayload::Branch {
            source_trajectory_id,
            source_commit_id,
            note: note.to_string(),
        };
        let entry = build_entry(new_trajectory_id, None, 0, EntryKind::Branch, payload)?;
        self.insert_entry(&entry, true)?;
        Ok(entry)
    }

    pub fn has_trajectory(&self, trajectory_id: TrajectoryId) -> bool {
        self.head(trajectory_id).is_ok()
    }

    pub fn head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)> {
        let conn = self.conn.lock().unwrap();
        Self::current_head(&conn, trajectory_id)
    }

    pub fn entries(&self, trajectory_id: TrajectoryId) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, trajectory_id, parent_id, seq, kind, payload_bytes
                 FROM entries
                 WHERE trajectory_id = ?1
                 ORDER BY seq ASC",
            )
            .map_err(|e| Error::Ledger(e.to_string()))?;
        let rows = stmt
            .query_map(params![trajectory_id.0.as_bytes().as_slice()], row_to_entry)
            .map_err(|e| Error::Ledger(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Ledger(e.to_string()))?);
        }
        Ok(out)
    }

    pub fn verify_integrity(&self, trajectory_id: TrajectoryId) -> Result<()> {
        let entries = self.entries(trajectory_id)?;
        crate::verify_integrity_entries(&entries)
    }

    /// Produce a publishable [`MerkleAnchor`] over the trajectory's current
    /// history. Integrity is verified first, so an anchor is only ever taken
    /// over a valid chain. Publish the returned anchor externally; later call
    /// [`crate::verify_anchor`] with the trajectory's entries to prove the
    /// ledger was not rewritten.
    pub fn anchor(&self, trajectory_id: TrajectoryId) -> Result<crate::MerkleAnchor> {
        let entries = self.entries(trajectory_id)?;
        crate::verify_integrity_entries(&entries)?;
        Ok(crate::compute_anchor(trajectory_id, &entries))
    }

    /// Query entries across all trajectories with optional filters.
    ///
    /// - `trajectory_id`: restrict to a single trajectory
    /// - `kind`: restrict to a specific entry kind (e.g. "commit", "rejection")
    /// - `from_ts` / `to_ts`: unix-second time range on `created_at`
    /// - `limit`: max rows returned (default 1000)
    pub fn query_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, trajectory_id, parent_id, seq, kind, payload_bytes, created_at
             FROM entries WHERE 1=1",
        );
        if trajectory_id.is_some() {
            sql.push_str(" AND trajectory_id = :traj");
        }
        if kind.is_some() {
            sql.push_str(" AND kind = :kind");
        }
        if from_ts.is_some() {
            sql.push_str(" AND created_at >= :from_ts");
        }
        if to_ts.is_some() {
            sql.push_str(" AND created_at <= :to_ts");
        }
        sql.push_str(" ORDER BY created_at ASC, seq ASC");
        let row_limit = limit.unwrap_or(1000);
        sql.push_str(&format!(" LIMIT {row_limit}"));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| Error::Ledger(e.to_string()))?;

        let mut named: Vec<(&str, &dyn rusqlite::types::ToSql)> = Vec::new();
        let traj_bytes_owned = trajectory_id
            .map(|tid| tid.0.as_bytes().to_vec())
            .unwrap_or_default();
        if trajectory_id.is_some() {
            named.push((":traj", &traj_bytes_owned as &dyn rusqlite::types::ToSql));
        }
        let kind_owned = kind.unwrap_or("").to_string();
        if kind.is_some() {
            named.push((":kind", &kind_owned as &dyn rusqlite::types::ToSql));
        }
        let from_i64 = from_ts.unwrap_or(0) as i64;
        if from_ts.is_some() {
            named.push((":from_ts", &from_i64 as &dyn rusqlite::types::ToSql));
        }
        let to_i64 = to_ts.unwrap_or(0) as i64;
        if to_ts.is_some() {
            named.push((":to_ts", &to_i64 as &dyn rusqlite::types::ToSql));
        }

        let rows = stmt
            .query_map(named.as_slice(), |row| {
                let id_bytes: Vec<u8> = row.get(0)?;
                let traj_bytes: Vec<u8> = row.get(1)?;
                let _parent_bytes: Option<Vec<u8>> = row.get(2)?;
                let seq: i64 = row.get(3)?;
                let kind_str: String = row.get(4)?;
                let payload_bytes: Vec<u8> = row.get(5)?;
                let created_at: i64 = row.get(6)?;
                Ok((
                    id_bytes,
                    traj_bytes,
                    seq,
                    kind_str,
                    payload_bytes,
                    created_at,
                ))
            })
            .map_err(|e| Error::Ledger(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            let (id_bytes, traj_bytes, seq, kind_str, payload_bytes, created_at) =
                row.map_err(|e| Error::Ledger(e.to_string()))?;
            let payload: EntryPayload =
                serde_json::from_slice(&payload_bytes).map_err(|e| Error::Ledger(e.to_string()))?;
            out.push(AuditEntry {
                id: hex::encode(&id_bytes),
                trajectory_id: hex::encode(&traj_bytes),
                seq: seq as u64,
                kind: kind_str,
                payload,
                created_at: created_at as u64,
            });
        }
        Ok(out)
    }

    /// Count entries matching the given filters.
    pub fn count_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from("SELECT COUNT(*) FROM entries WHERE 1=1");
        if trajectory_id.is_some() {
            sql.push_str(" AND trajectory_id = :traj");
        }
        if kind.is_some() {
            sql.push_str(" AND kind = :kind");
        }
        if from_ts.is_some() {
            sql.push_str(" AND created_at >= :from_ts");
        }
        if to_ts.is_some() {
            sql.push_str(" AND created_at <= :to_ts");
        }

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| Error::Ledger(e.to_string()))?;

        let mut named: Vec<(&str, &dyn rusqlite::types::ToSql)> = Vec::new();
        let traj_bytes_owned = trajectory_id
            .map(|tid| tid.0.as_bytes().to_vec())
            .unwrap_or_default();
        if trajectory_id.is_some() {
            named.push((":traj", &traj_bytes_owned as &dyn rusqlite::types::ToSql));
        }
        let kind_owned = kind.unwrap_or("").to_string();
        if kind.is_some() {
            named.push((":kind", &kind_owned as &dyn rusqlite::types::ToSql));
        }
        let from_i64 = from_ts.unwrap_or(0) as i64;
        if from_ts.is_some() {
            named.push((":from_ts", &from_i64 as &dyn rusqlite::types::ToSql));
        }
        let to_i64 = to_ts.unwrap_or(0) as i64;
        if to_ts.is_some() {
            named.push((":to_ts", &to_i64 as &dyn rusqlite::types::ToSql));
        }

        let count: i64 = stmt
            .query_row(named.as_slice(), |row| row.get(0))
            .map_err(|e| Error::Ledger(e.to_string()))?;
        Ok(count as u64)
    }

    // ---- internals ----

    fn current_head(conn: &Connection, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)> {
        let mut stmt = conn
            .prepare(
                "SELECT head_id, head_seq FROM heads WHERE trajectory_id = ?1 AND branch = 'main'",
            )
            .map_err(|e| Error::Ledger(e.to_string()))?;
        let mut rows = stmt
            .query(params![trajectory_id.0.as_bytes().as_slice()])
            .map_err(|e| Error::Ledger(e.to_string()))?;
        if let Some(row) = rows.next().map_err(|e| Error::Ledger(e.to_string()))? {
            let bytes: Vec<u8> = row.get(0).map_err(|e| Error::Ledger(e.to_string()))?;
            let seq: i64 = row.get(1).map_err(|e| Error::Ledger(e.to_string()))?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok((ContentHash(arr), seq as u64))
        } else {
            Err(Error::Ledger("trajectory has no head (not rooted)".into()))
        }
    }

    fn insert_entry(&self, entry: &Entry, advance_head: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        Self::insert_entry_inner(&conn, entry, advance_head)
    }

    fn insert_entry_inner(conn: &Connection, entry: &Entry, advance_head: bool) -> Result<()> {
        let payload_bytes = canonical_json_bytes(&entry.payload)?;

        let recomputed = blake3::hash(&payload_bytes);
        if recomputed.as_bytes() != entry.id.as_bytes() {
            return Err(Error::Invariant(
                "entry id does not match payload hash".into(),
            ));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;

        let kind_str = crate::kind_to_str(entry.kind);

        conn.execute(
            "INSERT INTO entries(id, trajectory_id, parent_id, seq, kind, payload_bytes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id.as_bytes().as_slice(),
                entry.trajectory_id.0.as_bytes().as_slice(),
                entry.parent.map(|p| p.0.to_vec()),
                entry.seq as i64,
                kind_str,
                payload_bytes,
                now,
            ],
        )
        .map_err(|e| Error::Ledger(e.to_string()))?;

        if advance_head {
            conn.execute(
                "INSERT INTO heads(trajectory_id, branch, head_id, head_seq)
                 VALUES (?1, 'main', ?2, ?3)
                 ON CONFLICT(trajectory_id, branch)
                 DO UPDATE SET head_id = excluded.head_id, head_seq = excluded.head_seq",
                params![
                    entry.trajectory_id.0.as_bytes().as_slice(),
                    entry.id.as_bytes().as_slice(),
                    entry.seq as i64,
                ],
            )
            .map_err(|e| Error::Ledger(e.to_string()))?;
        }
        Ok(())
    }
}

/// `LedgerStore` for the SQLite backend. Each method delegates to the
/// inherent method of the same name, so the trait is a pure re-exposure of the
/// existing surface with no behavior change. `has_trajectory`,
/// `verify_integrity`, and `anchor` use the trait defaults — identical to the
/// inherent versions, which are themselves defined over `entries`/`head`.
impl crate::LedgerStore for SqliteLedger {
    fn backend(&self) -> &'static str {
        "sqlite"
    }
    fn append_root(&self, trajectory_id: TrajectoryId, note: &str) -> Result<Entry> {
        SqliteLedger::append_root(self, trajectory_id, note)
    }

    fn append_commit(&self, commit: Commit) -> Result<Entry> {
        SqliteLedger::append_commit(self, commit)
    }

    fn append_rejection(
        &self,
        trajectory_id: TrajectoryId,
        intent_id: IntentId,
        reason: RejectionReason,
    ) -> Result<Entry> {
        SqliteLedger::append_rejection(self, trajectory_id, intent_id, reason)
    }

    fn append_pending_approval(
        &self,
        trajectory_id: TrajectoryId,
        proposal: Proposal,
        channel: String,
        reason: String,
    ) -> Result<Entry> {
        SqliteLedger::append_pending_approval(self, trajectory_id, proposal, channel, reason)
    }

    fn append_delegation(
        &self,
        trajectory_id: TrajectoryId,
        child_trajectory_id: TrajectoryId,
        task: &str,
        final_answer: Option<String>,
    ) -> Result<Entry> {
        SqliteLedger::append_delegation(self, trajectory_id, child_trajectory_id, task, final_answer)
    }

    fn append_skill_bound(
        &self,
        trajectory_id: TrajectoryId,
        skill: thymos_core::skill::SkillDef,
        params: Vec<(String, String)>,
    ) -> Result<Entry> {
        SqliteLedger::append_skill_bound(self, trajectory_id, skill, params)
    }

    fn append_branch_root(
        &self,
        new_trajectory_id: TrajectoryId,
        source_trajectory_id: TrajectoryId,
        source_commit_id: CommitId,
        note: &str,
    ) -> Result<Entry> {
        SqliteLedger::append_branch_root(
            self,
            new_trajectory_id,
            source_trajectory_id,
            source_commit_id,
            note,
        )
    }

    fn head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)> {
        SqliteLedger::head(self, trajectory_id)
    }

    fn entries(&self, trajectory_id: TrajectoryId) -> Result<Vec<Entry>> {
        SqliteLedger::entries(self, trajectory_id)
    }

    fn query_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>> {
        SqliteLedger::query_entries(self, trajectory_id, kind, from_ts, to_ts, limit)
    }

    fn count_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Result<u64> {
        SqliteLedger::count_entries(self, trajectory_id, kind, from_ts, to_ts)
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let id_bytes: Vec<u8> = row.get(0)?;
    let traj_bytes: Vec<u8> = row.get(1)?;
    let parent_bytes: Option<Vec<u8>> = row.get(2)?;
    let seq: i64 = row.get(3)?;
    let kind_str: String = row.get(4)?;
    let payload_bytes: Vec<u8> = row.get(5)?;

    let mut id_arr = [0u8; 32];
    id_arr.copy_from_slice(&id_bytes);
    let mut traj_arr = [0u8; 32];
    traj_arr.copy_from_slice(&traj_bytes);

    let parent = parent_bytes.map(|b| {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&b);
        ContentHash(arr)
    });

    let kind = crate::str_to_kind(&kind_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;

    let payload: EntryPayload = serde_json::from_slice(&payload_bytes).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(e))
    })?;

    Ok(Entry {
        id: ContentHash(id_arr),
        trajectory_id: TrajectoryId(ContentHash(traj_arr)),
        parent,
        seq: seq as u64,
        kind,
        payload,
    })
}
