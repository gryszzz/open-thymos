//! Persistent run store backed by SQLite.
//!
//! Stores run records so they survive server restarts. The in-memory
//! `HashMap<String, RunRecord>` in `AppState` is still used as a hot cache;
//! this module syncs to disk.

use rusqlite::{params, Connection};
use std::sync::Mutex;

use crate::{RunRecord, RunStatus, RunSummaryDto};

pub struct RunStore {
    conn: Mutex<Connection>,
}

impl RunStore {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        Self::bootstrap(&conn)?;
        Ok(RunStore {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::bootstrap(&conn)?;
        Ok(RunStore {
            conn: Mutex::new(conn),
        })
    }

    fn bootstrap(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous  = NORMAL;

            CREATE TABLE IF NOT EXISTS runs (
                run_id          TEXT PRIMARY KEY,
                trajectory_id   TEXT NOT NULL DEFAULT '',
                task            TEXT NOT NULL,
                status          TEXT NOT NULL DEFAULT 'running',
                tenant_id       TEXT NOT NULL DEFAULT '',
                summary_json    TEXT,
                created_at      INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at      INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
            CREATE INDEX IF NOT EXISTS idx_runs_created ON runs(created_at);
            CREATE INDEX IF NOT EXISTS idx_runs_tenant ON runs(tenant_id);
            "#,
        )
        .map_err(|e| e.to_string())
    }

    /// Insert a new run record.
    pub fn insert(&self, run_id: &str, task: &str, tenant_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO runs (run_id, task, status, tenant_id) VALUES (?1, ?2, 'running', ?3)",
            params![run_id, task, tenant_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Update a run record with completion status and summary.
    pub fn update(
        &self,
        run_id: &str,
        trajectory_id: &str,
        status: &str,
        summary: Option<&RunSummaryDto>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let summary_json = summary.map(|s| serde_json::to_string(s).unwrap_or_default());
        conn.execute(
            "UPDATE runs SET trajectory_id = ?1, status = ?2, summary_json = ?3, updated_at = unixepoch() WHERE run_id = ?4",
            params![trajectory_id, status, summary_json, run_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Load all runs (for restoring state on startup).
    pub fn load_all(&self) -> Result<Vec<(String, RunRecord)>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT run_id, trajectory_id, task, status, summary_json, tenant_id FROM runs ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                let run_id: String = row.get(0)?;
                let trajectory_id: String = row.get(1)?;
                let task: String = row.get(2)?;
                let status_str: String = row.get(3)?;
                let summary_json: Option<String> = row.get(4)?;
                let tenant_id: String = row.get::<_, Option<String>>(5)?.unwrap_or_default();

                let status = match status_str.as_str() {
                    "running" => RunStatus::Running,
                    "completed" => RunStatus::Completed,
                    _ => RunStatus::Failed,
                };

                let summary: Option<RunSummaryDto> =
                    summary_json.and_then(|s| serde_json::from_str(&s).ok());

                Ok((
                    run_id,
                    RunRecord {
                        trajectory_id,
                        task,
                        status,
                        summary,
                        tenant_id,
                    },
                ))
            })
            .map_err(|e| e.to_string())?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    }

    /// Get a single run by ID.
    pub fn get(&self, run_id: &str) -> Result<Option<RunRecord>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT trajectory_id, task, status, summary_json, tenant_id FROM runs WHERE run_id = ?1")
            .map_err(|e| e.to_string())?;

        let result = stmt
            .query_row(params![run_id], |row| {
                let trajectory_id: String = row.get(0)?;
                let task: String = row.get(1)?;
                let status_str: String = row.get(2)?;
                let summary_json: Option<String> = row.get(3)?;
                let tenant_id: String = row.get::<_, Option<String>>(4)?.unwrap_or_default();

                let status = match status_str.as_str() {
                    "running" => RunStatus::Running,
                    "completed" => RunStatus::Completed,
                    _ => RunStatus::Failed,
                };

                let summary = summary_json.and_then(|s| serde_json::from_str(&s).ok());

                Ok(RunRecord {
                    trajectory_id,
                    task,
                    status,
                    summary,
                    tenant_id,
                })
            })
            .ok(); // Returns None if not found.

        Ok(result)
    }

    /// List runs with pagination, optionally filtered by tenant.
    pub fn list(
        &self,
        limit: u32,
        offset: u32,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, RunRecord)>, String> {
        let conn = self.conn.lock().unwrap();
        let (sql, use_tenant) = if tenant_id.is_some() {
            (
                "SELECT run_id, trajectory_id, task, status, summary_json, tenant_id FROM runs WHERE tenant_id = ?3 ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
                true,
            )
        } else {
            (
                "SELECT run_id, trajectory_id, task, status, summary_json, tenant_id FROM runs ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
                false,
            )
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

        let rows = if use_tenant {
            stmt.query_map(
                params![limit, offset, tenant_id.unwrap()],
                row_to_run_record,
            )
            .map_err(|e| e.to_string())?
        } else {
            stmt.query_map(params![limit, offset], row_to_run_record)
                .map_err(|e| e.to_string())?
        };

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| e.to_string())?);
        }
        Ok(result)
    }

