//! Middleware: API key authentication and rate limiting.
//!
//! - **API key auth**: reads `Authorization: Bearer <key>` and validates
//!   against a set of known keys (in-memory for Phase 1; Postgres in prod).
//! - **Rate limiting**: token-bucket per API key, configurable per key tier.

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json},
};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// API key record.
#[derive(Clone, Debug)]
pub struct ApiKey {
    pub key: String,
    pub tenant_id: String,
    pub name: String,
    /// Max requests per minute.
    pub rate_limit_rpm: u32,
}

/// In-memory API key store + rate limiter.
pub struct ApiGateway {
    keys: HashMap<String, ApiKey>,
    /// Tracks (key -> (count, window_start)).
    rate_state: Mutex<HashMap<String, (u32, Instant)>>,
    store: Option<Arc<ApiKeyStore>>,
}

impl ApiGateway {
    pub fn new() -> Self {
        ApiGateway {
            keys: HashMap::new(),
            rate_state: Mutex::new(HashMap::new()),
            store: None,
        }
    }

    pub fn with_store(mut self, store: Arc<ApiKeyStore>) -> Result<Self, String> {
        for key in store.load_all()? {
            self.keys.insert(key.key.clone(), key);
        }
        self.store = Some(store);
        Ok(self)
    }

    /// Register an API key.
    pub fn add_key(&mut self, key: ApiKey) -> Result<(), String> {
        if let Some(store) = &self.store {
            store.upsert(&key)?;
        }
        self.keys.insert(key.key.clone(), key);
        Ok(())
    }

    /// Validate a key and check rate limits. Returns the key record on success.
    ///
    /// When a persistent store is configured, the counter lives in SQLite so
    /// all server nodes sharing the same `gateway.sqlite` observe the same
    /// per-key budget. Otherwise falls back to the in-memory path.
    pub fn authenticate(&self, bearer: &str) -> Result<&ApiKey, GatewayError> {
        let key = self.keys.get(bearer).ok_or(GatewayError::InvalidKey)?;

        if let Some(store) = &self.store {
            let (count, _elapsed) = store
                .incr_and_count(bearer, 60)
                .map_err(|_| GatewayError::InvalidKey)?;
            if count > key.rate_limit_rpm {
                return Err(GatewayError::RateLimited);
            }
            return Ok(key);
        }

        // Single-node fallback.
        let mut state = self.rate_state.lock().unwrap();
        let entry = state
            .entry(bearer.to_string())
            .or_insert((0, Instant::now()));

        if entry.1.elapsed().as_secs() >= 60 {
            *entry = (0, Instant::now());
        }

        if entry.0 >= key.rate_limit_rpm {
            return Err(GatewayError::RateLimited);
        }

        entry.0 += 1;
        Ok(key)
    }
}

/// Per-key usage stats snapshot.
#[derive(Clone, Debug, serde::Serialize)]
pub struct KeyUsageStats {
    pub key_name: String,
    pub tenant_id: String,
    pub requests_this_window: u32,
    pub rate_limit_rpm: u32,
    pub window_started_secs_ago: u64,
}

impl ApiGateway {
    /// Return usage stats for all known keys (for a dashboard).
    pub fn usage_stats(&self) -> Vec<KeyUsageStats> {
        let memory_state = self.rate_state.lock().unwrap();
        self.keys
            .values()
            .map(|k| {
                let (count, elapsed) = if let Some(store) = &self.store {
                    store.rate_snapshot(&k.key).unwrap_or((0, 0))
                } else {
                    memory_state
                        .get(&k.key)
                        .map(|(c, t)| (*c, t.elapsed().as_secs()))
                        .unwrap_or((0, 0))
                };
                KeyUsageStats {
                    key_name: k.name.clone(),
                    tenant_id: k.tenant_id.clone(),
                    requests_this_window: count,
                    rate_limit_rpm: k.rate_limit_rpm,
                    window_started_secs_ago: elapsed,
                }
            })
            .collect()
    }
}

impl ApiGateway {
    /// Seconds until the current rate-limit window resets for a given key.
    pub fn retry_after(&self, bearer: &str) -> u64 {
        if let Some(store) = &self.store {
            if let Some((_c, elapsed)) = store.rate_snapshot(bearer) {
                return 60_u64.saturating_sub(elapsed);
            }
            return 0;
        }
        let state = self.rate_state.lock().unwrap();
        match state.get(bearer) {
            Some((_count, started)) => {
                let elapsed = started.elapsed().as_secs();
                60_u64.saturating_sub(elapsed)
            }
            None => 0,
        }
    }
}

impl Default for ApiGateway {
    fn default() -> Self {
        Self::new()
    }
}

/// Persistent SQLite store for API keys.
pub struct ApiKeyStore {
    conn: Mutex<Connection>,
}

