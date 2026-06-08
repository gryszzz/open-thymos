//! Postgres-backed ledger implementation.
//!
//! Uses `deadpool-postgres` for connection pooling. The schema mirrors the
//! SQLite version but uses BYTEA for binary columns and BIGINT for sequences.
//!
//! All operations are async. Callers (the runtime, server) must run them
//! inside a tokio context.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::NoTls;

use thymos_core::{
    canonical_json_bytes,
    commit::Commit,
    content_hash,
    ids::IntentId,
    proposal::{Proposal, RejectionReason},
    CommitId, ContentHash, Error, Result, TrajectoryId,
};

use crate::{build_entry, AuditEntry, Entry, EntryKind, EntryPayload};

pub struct PostgresLedger {
    pool: Pool,
}

impl PostgresLedger {
    /// Connect to Postgres using a connection string like
    /// `host=localhost user=thymos dbname=thymos`.
    pub async fn connect(conn_str: &str) -> Result<Self> {
        let mut cfg = Config::new();
        cfg.url = Some(conn_str.into());
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| Error::Ledger(format!("pg pool: {e}")))?;

        let ledger = PostgresLedger { pool };
        ledger.bootstrap().await?;
        Ok(ledger)
    }

    async fn bootstrap(&self) -> Result<()> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;
        client
            .batch_execute(
                r#"
                BEGIN;
                -- `CREATE TABLE IF NOT EXISTS` is NOT atomic against the system
                -- catalogs: two connections bootstrapping a fresh database at the
                -- same time (multiple server replicas on first boot, or parallel
                -- tests sharing one database) can race and fail with a catalog
                -- unique violation. Serialize bootstrap behind a transaction-scoped
                -- advisory lock so exactly one connection runs the DDL; the others
                -- wait, then find the tables already exist. The key is an arbitrary
                -- fixed constant unique to this schema.
                SELECT pg_advisory_xact_lock(7479796814201601);

                CREATE TABLE IF NOT EXISTS entries (
                    id             BYTEA PRIMARY KEY,
                    trajectory_id  BYTEA NOT NULL,
                    parent_id      BYTEA,
                    seq            BIGINT NOT NULL,
                    kind           TEXT NOT NULL,
                    payload_bytes  BYTEA NOT NULL,
                    created_at     BIGINT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_entries_trajectory_seq
                    ON entries(trajectory_id, seq);

                -- Hard invariant against forked chains under multi-node races:
                -- at most one entry per (trajectory, seq). The losing concurrent
                -- INSERT fails on the unique violation instead of creating a
                -- second entry at the same sequence number.
                CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_trajectory_seq_unique
                    ON entries(trajectory_id, seq);

                CREATE TABLE IF NOT EXISTS heads (
                    trajectory_id  BYTEA NOT NULL,
                    branch         TEXT NOT NULL,
                    head_id        BYTEA NOT NULL,
                    head_seq       BIGINT NOT NULL,
                    PRIMARY KEY (trajectory_id, branch)
                );
                COMMIT;
                "#,
            )
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;
        Ok(())
    }

    pub async fn append_root(&self, trajectory_id: TrajectoryId, note: &str) -> Result<Entry> {
        let payload = EntryPayload::Root {
            trajectory_id,
            note: note.to_string(),
        };
        let entry = build_entry(trajectory_id, None, 0, EntryKind::Root, payload)?;
        self.insert_entry(&entry, true).await?;
        Ok(entry)
    }

    pub async fn append_commit(&self, commit: Commit) -> Result<Entry> {
        let (parent_id, parent_seq) = self.current_head(commit.body.trajectory_id).await?;

        let expected_parent = match &commit.body.parent[..] {
            [single] => Some(single.0),
            [] => None,
            _ => {
                return Err(Error::Invariant(
                    "multi-parent commits not supported".into(),
                ))
            }
        };
        if expected_parent != Some(parent_id) && expected_parent.is_some() {
            return Err(Error::Invariant(format!(
                "commit parent mismatch: {:?} vs head {:?}",
                expected_parent, parent_id
            )));
        }
        if commit.body.seq != parent_seq + 1 {
            return Err(Error::Invariant(format!(
                "commit seq {} does not follow head {}",
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
        self.insert_entry(&entry, true).await?;
        Ok(entry)
    }

    pub async fn append_rejection(
        &self,
        trajectory_id: TrajectoryId,
        intent_id: IntentId,
        reason: RejectionReason,
    ) -> Result<Entry> {
        let (parent_id, parent_seq) = self.current_head(trajectory_id).await?;
        let payload = EntryPayload::Rejection { intent_id, reason };
        let entry = build_entry(
            trajectory_id,
            Some(parent_id),
            parent_seq + 1,
            EntryKind::Rejection,
            payload,
        )?;
        self.insert_entry(&entry, true).await?;
        Ok(entry)
    }

    pub async fn append_pending_approval(
        &self,
        trajectory_id: TrajectoryId,
        proposal: Proposal,
        channel: String,
        reason: String,
    ) -> Result<Entry> {
        let (parent_id, parent_seq) = self.current_head(trajectory_id).await?;
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
        self.insert_entry(&entry, true).await?;
        Ok(entry)
    }

    pub async fn append_delegation(
        &self,
        trajectory_id: TrajectoryId,
        child_trajectory_id: TrajectoryId,
        task: &str,
        final_answer: Option<String>,
    ) -> Result<Entry> {
        let (parent_id, parent_seq) = self.current_head(trajectory_id).await?;
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
        self.insert_entry(&entry, true).await?;
        Ok(entry)
    }

    pub async fn append_skill_bound(
        &self,
        trajectory_id: TrajectoryId,
        skill: thymos_core::skill::SkillDef,
        params: Vec<(String, String)>,
    ) -> Result<Entry> {
        let (parent_id, parent_seq) = self.current_head(trajectory_id).await?;
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
        self.insert_entry(&entry, true).await?;
        Ok(entry)
    }

    pub async fn append_branch_root(
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
        self.insert_entry(&entry, true).await?;
        Ok(entry)
    }

    pub async fn has_trajectory(&self, trajectory_id: TrajectoryId) -> bool {
        self.current_head(trajectory_id).await.is_ok()
    }

    pub async fn head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)> {
        self.current_head(trajectory_id).await
    }

    pub async fn entries(&self, trajectory_id: TrajectoryId) -> Result<Vec<Entry>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;
        let rows = client
            .query(
                "SELECT id, trajectory_id, parent_id, seq, kind, payload_bytes
                 FROM entries
                 WHERE trajectory_id = $1
                 ORDER BY seq ASC",
                &[&trajectory_id.0.as_bytes().as_slice()],
            )
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;

        rows.iter().map(row_to_entry).collect()
    }

    pub async fn verify_integrity(&self, trajectory_id: TrajectoryId) -> Result<()> {
        let entries = self.entries(trajectory_id).await?;
        crate::verify_integrity_entries(&entries)
    }

    /// Query entries across all trajectories with optional filters.
    ///
    /// - `trajectory_id`: restrict to a single trajectory
    /// - `kind`: restrict to a specific entry kind (e.g. "commit", "rejection")
    /// - `from_ts` / `to_ts`: unix-second time range on `created_at`
    /// - `limit`: max rows returned (default 1000)
    pub async fn query_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;

        let mut sql = String::from(
            "SELECT id, trajectory_id, parent_id, seq, kind, payload_bytes, created_at
             FROM entries WHERE 1=1",
        );
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::new();
        let mut n: usize = 0;

        let traj_bytes: Option<Vec<u8>> = trajectory_id.map(|t| t.0.as_bytes().to_vec());
        if let Some(ref bytes) = traj_bytes {
            n += 1;
            sql.push_str(&format!(" AND trajectory_id = ${n}"));
            params.push(Box::new(bytes.clone()));
        }
        if let Some(k) = kind {
            n += 1;
            sql.push_str(&format!(" AND kind = ${n}"));
            params.push(Box::new(k.to_string()));
        }
        if let Some(from) = from_ts {
            n += 1;
            sql.push_str(&format!(" AND created_at >= ${n}"));
            params.push(Box::new(from as i64));
        }
        if let Some(to) = to_ts {
            n += 1;
            sql.push_str(&format!(" AND created_at <= ${n}"));
            params.push(Box::new(to as i64));
        }
        sql.push_str(" ORDER BY created_at ASC, seq ASC");
        let row_limit = limit.unwrap_or(1000);
        sql.push_str(&format!(" LIMIT {row_limit}"));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = client
            .query(sql.as_str(), &param_refs[..])
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let id_bytes: &[u8] = row.get(0);
            let traj_bytes: &[u8] = row.get(1);
            let seq: i64 = row.get(3);
            let kind_str: &str = row.get(4);
            let payload_bytes: &[u8] = row.get(5);
            let created_at: i64 = row.get(6);
            let payload: EntryPayload = serde_json::from_slice(payload_bytes)
                .map_err(|e| Error::Ledger(format!("deserialize payload: {e}")))?;
            out.push(AuditEntry {
                id: hex::encode(id_bytes),
                trajectory_id: hex::encode(traj_bytes),
                seq: seq as u64,
                kind: kind_str.to_string(),
                payload,
                created_at: created_at as u64,
            });
        }
        Ok(out)
    }

    /// Count entries matching the given filters.
    pub async fn count_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Result<u64> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;

        let mut sql = String::from("SELECT COUNT(*) FROM entries WHERE 1=1");
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::new();
        let mut n: usize = 0;

        let traj_bytes: Option<Vec<u8>> = trajectory_id.map(|t| t.0.as_bytes().to_vec());
        if let Some(ref bytes) = traj_bytes {
            n += 1;
            sql.push_str(&format!(" AND trajectory_id = ${n}"));
            params.push(Box::new(bytes.clone()));
        }
        if let Some(k) = kind {
            n += 1;
            sql.push_str(&format!(" AND kind = ${n}"));
            params.push(Box::new(k.to_string()));
        }
        if let Some(from) = from_ts {
            n += 1;
            sql.push_str(&format!(" AND created_at >= ${n}"));
            params.push(Box::new(from as i64));
        }
        if let Some(to) = to_ts {
            n += 1;
            sql.push_str(&format!(" AND created_at <= ${n}"));
            params.push(Box::new(to as i64));
        }

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref()).collect();

        let row = client
            .query_one(sql.as_str(), &param_refs[..])
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;
        let count: i64 = row.get(0);
        Ok(count as u64)
    }

    // ---- internals ----

    async fn current_head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;
        let rows = client
            .query(
                "SELECT head_id, head_seq FROM heads WHERE trajectory_id = $1 AND branch = 'main'",
                &[&trajectory_id.0.as_bytes().as_slice()],
            )
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;

        if let Some(row) = rows.first() {
            let bytes: &[u8] = row.get(0);
            let seq: i64 = row.get(1);
            let mut arr = [0u8; 32];
            arr.copy_from_slice(bytes);
            Ok((ContentHash(arr), seq as u64))
        } else {
            Err(Error::Ledger("trajectory has no head (not rooted)".into()))
        }
    }

    async fn insert_entry(&self, entry: &Entry, advance_head: bool) -> Result<()> {
        let payload_bytes = canonical_json_bytes(&entry.payload)?;

        let recomputed = blake3::hash(&payload_bytes);
        if recomputed.as_bytes() != entry.id.as_bytes() {
            return Err(Error::Invariant(
                "entry id does not match payload hash".into(),
            ));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let kind_str = crate::kind_to_str(entry.kind);
        let parent_bytes: Option<Vec<u8>> = entry.parent.map(|p| p.0.to_vec());

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;

        client
            .execute(
                "INSERT INTO entries(id, trajectory_id, parent_id, seq, kind, payload_bytes, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &entry.id.as_bytes().as_slice(),
                    &entry.trajectory_id.0.as_bytes().as_slice(),
                    &parent_bytes.as_deref(),
                    &(entry.seq as i64),
                    &kind_str,
                    &payload_bytes.as_slice(),
                    &now,
                ],
            )
            .await
            .map_err(|e| Error::Ledger(e.to_string()))?;

        if advance_head {
            client
                .execute(
                    "INSERT INTO heads(trajectory_id, branch, head_id, head_seq)
                     VALUES ($1, 'main', $2, $3)
                     ON CONFLICT(trajectory_id, branch)
                     DO UPDATE SET head_id = EXCLUDED.head_id, head_seq = EXCLUDED.head_seq",
                    &[
                        &entry.trajectory_id.0.as_bytes().as_slice(),
                        &entry.id.as_bytes().as_slice(),
                        &(entry.seq as i64),
                    ],
                )
                .await
                .map_err(|e| Error::Ledger(e.to_string()))?;
        }
        Ok(())
    }
}