    /// Count total runs, optionally filtered by tenant.
    pub fn count(&self, tenant_id: Option<&str>) -> Result<u64, String> {
        let conn = self.conn.lock().unwrap();
        if let Some(tid) = tenant_id {
            conn.query_row(
                "SELECT COUNT(*) FROM runs WHERE tenant_id = ?1",
                params![tid],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        } else {
            conn.query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
                .map_err(|e| e.to_string())
        }
    }
}

fn row_to_run_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, RunRecord)> {
    let run_id: String = row.get(0)?;
    let trajectory_id: String = row.get(1)?;
    let task: String = row.get(2)?;
    let status_str: String = row.get(3)?;
    let summary_json: Option<String> = row.get(4)?;
    let tenant_id: String = row.get::<_, Option<String>>(5)?.unwrap_or_default();

    let status = match status_str.as_str() {
        "running" => RunStatus::Running,
        "completed" => RunStatus::Completed,
        _ => RunStatus::Failed,
    };
    let summary = summary_json.and_then(|s| serde_json::from_str(&s).ok());

    Ok((
        run_id,
        RunRecord {
            trajectory_id,
            task,
            status,
            summary,
            tenant_id,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_load() {
        let store = RunStore::open_in_memory().unwrap();
        store.insert("run-1", "do the thing", "").unwrap();

        let rec = store.get("run-1").unwrap().unwrap();
        assert_eq!(rec.task, "do the thing");
        assert_eq!(rec.status, RunStatus::Running);
    }

    #[test]
    fn update_and_load() {
        let store = RunStore::open_in_memory().unwrap();
        store.insert("run-1", "do the thing", "").unwrap();

        let summary = RunSummaryDto {
            steps_executed: 3,
            intents_submitted: 5,
            commits: 4,
            rejections: 1,
            failures: 0,
            final_answer: Some("done".into()),
            terminated_by: "CognitionDone".into(),
        };
        store
            .update("run-1", "traj-abc", "completed", Some(&summary))
            .unwrap();

        let rec = store.get("run-1").unwrap().unwrap();
        assert_eq!(rec.status, RunStatus::Completed);
        assert_eq!(rec.trajectory_id, "traj-abc");
        assert_eq!(rec.summary.unwrap().commits, 4);
    }

    #[test]
    fn list_and_count() {
        let store = RunStore::open_in_memory().unwrap();
        store.insert("run-1", "task 1", "tenant-a").unwrap();
        store.insert("run-2", "task 2", "tenant-a").unwrap();
        store.insert("run-3", "task 3", "tenant-b").unwrap();

        assert_eq!(store.count(None).unwrap(), 3);
        assert_eq!(store.count(Some("tenant-a")).unwrap(), 2);

        let page = store.list(2, 0, None).unwrap();
        assert_eq!(page.len(), 2);

        let page2 = store.list(2, 2, None).unwrap();
        assert_eq!(page2.len(), 1);

        let tenant_page = store.list(10, 0, Some("tenant-b")).unwrap();
        assert_eq!(tenant_page.len(), 1);
    }

    #[test]
    fn load_all_restores_runs() {
        let store = RunStore::open_in_memory().unwrap();
        store.insert("run-1", "task 1", "").unwrap();
        store.insert("run-2", "task 2", "").unwrap();

        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 2);
    }
}
