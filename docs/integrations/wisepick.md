# Integrating WisePick with OpenThymos

WisePick is a pre-Proposal **routing advisor**: it scores candidate capabilities
and returns a route plus decision metadata *before* governed execution begins.
OpenThymos owns everything after that — authority, execution, side effects,
recovery, and the durable record of what happened.

```
Intent
  → WisePick scores candidate capabilities (off-runtime)
  → returns routing evidence (selected route + alternatives + estimates)
  → THYMOS compiles a Proposal and attaches the evidence
  → writ / effect-ceiling / budget / policy decide whether it runs
  → THYMOS executes through its tool fabric
  → result / rejection / suspension / delegation is ledgered
```

**The boundary:** routing decides what *looks* optimal; governance decides what
is *allowed*; execution proves what actually *happened*. WisePick never gains
execution authority — its evidence is recorded for audit/replay and is **never**
read by the runtime for authority, budget, or policy decisions.

## The routing-evidence contract (Proposal Contract v1, Option 2)

`routing_evidence` is a first-class **optional** field on `Proposal` — it lives
*outside* `ProposalBody`, so it does **not** affect `ProposalId`. When recorded
it is bound into the ledgered envelope (the `Commit` / `PendingApproval` entry
hashes), so it is immutable and replay-safe. See
[`docs/rfcs/proposal-contract-v1.md`](../rfcs/proposal-contract-v1.md).

Wire shape (JSON):

```json
{
  "decision_hash": "<hex digest over the integer-valued payload>",
  "selected": "provider:capability",
  "alternatives": ["provider:capabilityA", "provider:capabilityB"],
  "confidence_bps": 9500,
  "reason_codes": ["cost_optimal"],
  "latency_estimate_ms": 800,
  "cost_estimate_millicents": 4200,
  "fallback_hint": { "provider": "openai", "model": "gpt-4o", "reason": "primary overloaded" }
}
```

Rules:

- **No floating point.** `confidence_bps` is basis points (0–10000); cost is USD
  millicents (`cost_estimate_millicents`). Both are fixed-point integers, so the
  payload is deterministic and replay-stable.
- **`decision_hash`** is a hex digest derived deterministically over those
  integer values — no ephemeral provider/request identifiers — so it is the
  stable rehydration key across replays.
- **`alternatives`** is the ranked fallback list. Retry/fallback topology stays
  THYMOS-owned; alternatives let it fall back without re-querying WisePick
  mid-execution.
- `fallback_hint` and `fallback_hint.model` are optional; all other fields are
  required.

Minimum version: the **types** ship in `v0.4.0`. The **HTTP endpoint** below
ships in `v0.4.1`.

## Integration path A — HTTP (`POST /routed-submit`)

For adapters that talk to the THYMOS server. One request = one governed action.

Request:

```json
{
  "tool": "kv_set",
  "args": { "key": "k", "value": "v" },
  "rationale": "wisepick selected this route",
  "routing_evidence": { /* the object above */ }
}
```

Response (one of):

```json
{ "status": "committed", "trajectory_id": "<hex>", "commit_id": "<hex>", "routing_evidence_recorded": true }
{ "status": "rejected",  "trajectory_id": "<hex>", "reason": "writ does not authorize tool '...'": }
{ "status": "suspended", "trajectory_id": "<hex>", "channel": "ops", "reason": "..." }
{ "status": "delegated", "trajectory_id": "<hex>", "child_trajectory_id": "<hex>" }
```

The server mints a writ scoped to the requested tool, compiles the proposal,
attaches `routing_evidence`, and runs it through the full governance pipeline.
The evidence is recorded immutably on the commit.

## Integration path B — Rust runtime

For embedding directly against `thymos-runtime`:

```rust
let evidence = RoutingEvidence { /* ... */ };
let step = run.submit_with_routing_evidence(intent, &writ, evidence)?;
```

`Proposal::with_routing_evidence(evidence)` is also available if you construct
proposals yourself.

## Replay & determinism

THYMOS replay folds committed ledger deltas to reproduce the same world
projection. WisePick's routing state evolves over time from execution feedback,
which is **not** deterministic — so replay must never call back into WisePick.

This is why the artifact is recorded immutably at commit time. On replay, the
runtime reads `routing_evidence` straight from the ledgered entry (keyed by
`decision_hash`); it does not re-query the advisor. Your adapter's
cached-decision mode is the canonical replay path.

## Routing feedback (safe, pull-based)

A routing advisor improves from execution outcomes. THYMOS can supply that
signal without breaking determinism or data sovereignty:

- **`GET /runs/{id_or_trajectory}/routing-outcomes`** returns the outcomes for a
  trajectory (also accepts the `trajectory_id` returned by `/routed-submit`).
- Each record is minimal and non-sensitive:

  ```json
  { "decision_hash": "deadbeef", "selected": "anthropic:claude", "status": "committed", "latency_ms": 42 }
  ```

  Keyed by `decision_hash` so the advisor joins it back to its own decision —
  **never** intent args, tool output, tenant, writ, resource values, or any free
  text.

Properties that keep this safe and THYMOS-consistent:

- **Pull, not push.** THYMOS initiates no egress; the advisor fetches outcomes
  and decides what to do with them. The operator owns the data boundary.
- **Out of the replay path.** Outcomes are derived from the committed ledger
  after the fact; they are never read back into execution, so they cannot make
  the same intent route differently on replay.
- **Derived from the ledger** (the audit source of truth), so the export is
  exactly what was committed — auditable and stable.

A live, third-party *shared* feedback pool is a separate decision: any such loop
must stay strictly on the forward path (never replay), and the operator should
weigh telemetry egress explicitly. This endpoint is the building block that lets
you do that on your own terms.

## What stays on each side

| WisePick | OpenThymos |
|----------|------------|
| capability scoring | proposal lifecycle |
| route suggestion + ranked alternatives | authority / governance (writ, effect ceiling, budget, policy) |
| cost / latency / confidence estimates | execution + retries / fallback / compensation |
| feedback-shaped routing efficacy | durable execution truth (the ledger) |

See issue [#1](https://github.com/gryszzz/open-thymos/issues/1) for the design
discussion this integration formalizes.