fn row_to_entry(row: &tokio_postgres::Row) -> Result<Entry> {
    let id_bytes: &[u8] = row.get(0);
    let traj_bytes: &[u8] = row.get(1);
    let parent_bytes: Option<&[u8]> = row.get(2);
    let seq: i64 = row.get(3);
    let kind_str: &str = row.get(4);
    let payload_bytes: &[u8] = row.get(5);

    let mut id_arr = [0u8; 32];
    id_arr.copy_from_slice(id_bytes);
    let mut traj_arr = [0u8; 32];
    traj_arr.copy_from_slice(traj_bytes);

    let parent = parent_bytes.map(|b| {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(b);
        ContentHash(arr)
    });

    let kind = crate::str_to_kind(kind_str)?;
    let payload: EntryPayload = serde_json::from_slice(payload_bytes)
        .map_err(|e| Error::Ledger(format!("deserialize payload: {e}")))?;

    Ok(Entry {
        id: ContentHash(id_arr),
        trajectory_id: TrajectoryId(ContentHash(traj_arr)),
        parent,
        seq: seq as u64,
        kind,
        payload,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Blocking facade: a synchronous `LedgerStore` over the async `PostgresLedger`.
//
// The runtime and the `LedgerStore` trait are synchronous, but `PostgresLedger`
// is async. We cannot bridge with `block_on` on the request executor: the HTTP
// server calls sync ledger methods *from tokio worker threads* (async handlers
// and `run_agent_streaming`), where `Runtime::block_on` / `Handle::block_on`
// panics ("cannot block the current thread from within a runtime").
//
// So the facade owns a dedicated OS thread running its *own* multi-thread tokio
// runtime plus the connection pool. Each sync call ships a future to that thread
// over a tokio unbounded channel (whose `send` is itself synchronous) and blocks
// on a plain `std::sync::mpsc` reply. The only blocking is a std channel `recv`
// on the *caller's* thread — never a tokio runtime — so it is safe from any
// context (sync or async) and cannot deadlock against the request executor.
// This realizes RFC `runtime-ledger-trait-v1` Option A.

// A job is a `Send` closure that, when run *on the worker thread*, produces the
// actual per-call future. The future itself need not be `Send` (the Postgres
// query builder holds `!Send` params across awaits); only the closure crosses
// the thread boundary, and it captures only `Send` data.
type LedgerJob = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()>>> + Send>;

/// Synchronous [`crate::LedgerStore`] adapter over [`PostgresLedger`]. Build it
/// with [`BlockingPostgresLedger::connect`]; drop it to stop the worker thread.
pub struct BlockingPostgresLedger {
    job_tx: tokio::sync::mpsc::UnboundedSender<LedgerJob>,
    ledger: Arc<PostgresLedger>,
    // Owned so the worker thread's lifetime is tied to this value. The thread
    // exits once `job_tx` is dropped (channel closes) when the facade drops.
    _worker: std::thread::JoinHandle<()>,
}

impl BlockingPostgresLedger {
    /// Connect to Postgres on a dedicated runtime thread and bootstrap the
    /// schema. Blocks until the connection is established (or fails).
    pub fn connect(conn_str: &str) -> Result<Self> {
        let conn_str = conn_str.to_string();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<Arc<PostgresLedger>>>();
        let (job_tx, mut job_rx) = tokio::sync::mpsc::unbounded_channel::<LedgerJob>();

        let worker = std::thread::Builder::new()
            .name("thymos-pg-ledger".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ =
                            ready_tx.send(Err(Error::Ledger(format!("pg facade runtime: {e}"))));
                        return;
                    }
                };
                // A `LocalSet` lets us `spawn_local` the per-call futures, which
                // need not be `Send`. Concurrency is cooperative on this single
                // thread — fine for I/O-bound ledger ops, where overlapping
                // awaits on the connection pool still progress.
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async move {
                    let ledger = match PostgresLedger::connect(&conn_str).await {
                        Ok(l) => Arc::new(l),
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    if ready_tx.send(Ok(ledger)).is_err() {
                        return; // caller stopped waiting
                    }
                    // Drive jobs until every sender drops (the facade is gone).
                    while let Some(job) = job_rx.recv().await {
                        tokio::task::spawn_local(job());
                    }
                });
            })
            .map_err(|e| Error::Ledger(format!("pg facade thread: {e}")))?;

        let ledger = ready_rx
            .recv()
            .map_err(|_| Error::Ledger("pg facade thread exited before ready".into()))??;

        Ok(Self {
            job_tx,
            ledger,
            _worker: worker,
        })
    }

    /// Ship an async operation to the worker thread and block until it returns.
    fn call<T, F, Fut>(&self, f: F) -> T
    where
        F: FnOnce(Arc<PostgresLedger>) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<T>();
        let ledger = self.ledger.clone();
        let job: LedgerJob = Box::new(move || {
            Box::pin(async move {
                let out = f(ledger).await;
                let _ = reply_tx.send(out);
            }) as Pin<Box<dyn Future<Output = ()>>>
        });
        self.job_tx
            .send(job)
            .expect("thymos postgres ledger worker thread is not running");
        reply_rx
            .recv()
            .expect("thymos postgres ledger worker dropped the reply")
    }
}