impl ApiKeyStore {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        Self::bootstrap(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::bootstrap(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn bootstrap(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS api_keys (
                api_key          TEXT PRIMARY KEY,
                tenant_id        TEXT NOT NULL DEFAULT '',
                name             TEXT NOT NULL,
                rate_limit_rpm   INTEGER NOT NULL,
                created_at       INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at       INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE IF NOT EXISTS rate_state (
                api_key          TEXT PRIMARY KEY,
                count            INTEGER NOT NULL,
                window_start     INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())
    }

    /// Atomically increment the per-key counter in the current 60s window and
    /// return the new count. If the window has expired, resets it first. This
    /// is the shared-state path used for distributed rate limiting.
    pub fn incr_and_count(&self, api_key: &str, window_secs: u64) -> Result<(u32, u64), String> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        let now: i64 = tx
            .query_row("SELECT unixepoch()", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;

        let existing: Option<(i64, i64)> = tx
            .query_row(
                "SELECT count, window_start FROM rate_state WHERE api_key = ?1",
                params![api_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();

        let (new_count, window_start) = match existing {
            Some((count, start)) if (now - start) < window_secs as i64 => (count as u32 + 1, start),
            _ => (1, now),
        };

        tx.execute(
            "INSERT INTO rate_state (api_key, count, window_start) VALUES (?1, ?2, ?3)
             ON CONFLICT(api_key) DO UPDATE SET count = excluded.count, window_start = excluded.window_start",
            params![api_key, new_count as i64, window_start],
        )
        .map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;
        Ok((new_count, (now - window_start) as u64))
    }

    /// Snapshot of current counters (for /usage).
    pub fn rate_snapshot(&self, api_key: &str) -> Option<(u32, u64)> {
        let conn = self.conn.lock().ok()?;
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT count, (unixepoch() - window_start) FROM rate_state WHERE api_key = ?1",
                params![api_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        row.map(|(c, e)| (c as u32, e.max(0) as u64))
    }

    pub fn load_all(&self) -> Result<Vec<ApiKey>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT api_key, tenant_id, name, rate_limit_rpm
                 FROM api_keys
                 ORDER BY created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ApiKey {
                    key: row.get(0)?,
                    tenant_id: row.get(1)?,
                    name: row.get(2)?,
                    rate_limit_rpm: row.get::<_, i64>(3)? as u32,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut keys = Vec::new();
        for row in rows {
            keys.push(row.map_err(|e| e.to_string())?);
        }
        Ok(keys)
    }

    pub fn upsert(&self, key: &ApiKey) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO api_keys (api_key, tenant_id, name, rate_limit_rpm, updated_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())
             ON CONFLICT(api_key) DO UPDATE SET
                 tenant_id = excluded.tenant_id,
                 name = excluded.name,
                 rate_limit_rpm = excluded.rate_limit_rpm,
                 updated_at = unixepoch()",
            params![key.key, key.tenant_id, key.name, key.rate_limit_rpm],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum GatewayError {
    InvalidKey,
    RateLimited,
    MissingAuth,
}

/// Axum middleware layer for API key auth + rate limiting.
///
/// Usage in router:
/// ```ignore
/// let gateway = Arc::new(api_gateway);
/// app.layer(axum::middleware::from_fn_with_state(
///     gateway.clone(),
///     api_key_middleware,
/// ))
/// ```
pub async fn api_key_middleware(
    axum::extract::State(gateway): axum::extract::State<Arc<ApiGateway>>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    // Skip auth for health and readiness checks.
    if matches!(request.uri().path(), "/health" | "/ready") {
        return next.run(request).await.into_response();
    }

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let bearer = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "missing or invalid Authorization header" })),
            )
                .into_response()
        }
    };

    match gateway.authenticate(bearer) {
        Ok(key) => {
            // Inject tenant_id and key name into request extensions for downstream use.
            let mut request = request;
            request.extensions_mut().insert(GatewayContext {
                tenant_id: key.tenant_id.clone(),
                key_name: key.name.clone(),
            });
            next.run(request).await.into_response()
        }
        Err(GatewayError::InvalidKey) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid API key" })),
        )
            .into_response(),
        Err(GatewayError::RateLimited) => {
            // Compute seconds remaining in the current window.
            let retry_after = gateway.retry_after(bearer);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", retry_after.to_string())],
                Json(serde_json::json!({
                    "error": "rate limit exceeded",
                    "retry_after_secs": retry_after,
                })),
            )
                .into_response()
        }
        Err(GatewayError::MissingAuth) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "missing auth" })),
        )
            .into_response(),
    }
}

/// Context injected into request extensions after successful gateway auth.
#[derive(Clone, Debug)]
pub struct GatewayContext {
    pub tenant_id: String,
    pub key_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_store_roundtrip() {
        let store = ApiKeyStore::open_in_memory().unwrap();
        let key = ApiKey {
            key: "k1".into(),
            tenant_id: "tenant-a".into(),
            name: "dev".into(),
            rate_limit_rpm: 60,
        };
        store.upsert(&key).unwrap();
        let keys = store.load_all().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].tenant_id, "tenant-a");
    }

    #[test]
    fn shared_rate_state_across_gateways() {
        // Simulate two server nodes pointing at the same SQLite file by
        // sharing an `ApiKeyStore`. The counter must be the same for both.
        let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
        store
            .upsert(&ApiKey {
                key: "shared".into(),
                tenant_id: "tenant-a".into(),
                name: "shared".into(),
                rate_limit_rpm: 3,
            })
            .unwrap();

        let g1 = ApiGateway::new().with_store(store.clone()).unwrap();
        let g2 = ApiGateway::new().with_store(store.clone()).unwrap();

        g1.authenticate("shared").unwrap();
        g2.authenticate("shared").unwrap();
        g1.authenticate("shared").unwrap();
        // Fourth request crosses the limit regardless of which node serves it.
        let err = g2.authenticate("shared").unwrap_err();
        assert!(matches!(err, GatewayError::RateLimited));
    }

    #[test]
    fn gateway_loads_keys_from_store() {
        let store = Arc::new(ApiKeyStore::open_in_memory().unwrap());
        store
            .upsert(&ApiKey {
                key: "persisted".into(),
                tenant_id: "tenant-a".into(),
                name: "persisted-key".into(),
                rate_limit_rpm: 60,
            })
            .unwrap();

        let gateway = ApiGateway::new().with_store(store).unwrap();
        let key = gateway.authenticate("persisted").unwrap();
        assert_eq!(key.name, "persisted-key");
    }
}
