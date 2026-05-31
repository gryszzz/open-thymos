//! Safe routing-feedback export.
//!
//! A routing advisor (e.g. WisePick) improves over time from execution
//! outcomes. THYMOS can supply that signal **without** compromising the two
//! things it guarantees:
//!
//! 1. **Determinism / replay** — this is a *pull* derived from the committed
//!    ledger after the fact. It is never read back into execution and never
//!    touches the replay path, so it cannot make the same intent route
//!    differently on replay. (Replay still rehydrates routing evidence from the
//!    immutable ledger snapshot, never from a live feedback pool.)
//! 2. **Data sovereignty** — a [`RoutingOutcome`] carries only the routing
//!    decision id, the route that was chosen, a coarse status, and latency.
//!    It deliberately excludes intent args, tool output, tenant identity, writ
//!    ids, resource values, and any free-text reason — nothing that could leak
//!    workload content or identity. And there is **no built-in network egress**:
//!    callers obtain the records and decide whether/where to send them. Off by
//!    default — it's a pull, not a push.
//!
//! The records are derived purely from the ledger (the audit source of truth),
//! so what is exported is exactly what was committed — auditable and stable.

use serde::{Deserialize, Serialize};

use thymos_ledger::{Entry, EntryPayload};

/// A single, non-sensitive routing outcome suitable for export to a routing
/// advisor's feedback channel. Keyed by `decision_hash` so the advisor can join
/// it back to the decision it made — without THYMOS revealing what was done.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingOutcome {
    /// The advisor's decision id (from `routing_evidence.decision_hash`). The
    /// join key; reveals nothing about the workload by itself.
    pub decision_hash: String,
    /// The route that was selected (`provider:capability`). The advisor supplied
    /// this, so returning it leaks nothing new.
    pub selected: String,
    /// Coarse outcome of the routed action. Currently `"committed"` — the route
    /// reached execution and was recorded. (Routes rejected at the governance
    /// boundary do not carry routing evidence on the ledger, so they are not
    /// exported.)
    pub status: String,
    /// Execution latency in milliseconds, from the recorded observation.
    pub latency_ms: u64,
}

/// Derive the safe routing-outcome records for a trajectory from its ledger
/// entries. Pure and read-only: only committed entries that carry routing
/// evidence produce an outcome, and only the non-sensitive fields above are
/// emitted.
pub fn routing_outcomes(entries: &[Entry]) -> Vec<RoutingOutcome> {
    entries
        .iter()
        .filter_map(|e| match &e.payload {
            EntryPayload::Commit(c) => {
                let ev = c.body.routing_evidence.as_ref()?;
                Some(RoutingOutcome {
                    decision_hash: ev.decision_hash.clone(),
                    selected: ev.selected.clone(),
                    status: "committed".to_string(),
                    latency_ms: c
                        .body
                        .observations
                        .first()
                        .map(|o| o.latency_ms)
                        .unwrap_or(0),
                })
            }
            _ => None,
        })
        .collect()
}