impl crate::LedgerStore for BlockingPostgresLedger {
    fn backend(&self) -> &'static str {
        "postgres"
    }
    fn append_root(&self, trajectory_id: TrajectoryId, note: &str) -> Result<Entry> {
        let note = note.to_string();
        self.call(move |l| async move { l.append_root(trajectory_id, &note).await })
    }
    fn append_commit(&self, commit: Commit) -> Result<Entry> {
        self.call(move |l| async move { l.append_commit(commit).await })
    }
    fn append_rejection(
        &self,
        trajectory_id: TrajectoryId,
        intent_id: IntentId,
        reason: RejectionReason,
    ) -> Result<Entry> {
        self.call(move |l| async move { l.append_rejection(trajectory_id, intent_id, reason).await })
    }
    fn append_pending_approval(
        &self,
        trajectory_id: TrajectoryId,
        proposal: Proposal,
        channel: String,
        reason: String,
    ) -> Result<Entry> {
        self.call(move |l| async move {
            l.append_pending_approval(trajectory_id, proposal, channel, reason)
                .await
        })
    }
    fn append_delegation(
        &self,
        trajectory_id: TrajectoryId,
        child_trajectory_id: TrajectoryId,
        task: &str,
        final_answer: Option<String>,
    ) -> Result<Entry> {
        let task = task.to_string();
        self.call(move |l| async move {
            l.append_delegation(trajectory_id, child_trajectory_id, &task, final_answer)
                .await
        })
    }
    fn append_skill_bound(
        &self,
        trajectory_id: TrajectoryId,
        skill: thymos_core::skill::SkillDef,
        params: Vec<(String, String)>,
    ) -> Result<Entry> {
        self.call(move |l| async move { l.append_skill_bound(trajectory_id, skill, params).await })
    }
    fn append_branch_root(
        &self,
        new_trajectory_id: TrajectoryId,
        source_trajectory_id: TrajectoryId,
        source_commit_id: CommitId,
        note: &str,
    ) -> Result<Entry> {
        let note = note.to_string();
        self.call(move |l| async move {
            l.append_branch_root(new_trajectory_id, source_trajectory_id, source_commit_id, &note)
                .await
        })
    }
    fn head(&self, trajectory_id: TrajectoryId) -> Result<(ContentHash, u64)> {
        self.call(move |l| async move { l.head(trajectory_id).await })
    }
    fn entries(&self, trajectory_id: TrajectoryId) -> Result<Vec<Entry>> {
        self.call(move |l| async move { l.entries(trajectory_id).await })
    }
    fn query_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<AuditEntry>> {
        let kind = kind.map(|s| s.to_string());
        self.call(move |l| async move {
            l.query_entries(trajectory_id, kind.as_deref(), from_ts, to_ts, limit)
                .await
        })
    }
    fn count_entries(
        &self,
        trajectory_id: Option<TrajectoryId>,
        kind: Option<&str>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Result<u64> {
        let kind = kind.map(|s| s.to_string());
        self.call(move |l| async move {
            l.count_entries(trajectory_id, kind.as_deref(), from_ts, to_ts)
                .await
        })
    }
    fn has_trajectory(&self, trajectory_id: TrajectoryId) -> bool {
        self.call(move |l| async move { l.has_trajectory(trajectory_id).await })
    }
    fn verify_integrity(&self, trajectory_id: TrajectoryId) -> Result<()> {
        self.call(move |l| async move { l.verify_integrity(trajectory_id).await })
    }
    // `anchor` uses the trait default (entries-based) — identical across backends.
}
