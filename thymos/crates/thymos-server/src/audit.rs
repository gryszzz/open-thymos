//! Audit log export endpoints.
//!
//! Routes:
//!   GET /audit/entries?run_id=...&kind=...&from=...&to=...&format=json|csv&limit=...
//!   GET /audit/entries/count?run_id=...&kind=...&from=...&to=...

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::AppState;
use thymos_core::{ContentHash, TrajectoryId};

#[derive(Deserialize)]
pub struct AuditQuery {
    /// Filter by run ID (looks up the trajectory_id from the runs map).
    pub run_id: Option<String>,
    /// Filter by entry kind: root, commit, rejection, pending_approval, delegation, branch.
    pub kind: Option<String>,
    /// Unix timestamp lower bound (inclusive).
    pub from: Option<u64>,
    /// Unix timestamp upper bound (inclusive).
    pub to: Option<u64>,
    /// Output format: "json" (default) or "csv".
    #[serde(default = "default_format")]
    pub format: String,
    /// Max entries to return (default 1000).
    pub limit: Option<u32>,
}

fn default_format() -> String {
    "json".into()
}

/// Resolve a run_id to a TrajectoryId by looking up the runs map.
fn resolve_trajectory(state: &AppState, run_id: &str) -> Option<TrajectoryId> {
    let runs = state.runs.lock().unwrap();
    // Run id (the normal case), else accept a trajectory id directly — the
    // Runs list and chat history carry both, and operators paste either.
    let hex_id = match runs.get(run_id) {
        Some(rec) if !rec.trajectory_id.is_empty() => rec.trajectory_id.clone(),
        _ => {
            let candidate = run_id.strip_prefix("traj:").unwrap_or(run_id);
            if runs.values().any(|r| r.trajectory_id == candidate) {
                candidate.to_string()
            } else {
                return None;
            }
        }
    };
    let bytes = hex::decode(&hex_id).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(TrajectoryId(ContentHash(arr)))
}

/// GET /audit/entries — query and export audit log entries.
pub async fn get_audit_entries(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    let trajectory_id = q
        .run_id
        .as_deref()
        .and_then(|id| resolve_trajectory(&state, id));

    // If run_id was given but couldn't be resolved, return 404.
    if q.run_id.is_some() && trajectory_id.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found or has no trajectory" })),
        )
            .into_response();
    }

    let entries =
        state
            .runtime
            .ledger
            .query_entries(trajectory_id, q.kind.as_deref(), q.from, q.to, q.limit);

    let entries = match entries {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    match q.format.as_str() {
        "csv" => {
            let mut csv = String::from("id,trajectory_id,seq,kind,created_at,payload\n");
            for entry in &entries {
                let payload_json = serde_json::to_string(&entry.payload).unwrap_or_default();
                // Escape double-quotes in CSV payload field.
                let escaped = payload_json.replace('"', "\"\"");
                csv.push_str(&format!(
                    "{},{},{},{},{},\"{}\"\n",
                    entry.id, entry.trajectory_id, entry.seq, entry.kind, entry.created_at, escaped
                ));
            }
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"audit-log.csv\"",
                    ),
                ],
                csv,
            )
                .into_response()
        }
        _ => (
            StatusCode::OK,
            Json(serde_json::json!({
                "entries": entries,
                "count": entries.len(),
            })),
        )
            .into_response(),
    }
}

/// GET /audit/entries/count — count matching entries without fetching payloads.
pub async fn count_audit_entries(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditQuery>,
) -> impl IntoResponse {
    let trajectory_id = q
        .run_id
        .as_deref()
        .and_then(|id| resolve_trajectory(&state, id));

    if q.run_id.is_some() && trajectory_id.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "run not found or has no trajectory" })),
        )
            .into_response();
    }

    match state
        .runtime
        .ledger
        .count_entries(trajectory_id, q.kind.as_deref(), q.from, q.to)
    {
        Ok(count) => (StatusCode::OK, Json(serde_json::json!({ "count": count }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
